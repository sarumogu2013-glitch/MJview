"""分類段（設計書 4.10 ③ 小型CNNで 34種＋赤5＋裏向き を判定）。

- モデル: torchvision の ResNet18 / MobileNetV3-Small（BSD-3-Clause）。**重みはランダム初期化から自前で学習**する
  （ImageNet の学習済み重みは配布条件が明記されていないため、ライセンス方針により使わない）。
- 入力: 牌1枚の切り抜き 96×128（crops.crop_tile）。
- 学習時の拡張: 枠のずれ・180°回転・明るさ/色・ぼかし・ノイズに加え、「低解像度化」（一度小さくしてから戻す）を入れる。
  小さく写った牌（盤面カメラの河など）でも読めるようにするため。
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np
import torch
from torch import nn
from torch.utils.data import DataLoader, Dataset

from .crops import CROP_H, CROP_W, crop_tile
from .dataset import Box, label_path_for, list_images, read_labels
from .tiles import CLASS_NAMES, NUM_CLASSES

ARCHS = ("resnet18", "mobilenet_v3_small")
MEAN = np.array([0.5, 0.5, 0.5], np.float32)
STD = np.array([0.25, 0.25, 0.25], np.float32)


def build_model(arch: str = "resnet18", num_classes: int = NUM_CLASSES) -> nn.Module:
    import torchvision

    if arch == "resnet18":
        return torchvision.models.resnet18(weights=None, num_classes=num_classes)
    if arch == "mobilenet_v3_small":
        return torchvision.models.mobilenet_v3_small(weights=None, num_classes=num_classes)
    raise ValueError(f"arch は {ARCHS} のどれか")


def to_tensor(crops_bgr: list[np.ndarray]) -> torch.Tensor:
    arr = np.stack([c[..., ::-1] for c in crops_bgr]).astype(np.float32) / 255  # RGB
    arr = (arr - MEAN) / STD
    return torch.from_numpy(arr.transpose(0, 3, 1, 2).copy())


# ---------------------------------------------------------------- 学習データ

_SRC_PAD = 0.2  # 拡張用に、正解枠より広めに切り抜いておく
_SRC_W, _SRC_H = 120, 160


@dataclass
class _Item:
    img: np.ndarray  # 広めの切り抜き（_SRC_H×_SRC_W、横向きは縦にそろえ済み）
    cls: int
    short_side: float  # 元画像での牌の短辺（画素）


def _load_items(dataset: Path, split: str) -> list[_Item]:
    items: list[_Item] = []
    for ip in list_images(dataset / "images" / split):
        img = cv2.imread(str(ip), cv2.IMREAD_COLOR)
        if img is None:
            continue
        for b in read_labels(label_path_for(dataset, split, ip), img.shape[1], img.shape[0]):
            src = crop_tile(img, b, size=(_SRC_W, _SRC_H), pad=_SRC_PAD)
            items.append(_Item(src, b.cls, b.short_side))
    return items


class CropDataset(Dataset):
    def __init__(self, items: list[_Item], augment: bool, min_px: int = 8, seed: int = 0):
        self.items = items
        self.augment = augment
        self.min_px = min_px
        self.seed = seed
        self.epoch = 0

    def __len__(self) -> int:
        return len(self.items)

    def _final_crop(self, src: np.ndarray, jitter: np.ndarray | None) -> np.ndarray:
        # 広めの切り抜き（pad=_SRC_PAD）から、通常の pad の枠を取り直す
        inner = 1 / (1 + 2 * _SRC_PAD)
        bw, bh = _SRC_W * inner, _SRC_H * inner
        x1, y1 = (_SRC_W - bw) / 2, (_SRC_H - bh) / 2
        if jitter is not None:
            x1 += jitter[0] * bw
            y1 += jitter[1] * bh
            bw *= 1 + jitter[2]
            bh *= 1 + jitter[3]
        return crop_tile(src, Box(0, x1, y1, x1 + bw, y1 + bh))

    def __getitem__(self, i: int):
        it = self.items[i]
        if not self.augment:
            return to_tensor([self._final_crop(it.img, None)])[0], it.cls
        rng = np.random.default_rng((self.seed, self.epoch, i))
        c = self._final_crop(it.img, rng.uniform(-0.08, 0.08, 4))
        if rng.random() < 0.5:
            c = cv2.rotate(c, cv2.ROTATE_180)
        if rng.random() < 0.6:  # 低解像度化: 牌の短辺が min_px〜元の大きさになるよう縮小してから戻す
            native = min(it.short_side, float(CROP_W))
            target = rng.uniform(self.min_px, max(self.min_px + 1, native))
            s = target / CROP_W
            small = cv2.resize(c, (max(2, round(CROP_W * s)), max(2, round(CROP_H * s))), interpolation=cv2.INTER_AREA)
            c = cv2.resize(small, (CROP_W, CROP_H), interpolation=cv2.INTER_LINEAR)
        f = c.astype(np.float32)
        f = f * rng.uniform(0.7, 1.3) + rng.uniform(-30, 30)
        f *= rng.uniform(0.85, 1.15, 3).astype(np.float32)
        if rng.random() < 0.3:
            f = cv2.GaussianBlur(f, (0, 0), rng.uniform(0.3, 1.2))
        f += rng.normal(0, rng.uniform(0, 8), f.shape).astype(np.float32)
        return to_tensor([np.clip(f, 0, 255).astype(np.uint8)])[0], it.cls


# ---------------------------------------------------------------- 学習・推論


def pick_device(name: str = "auto") -> torch.device:
    if name != "auto":
        return torch.device(name)
    return torch.device("cuda" if torch.cuda.is_available() else "cpu")


def train(
    dataset: Path,
    out: Path,
    arch: str = "resnet18",
    epochs: int = 30,
    batch: int = 128,
    lr: float = 2e-3,
    min_px: int = 8,
    workers: int = 2,
    device: str = "auto",
    seed: int = 0,
    log=print,
) -> dict:
    torch.manual_seed(seed)
    dev = pick_device(device)
    tr_items = _load_items(dataset, "train")
    va_items = _load_items(dataset, "val")
    if not tr_items or not va_items:
        raise RuntimeError("train / val に牌のラベルがありません")
    log(f"分類の学習: train {len(tr_items)} 枚 / val {len(va_items)} 枚, arch={arch}, device={dev}")
    tr = CropDataset(tr_items, augment=True, min_px=min_px, seed=seed)
    va = CropDataset(va_items, augment=False)
    tl = DataLoader(tr, batch_size=batch, shuffle=True, num_workers=workers, drop_last=len(tr) > batch)
    vl = DataLoader(va, batch_size=batch, shuffle=False, num_workers=workers)
    model = build_model(arch).to(dev)
    opt = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=lr, total_steps=max(1, epochs * len(tl)))
    loss_fn = nn.CrossEntropyLoss(label_smoothing=0.05)
    out.mkdir(parents=True, exist_ok=True)
    best = {"val_acc": -1.0}
    history = []
    for ep in range(epochs):
        tr.epoch = ep
        model.train()
        t0, tot, n = time.time(), 0.0, 0
        for x, y in tl:
            x, y = x.to(dev), y.to(dev)
            opt.zero_grad()
            loss = loss_fn(model(x), y)
            loss.backward()
            opt.step()
            sched.step()
            tot += float(loss.detach()) * len(y)
            n += len(y)
        acc = evaluate_loader(model, vl, dev)
        history.append({"epoch": ep + 1, "loss": tot / max(n, 1), "val_acc": acc})
        log(f"  epoch {ep + 1:3d}/{epochs}  loss {tot / max(n, 1):.4f}  val_acc {acc:.4f}  ({time.time() - t0:.1f}s)")
        if acc > best["val_acc"]:
            best = {"val_acc": acc, "epoch": ep + 1}
            save(model, arch, out / "classifier.pt")
    (out / "history.json").write_text(json.dumps({"best": best, "history": history}, indent=2), encoding="utf-8")
    log(f"最良: epoch {best['epoch']}  val_acc {best['val_acc']:.4f} → {out / 'classifier.pt'}")
    return best


@torch.no_grad()
def evaluate_loader(model: nn.Module, loader: DataLoader, dev: torch.device) -> float:
    model.eval()
    ok = n = 0
    for x, y in loader:
        p = model(x.to(dev)).argmax(1).cpu()
        ok += int((p == y).sum())
        n += len(y)
    return ok / max(n, 1)


def save(model: nn.Module, arch: str, path: Path) -> None:
    torch.save(
        {"arch": arch, "state_dict": model.state_dict(), "class_names": list(CLASS_NAMES), "input": [CROP_W, CROP_H]},
        path,
    )


class Classifier:
    def __init__(self, path: Path, device: str = "auto"):
        ck = torch.load(path, map_location="cpu", weights_only=False)
        if ck["class_names"] != list(CLASS_NAMES):
            raise ValueError("クラスの並びが違うモデルです")
        self.arch = ck["arch"]
        self.dev = pick_device(device)
        self.model = build_model(self.arch).to(self.dev).eval()
        self.model.load_state_dict(ck["state_dict"])

    @torch.no_grad()
    def predict(self, crops_bgr: list[np.ndarray], batch: int = 256) -> np.ndarray:
        """切り抜き（96×128 BGR）のリスト → 確率 (N, 38)。"""
        if not crops_bgr:
            return np.zeros((0, NUM_CLASSES), np.float32)
        outs = []
        for i in range(0, len(crops_bgr), batch):
            x = to_tensor(crops_bgr[i : i + batch]).to(self.dev)
            outs.append(torch.softmax(self.model(x), 1).cpu().numpy())
        return np.concatenate(outs)

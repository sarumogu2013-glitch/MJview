"""段階A3: 解像度ごとの精度評価。

最大解像度で撮った画像（または動画）を縮小しながら認識させ、
  - 解像度・条件（撮影パターン）ごとの精度表
  - 牌1枚の大きさ（短辺の画素数）ごとの精度表 → 「牌1枚が何画素あれば読めるか」
を出す。

指標:
  - 検出の再現率・適合率（IoU ≥ 0.5 で正解の枠と対応づけ）
  - 分類の正解率（正解の枠で切り抜いた場合 = 分類段だけの実力。検出の影響を除く）
  - 総合の認識率（正解の牌のうち、検出され、かつ種類も正しかった割合 = 牌1枚単位の認識率）

縮小した画像は縮小したままの画素数で検出させる（検出器の入力サイズを画像の長辺に合わせる）。
"""

from __future__ import annotations

import csv
import json
from collections import defaultdict
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterator

import cv2
import numpy as np

from .classify import Classifier
from .crops import crop_tile
from .dataset import Box, group_of, iou, label_path_for, list_images, read_labels
from .detect import Detector
from .tiles import CLASS_NAMES

PX_BUCKETS = (0, 8, 10, 12, 14, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64, 96, 1e9)


@dataclass
class GtRecord:
    group: str
    image: str
    scale: float
    width: int
    height: int
    true: str
    short_px: float
    oracle_pred: str
    oracle_conf: float
    detected: bool
    e2e_pred: str  # 検出されなかったら ""
    e2e_conf: float

    @property
    def oracle_ok(self) -> bool:
        return self.oracle_pred == self.true

    @property
    def e2e_ok(self) -> bool:
        return self.detected and self.e2e_pred == self.true


@dataclass
class Sample:
    group: str
    name: str
    img: np.ndarray
    boxes: list[Box]


def iter_dataset(dataset: Path, split: str) -> Iterator[Sample]:
    root = dataset / "images" / split
    if not root.is_dir():
        raise FileNotFoundError(root)
    for ip in list_images(root):
        lp = label_path_for(dataset, split, ip)
        if not lp.exists():
            continue
        img = cv2.imread(str(ip), cv2.IMREAD_COLOR)
        if img is None:
            continue
        yield Sample(group_of(dataset, split, ip), ip.relative_to(root).as_posix(), img,
                     read_labels(lp, img.shape[1], img.shape[0]))


def iter_video(video: Path, labels: Path) -> Iterator[Sample]:
    """動画の各フレーム。labels/<動画名>_<フレーム番号6桁>.txt があるフレームだけ評価する（frames.py の命名と同じ）。"""
    cap = cv2.VideoCapture(str(video))
    if not cap.isOpened():
        raise FileNotFoundError(video)
    idx = 0
    try:
        while True:
            ok, img = cap.read()
            if not ok:
                break
            lp = labels / f"{video.stem}_{idx:06d}.txt"
            if lp.exists():
                yield Sample(video.stem, lp.stem, img, read_labels(lp, img.shape[1], img.shape[0]))
            idx += 1
    finally:
        cap.release()


def match(gt: list[Box], det: list[Box], thr: float = 0.5) -> dict[int, int]:
    """確信度の高い検出から順に、IoU が最大の未対応の正解に割り当てる。戻り値 {gt_index: det_index}。"""
    pairs: dict[int, int] = {}
    used: set[int] = set()
    for di in sorted(range(len(det)), key=lambda i: -det[i].conf):
        best, best_iou = -1, thr
        for gi, g in enumerate(gt):
            if gi in used:
                continue
            v = iou(g, det[di])
            if v >= best_iou:
                best, best_iou = gi, v
        if best >= 0:
            used.add(best)
            pairs[best] = di
    return pairs


def _resize(img: np.ndarray, s: float) -> np.ndarray:
    if abs(s - 1.0) < 1e-6:
        return img
    w, h = max(1, round(img.shape[1] * s)), max(1, round(img.shape[0] * s))
    return cv2.resize(img, (w, h), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_CUBIC)


def scales_for(img_h: int, scales: list[float] | None, heights: list[int] | None) -> list[float]:
    if heights:
        return [h / img_h for h in heights if h <= img_h * 1.0001]
    return scales or [1.0]


def run(
    samples: Iterator[Sample],
    classifier: Classifier,
    detector: Detector | None,
    scales: list[float] | None = None,
    heights: list[int] | None = None,
    det_conf: float = 0.25,
    log=print,
) -> tuple[list[GtRecord], dict[tuple[str, float], int]]:
    """評価を実行し、正解の牌1枚ごとの記録と、(条件, 倍率) ごとの誤検出数を返す。"""
    records: list[GtRecord] = []
    false_pos: dict[tuple[str, float], int] = defaultdict(int)
    n_img = 0
    for smp in samples:
        n_img += 1
        for s in scales_for(smp.img.shape[0], scales, heights):
            img = _resize(smp.img, s)
            gt = [b.scaled(s) for b in smp.boxes]
            oracle = classifier.predict([crop_tile(img, b) for b in gt])
            det: list[Box] = []
            pairs: dict[int, int] = {}
            det_probs = np.zeros((0, len(CLASS_NAMES)))
            if detector is not None:
                det = detector.predict(img, imgsz=None, conf=det_conf)
                pairs = match(gt, det)
                false_pos[(smp.group, round(s, 4))] += len(det) - len(pairs)
                det_probs = classifier.predict([crop_tile(img, det[di]) for di in pairs.values()])
            det_pred = {gi: det_probs[k] for k, gi in enumerate(pairs.keys())}
            for gi, b in enumerate(gt):
                op = oracle[gi]
                ep = det_pred.get(gi)
                records.append(
                    GtRecord(
                        group=smp.group, image=smp.name, scale=round(s, 4),
                        width=img.shape[1], height=img.shape[0],
                        true=CLASS_NAMES[b.cls], short_px=round(b.short_side, 2),
                        oracle_pred=CLASS_NAMES[int(op.argmax())], oracle_conf=round(float(op.max()), 4),
                        detected=ep is not None,
                        e2e_pred=CLASS_NAMES[int(ep.argmax())] if ep is not None else "",
                        e2e_conf=round(float(ep.max()) * det[pairs[gi]].conf, 4) if ep is not None else 0.0,
                    )
                )
        if n_img % 20 == 0:
            log(f"  {n_img} 枚処理")
    return records, dict(false_pos)


# ---------------------------------------------------------------- 集計


def _rate(ok: int, n: int) -> str:
    return f"{ok / n * 100:.1f}%" if n else "-"


def _bucket_label(lo: float, hi: float) -> str:
    return f"{int(lo)}〜{int(hi)}px" if hi < 1e8 else f"{int(lo)}px〜"


def summarize(
    records: list[GtRecord],
    false_pos: dict[tuple[str, float], int],
    has_detector: bool,
    target: float = 0.99,
    min_n: int = 30,
) -> tuple[str, dict]:
    """Markdown の表と、機械可読な要約を作る。"""
    lines: list[str] = ["# 牌認識 解像度評価（段階A3）", ""]
    summary: dict = {"target": target, "by_condition": [], "by_size": [], "required_px": {}}

    # 1) 条件 × 解像度
    lines += ["## 条件・解像度ごとの精度", ""]
    head = "| 条件 | 解像度 | 倍率 | 牌の短辺（中央値） | 牌の数 | 分類（正解枠） |"
    sep = "|---|---|---|---|---|---|"
    if has_detector:
        head += " 検出の再現率 | 検出の適合率 | 総合の認識率 |"
        sep += "---|---|---|"
    lines += [head, sep]
    groups: dict[tuple[str, float], list[GtRecord]] = defaultdict(list)
    for r in records:
        groups[(r.group, r.scale)].append(r)
    for (g, s), rs in sorted(groups.items(), key=lambda kv: (kv[0][0], -kv[0][1])):
        n = len(rs)
        res = {f"{r.width}x{r.height}" for r in rs}
        o = sum(r.oracle_ok for r in rs)
        row = {
            "group": g, "scale": s, "resolution": sorted(res)[0] if len(res) == 1 else "混在",
            "median_px": float(np.median([r.short_px for r in rs])), "n": n, "oracle_acc": o / n,
        }
        line = f"| {g} | {row['resolution']} | {s:g} | {row['median_px']:.1f}px | {n} | {_rate(o, n)} |"
        if has_detector:
            d = sum(r.detected for r in rs)
            e = sum(r.e2e_ok for r in rs)
            fp = false_pos.get((g, s), 0)
            row |= {"det_recall": d / n, "det_precision": d / (d + fp) if d + fp else 0.0, "e2e_acc": e / n,
                    "false_positives": fp}
            line += f" {_rate(d, n)} | {_rate(d, d + fp)} | {_rate(e, n)} |"
        lines.append(line)
        summary["by_condition"].append(row)

    # 2) 牌の大きさ別
    lines += ["", "## 牌の大きさ（短辺の画素数）ごとの精度", "",
              "全条件・全倍率をまとめて、写った牌の大きさで分けた表。", ""]
    head = "| 牌の短辺 | 牌の数 | 分類（正解枠） |" + (" 検出の再現率 | 総合の認識率 |" if has_detector else "")
    lines += [head, "|---|---|---|" + ("---|---|" if has_detector else "")]
    for lo, hi in zip(PX_BUCKETS[:-1], PX_BUCKETS[1:]):
        rs = [r for r in records if lo <= r.short_px < hi]
        if not rs:
            continue
        n = len(rs)
        o = sum(r.oracle_ok for r in rs)
        row = {"lo": lo, "hi": hi if hi < 1e8 else None, "n": n, "oracle_acc": o / n}
        line = f"| {_bucket_label(lo, hi)} | {n} | {_rate(o, n)} |"
        if has_detector:
            d = sum(r.detected for r in rs)
            e = sum(r.e2e_ok for r in rs)
            row |= {"det_recall": d / n, "e2e_acc": e / n}
            line += f" {_rate(d, n)} | {_rate(e, n)} |"
        lines.append(line)
        summary["by_size"].append(row)

    # 3) 必要画素数
    def required(key: str) -> float | None:
        """その大きさ以上の牌をまとめた精度が target 以上になる、最小の大きさ（下限）。"""
        best = None
        for lo, hi in reversed(list(zip(PX_BUCKETS[:-1], PX_BUCKETS[1:]))):
            if not any(lo <= r.short_px < hi for r in records):
                continue  # データのない区間の下限は答えにしない
            rs = [r for r in records if r.short_px >= lo]
            if len(rs) < min_n:
                continue
            acc = sum(getattr(r, key) for r in rs) / len(rs)
            if acc >= target:
                best = lo
            else:
                break
        return best

    lines += ["", f"## 必要な画素数（目標 {target * 100:g}% 以上）", ""]
    for key, label in [("oracle_ok", "分類（正解枠）"), ("e2e_ok", "総合の認識率")]:
        if key == "e2e_ok" and not has_detector:
            continue
        v = required(key)
        summary["required_px"][key] = v
        txt = f"牌の短辺 **{v:g}px 以上**" if v is not None else "どの大きさでも目標に届かない（またはデータ不足）"
        lines.append(f"- {label}: {txt}")
    lines.append(f"  （各大きさ以上の牌をまとめた精度で判定。{min_n}枚未満の区間は判定に使わない）")

    # 4) 間違えやすい組み合わせ
    conf: dict[tuple[str, str], int] = defaultdict(int)
    for r in records:
        if not r.oracle_ok:
            conf[(r.true, r.oracle_pred)] += 1
    if conf:
        lines += ["", "## 間違えやすい組み合わせ（分類・正解枠、上位10）", "", "| 正解 | 誤認識 | 件数 |", "|---|---|---|"]
        for (t, p), c in sorted(conf.items(), key=lambda kv: -kv[1])[:10]:
            lines.append(f"| {t} | {p} | {c} |")
        summary["confusions"] = [{"true": t, "pred": p, "count": c} for (t, p), c in
                                 sorted(conf.items(), key=lambda kv: -kv[1])[:30]]
    return "\n".join(lines) + "\n", summary


def write_outputs(out: Path, records: list[GtRecord], md: str, summary: dict) -> None:
    out.mkdir(parents=True, exist_ok=True)
    with (out / "records.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(GtRecord.__dataclass_fields__))
        w.writeheader()
        for r in records:
            w.writerow(asdict(r))
    (out / "summary.md").write_text(md, encoding="utf-8")
    (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")

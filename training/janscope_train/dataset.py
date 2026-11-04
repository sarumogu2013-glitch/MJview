"""データセットの形式（YOLO 形式）と読み書き。

    <dataset>/
      classes.txt              1行に1クラス名（正規の並び = tiles.CLASS_NAMES）
      images/<split>/...       画像（サブフォルダ可。サブフォルダ名は評価時の「条件」グループになる）
      labels/<split>/...       画像と同じ相対パスで拡張子 .txt
                               1行 = "class_id cx cy w h"（0〜1 に正規化、画像サイズ基準）

Label Studio / CVAT の「YOLO」書き出しをそのまま置き、`remap` で正規のクラス番号に揃える。
"""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path

import yaml

from .tiles import CLASS_NAMES, canonical_id

IMAGE_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp"}


@dataclass(frozen=True)
class Box:
    """画素単位の軸並行の枠（x1,y1 左上 / x2,y2 右下）。"""

    cls: int
    x1: float
    y1: float
    x2: float
    y2: float
    conf: float = 1.0

    @property
    def w(self) -> float:
        return self.x2 - self.x1

    @property
    def h(self) -> float:
        return self.y2 - self.y1

    @property
    def short_side(self) -> float:
        return min(self.w, self.h)

    def scaled(self, s: float) -> "Box":
        return Box(self.cls, self.x1 * s, self.y1 * s, self.x2 * s, self.y2 * s, self.conf)


def iou(a: Box, b: Box) -> float:
    ix = max(0.0, min(a.x2, b.x2) - max(a.x1, b.x1))
    iy = max(0.0, min(a.y2, b.y2) - max(a.y1, b.y1))
    inter = ix * iy
    union = a.w * a.h + b.w * b.h - inter
    return inter / union if union > 0 else 0.0


def read_labels(path: Path, img_w: int, img_h: int) -> list[Box]:
    boxes: list[Box] = []
    if not path.exists():
        return boxes
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        parts = line.split()
        if not parts:
            continue
        if len(parts) != 5:
            raise ValueError(f"{path}:{line_no}: 'class cx cy w h' の5項目が必要です（ポリゴン形式は未対応）")
        c = int(parts[0])
        cx, cy, w, h = (float(v) for v in parts[1:])
        boxes.append(
            Box(c, (cx - w / 2) * img_w, (cy - h / 2) * img_h, (cx + w / 2) * img_w, (cy + h / 2) * img_h)
        )
    return boxes


def write_labels(path: Path, boxes: list[Box], img_w: int, img_h: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = []
    for b in boxes:
        x1, y1 = max(0.0, b.x1), max(0.0, b.y1)
        x2, y2 = min(float(img_w), b.x2), min(float(img_h), b.y2)
        if x2 <= x1 or y2 <= y1:
            continue
        cx, cy = (x1 + x2) / 2 / img_w, (y1 + y2) / 2 / img_h
        lines.append(f"{b.cls} {cx:.6f} {cy:.6f} {(x2 - x1) / img_w:.6f} {(y2 - y1) / img_h:.6f}")
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def list_images(root: Path) -> list[Path]:
    return sorted(p for p in root.rglob("*") if p.suffix.lower() in IMAGE_EXTS)


def label_path_for(dataset: Path, split: str, image: Path) -> Path:
    rel = image.relative_to(dataset / "images" / split)
    return (dataset / "labels" / split / rel).with_suffix(".txt")


def group_of(dataset: Path, split: str, image: Path) -> str:
    """評価の条件グループ名（images/<split>/ 直下からの親フォルダ。直下なら "-"）。"""
    rel = image.relative_to(dataset / "images" / split).parent
    return rel.as_posix() if rel.as_posix() != "." else "-"


def write_classes(dataset: Path) -> None:
    (dataset / "classes.txt").write_text("\n".join(CLASS_NAMES) + "\n", encoding="utf-8")


def write_data_yaml(dataset: Path, out: Path | None = None) -> Path:
    """Ultralytics 用の data.yaml を作る（クラス名は38種のまま。検出学習では single_cls で1クラスに畳む）。"""
    dataset = dataset.resolve()
    splits = {s: f"images/{s}" for s in ("train", "val", "test") if (dataset / "images" / s).is_dir()}
    if "train" not in splits or "val" not in splits:
        raise FileNotFoundError(f"{dataset}/images/train と images/val が必要です")
    data = {"path": str(dataset), **splits, "names": {i: n for i, n in enumerate(CLASS_NAMES)}}
    out = out or dataset / "data.yaml"
    out.write_text(yaml.safe_dump(data, allow_unicode=True, sort_keys=False), encoding="utf-8")
    return out


def remap(src: Path, dst: Path, classes_file: Path | None = None) -> dict[str, int]:
    """注釈ツールの YOLO 書き出し（独自のクラス順）を正規のクラス番号に変換してコピーする。

    src には images/ labels/ と、クラス名の一覧（classes.txt / obj.names / data.yaml のいずれか）が必要。
    戻り値: 変換したラベル数（クラス名ごと）。
    """
    names = _read_tool_class_names(src, classes_file)
    mapping = {i: canonical_id(n) for i, n in enumerate(names)}
    counts: dict[str, int] = {}
    if (src / "images").resolve() != (dst / "images").resolve():
        shutil.copytree(src / "images", dst / "images", dirs_exist_ok=True)
    for lab in sorted((src / "labels").rglob("*.txt")):
        out_lines = []
        for line in lab.read_text(encoding="utf-8").splitlines():
            parts = line.split()
            if not parts:
                continue
            new = mapping[int(parts[0])]
            counts[CLASS_NAMES[new]] = counts.get(CLASS_NAMES[new], 0) + 1
            out_lines.append(" ".join([str(new), *parts[1:]]))
        out = dst / "labels" / lab.relative_to(src / "labels")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text("\n".join(out_lines) + ("\n" if out_lines else ""), encoding="utf-8")
    write_classes(dst)
    return counts


def _read_tool_class_names(src: Path, classes_file: Path | None) -> list[str]:
    if classes_file is not None:
        cands = [classes_file]
    else:
        cands = [src / "classes.txt", src / "obj.names", src / "data.yaml", src / "dataset.yaml"]
    for c in cands:
        if not c.exists():
            continue
        if c.suffix in (".yaml", ".yml"):
            names = yaml.safe_load(c.read_text(encoding="utf-8"))["names"]
            if isinstance(names, dict):
                return [names[k] for k in sorted(names, key=int)]
            return list(names)
        return [ln.strip() for ln in c.read_text(encoding="utf-8").splitlines() if ln.strip()]
    raise FileNotFoundError(f"{src} にクラス名の一覧（classes.txt / obj.names / data.yaml）がありません")

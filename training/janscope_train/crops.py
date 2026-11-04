"""牌1枚の切り抜きと正規化（設計書 4.10 ② 「牌1枚を正面向き・一定サイズに切り出し」）。

検出は軸並行の枠なので、ここでは
  - 枠を少し広げて切り抜く
  - 横長（横向きの牌: リーチ宣言牌・鳴いた牌）なら90°回して縦長にそろえる
  - 一定サイズ（既定 96×128）に縮小・拡大
を行う。上下逆（他家の向き）は分類の学習で180°回転を加えて吸収する。
卓の領域ごとの射影補正（校正の4隅指定）は、本体（段階C2）で切り抜き前に行う。
"""

from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np

from .dataset import Box, label_path_for, list_images, read_labels

CROP_W, CROP_H = 96, 128
PAD = 0.06


def crop_tile(img: np.ndarray, box: Box, size: tuple[int, int] = (CROP_W, CROP_H), pad: float = PAD) -> np.ndarray:
    h, w = img.shape[:2]
    px, py = box.w * pad, box.h * pad
    x1, y1 = int(max(0, np.floor(box.x1 - px))), int(max(0, np.floor(box.y1 - py)))
    x2, y2 = int(min(w, np.ceil(box.x2 + px))), int(min(h, np.ceil(box.y2 + py)))
    if x2 - x1 < 2 or y2 - y1 < 2:
        return np.zeros((size[1], size[0], 3), np.uint8)
    c = img[y1:y2, x1:x2]
    if (x2 - x1) > (y2 - y1) * 1.1:
        c = cv2.rotate(c, cv2.ROTATE_90_CLOCKWISE)
    interp = cv2.INTER_AREA if c.shape[0] > size[1] else cv2.INTER_CUBIC
    return cv2.resize(c, size, interpolation=interp)


def extract_crops(dataset: Path, split: str, out: Path, min_side: float = 0.0) -> int:
    """データセットの枠から分類用の切り抜きを作る（out/<class_name>/xxx.png）。"""
    from .tiles import CLASS_NAMES

    n = 0
    for ip in list_images(dataset / "images" / split):
        img = cv2.imread(str(ip), cv2.IMREAD_COLOR)
        if img is None:
            continue
        boxes = read_labels(label_path_for(dataset, split, ip), img.shape[1], img.shape[0])
        for k, b in enumerate(boxes):
            if b.short_side < min_side:
                continue
            d = out / CLASS_NAMES[b.cls]
            d.mkdir(parents=True, exist_ok=True)
            cv2.imwrite(str(d / f"{ip.stem}_{k:03d}.png"), crop_tile(img, b))
            n += 1
    return n

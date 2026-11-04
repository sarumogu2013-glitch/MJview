"""検出 → 切り抜き → 分類 をつないだ認識（本体の Rust 側 `ort` 実装の基準になる処理）。"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

from .classify import Classifier
from .crops import crop_tile
from .dataset import Box
from .detect import Detector
from .tiles import CLASS_NAMES


@dataclass
class TileResult:
    box: Box  # cls = 分類結果
    det_conf: float
    cls_conf: float

    @property
    def name(self) -> str:
        return CLASS_NAMES[self.box.cls]

    @property
    def confidence(self) -> float:
        return self.det_conf * self.cls_conf


class Recognizer:
    def __init__(self, detector: Path, classifier: Path, device: str = "auto"):
        self.detector = Detector(detector, device)
        self.classifier = Classifier(classifier, device)

    def __call__(self, img: np.ndarray, imgsz: int | None = None, conf: float = 0.25) -> list[TileResult]:
        dets = self.detector.predict(img, imgsz=imgsz, conf=conf)
        return classify_boxes(self.classifier, img, dets)


def classify_boxes(classifier: Classifier, img: np.ndarray, boxes: list[Box]) -> list[TileResult]:
    probs = classifier.predict([crop_tile(img, b) for b in boxes])
    out = []
    for b, p in zip(boxes, probs):
        k = int(p.argmax())
        out.append(TileResult(Box(k, b.x1, b.y1, b.x2, b.y2, b.conf), b.conf, float(p[k])))
    return out


def draw(img: np.ndarray, results: list[TileResult], low: float = 0.6) -> np.ndarray:
    """認識結果を画像に重ねる（確信度が low 未満は赤、それ以外は緑）。"""
    vis = img.copy()
    for r in results:
        b = r.box
        color = (0, 0, 255) if r.confidence < low else (0, 200, 0)
        cv2.rectangle(vis, (int(b.x1), int(b.y1)), (int(b.x2), int(b.y2)), color, 1)
        cv2.putText(
            vis, f"{r.name} {r.confidence:.2f}", (int(b.x1), max(10, int(b.y1) - 3)),
            cv2.FONT_HERSHEY_SIMPLEX, 0.4, color, 1, cv2.LINE_AA,
        )
    return vis

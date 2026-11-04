"""検出段（設計書 4.10 ① Ultralytics YOLO で牌の位置だけを1クラス「牌」として検出）。

Ultralytics（AGPL-3.0）を使う。データセットのラベルは38種のままにしておき、学習時に single_cls で1クラスに畳む。
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from .dataset import Box, write_data_yaml


def train(
    dataset: Path,
    out: Path,
    model: str = "yolo11n.pt",
    epochs: int = 100,
    imgsz: int = 1280,
    batch: int = 8,
    device: str = "auto",
    workers: int = 2,
    extra: dict | None = None,
) -> Path:
    """YOLO を学習し、best.pt のパスを返す。

    model: "yolo11n.pt" など（Ultralytics 配布の重み、AGPL-3.0）または "yolo11n.yaml"（ランダム初期化）。
    imgsz: 学習時の入力の長辺。小さな牌を拾うため既定 1280（2Kの画像を縮めすぎない）。
    """
    from ultralytics import YOLO

    data_yaml = write_data_yaml(dataset)
    yolo = YOLO(model)
    args = dict(
        data=str(data_yaml),
        epochs=epochs,
        imgsz=imgsz,
        batch=batch,
        single_cls=True,
        project=str(out.resolve().parent),
        name=out.name,
        exist_ok=True,
        workers=workers,
        # 牌は上下逆・横向きもあるが左右反転すると絵柄が変わる。検出は位置だけなので反転は可
        fliplr=0.5,
        flipud=0.5,
        degrees=10.0,
        mosaic=1.0,
        plots=False,
    )
    if device != "auto":
        args["device"] = device
    args.update(extra or {})
    yolo.train(**args)
    best = out / "weights" / "best.pt"
    if not best.exists():
        raise RuntimeError(f"学習結果が見つかりません: {best}")
    return best


class Detector:
    def __init__(self, weights: Path, device: str = "auto"):
        from ultralytics import YOLO

        self.model = YOLO(str(weights))
        self.device = None if device == "auto" else device

    def predict(self, img_bgr: np.ndarray, imgsz: int | None = None, conf: float = 0.25, iou: float = 0.6) -> list[Box]:
        """画像（BGR）→ 牌の枠（cls は 0 固定、conf は検出の確信度）。

        imgsz=None なら画像の長辺（32の倍数に切り上げ）をそのまま使う。
        解像度評価では、縮小した画像を縮小したままの画素数で推論させるため None にする。
        """
        if imgsz is None:
            imgsz = int(np.ceil(max(img_bgr.shape[:2]) / 32) * 32)
        kw = dict(imgsz=imgsz, conf=conf, iou=iou, verbose=False, max_det=300)
        if self.device:
            kw["device"] = self.device
        res = self.model.predict(img_bgr, **kw)[0]
        out = []
        if res.boxes is None:
            return out
        xyxy = res.boxes.xyxy.cpu().numpy()
        cf = res.boxes.conf.cpu().numpy()
        for (x1, y1, x2, y2), c in zip(xyxy, cf):
            out.append(Box(0, float(x1), float(y1), float(x2), float(y2), float(c)))
        return out

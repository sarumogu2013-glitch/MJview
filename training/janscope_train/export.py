"""学習したモデルを ONNX に書き出す（本体は Rust の `ort` で読む。設計書 4.10「モデル出力」）。"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import torch

from .classify import Classifier
from .crops import CROP_H, CROP_W
from .classify import MEAN, STD
from .tiles import CLASS_NAMES


def export_classifier(weights: Path, out: Path, opset: int = 17) -> Path:
    clf = Classifier(weights, device="cpu")
    out.parent.mkdir(parents=True, exist_ok=True)
    dummy = torch.zeros(1, 3, CROP_H, CROP_W)
    torch.onnx.export(
        clf.model, dummy, str(out), input_names=["input"], output_names=["logits"],
        dynamic_axes={"input": {0: "batch"}, "logits": {0: "batch"}}, opset_version=opset, dynamo=False,
    )
    meta = {
        "kind": "janscope-tile-classifier",
        "arch": clf.arch,
        "input": {"width": CROP_W, "height": CROP_H, "layout": "NCHW", "color": "RGB",
                  "scale": 1 / 255, "mean": MEAN.tolist(), "std": STD.tolist()},
        "classes": list(CLASS_NAMES),
        "license": "AGPL-3.0",
    }
    out.with_suffix(".json").write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")
    return out


def export_detector(weights: Path, out: Path, imgsz: int = 1280, opset: int = 17) -> Path:
    from ultralytics import YOLO

    path = Path(YOLO(str(weights)).export(format="onnx", imgsz=imgsz, opset=opset, dynamic=True, simplify=False))
    out.parent.mkdir(parents=True, exist_ok=True)
    if path.resolve() != out.resolve():
        shutil.copyfile(path, out)
    meta = {"kind": "janscope-tile-detector", "framework": "ultralytics-yolo", "imgsz": imgsz,
            "classes": ["tile"], "license": "AGPL-3.0"}
    out.with_suffix(".json").write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")
    return out

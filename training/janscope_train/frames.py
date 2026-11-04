"""段階A1: 撮影した動画から、注釈付け用のフレームを切り出す。

ファイル名は <動画名>_<フレーム番号6桁>.jpg。評価（evaluate --video）はこの番号でラベルと対応づける。
"""

from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np


def extract(video: Path, out: Path, every_sec: float = 1.0, skip_similar: float = 0.0) -> int:
    """every_sec 秒ごとにフレームを保存する。

    skip_similar > 0 のとき、直前に保存したフレームとの差（画素の平均絶対差、0〜255）が
    これ未満なら保存しない（牌が動いていない場面の重複を減らす）。
    """
    cap = cv2.VideoCapture(str(video))
    if not cap.isOpened():
        raise FileNotFoundError(video)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    step = max(1, round(fps * every_sec))
    out.mkdir(parents=True, exist_ok=True)
    idx = saved = 0
    last: np.ndarray | None = None
    try:
        while True:
            ok, img = cap.read()
            if not ok:
                break
            if idx % step == 0:
                small = cv2.resize(img, (160, 90), interpolation=cv2.INTER_AREA).astype(np.int16)
                if skip_similar <= 0 or last is None or np.abs(small - last).mean() >= skip_similar:
                    cv2.imwrite(str(out / f"{video.stem}_{idx:06d}.jpg"), img, [cv2.IMWRITE_JPEG_QUALITY, 95])
                    last = small
                    saved += 1
            idx += 1
    finally:
        cap.release()
    return saved

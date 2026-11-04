"""牌画像（assets/tiles の SVG）をラスタライズして、牌ごとの RGBA 画像を作る。"""

from __future__ import annotations

import io
from functools import lru_cache
from pathlib import Path

import numpy as np

from .tiles import CLASS_NAMES, fluffy_name

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_TILE_DIR = REPO_ROOT / "assets" / "tiles"
THEMES = ("regular", "black")

TILE_W, TILE_H = 192, 256  # 3:4。分類の入力（96×128）の2倍


def _svg_to_rgba(path: Path, w: int, h: int) -> np.ndarray:
    import cairosvg
    from PIL import Image

    png = cairosvg.svg2png(url=str(path), output_width=w, output_height=h)
    img = Image.open(io.BytesIO(png)).convert("RGBA")
    return np.asarray(img, dtype=np.uint8).copy()  # RGBA


def _over(dst: np.ndarray, src: np.ndarray) -> np.ndarray:
    """アルファ合成（src を dst の上に）。どちらも RGBA uint8。"""
    sa = src[..., 3:4].astype(np.float32) / 255
    da = dst[..., 3:4].astype(np.float32) / 255
    oa = sa + da * (1 - sa)
    rgb = (src[..., :3] * sa + dst[..., :3] * da * (1 - sa)) / np.maximum(oa, 1e-6)
    return np.concatenate([rgb, oa * 255], axis=-1).clip(0, 255).astype(np.uint8)


@lru_cache(maxsize=4)
def tile_faces(theme: str = "regular", tile_dir: str | None = None) -> tuple[np.ndarray, ...]:
    """クラス id 順に、牌の表（台＋絵柄）の RGBA 画像（TILE_H×TILE_W×4、RGB順）を返す。"""
    if theme not in THEMES:
        raise ValueError(f"theme は {THEMES} のどれか")
    d = Path(tile_dir) if tile_dir else DEFAULT_TILE_DIR
    d = d / theme
    front = _svg_to_rgba(d / "Front.svg", TILE_W, TILE_H)
    out = []
    for name in CLASS_NAMES:
        if name == "back":
            out.append(_svg_to_rgba(d / "Back.svg", TILE_W, TILE_H))
        else:
            face = _svg_to_rgba(d / f"{fluffy_name(name)}.svg", TILE_W, TILE_H)
            out.append(_over(front.copy(), face))
    return tuple(out)


def back_color(theme: str = "regular") -> tuple[int, int, int]:
    """牌の背の色（側面の描画に使う）。Back.svg の中央付近の平均色。"""
    back = tile_faces(theme)[-1]
    c = back[TILE_H // 3 : 2 * TILE_H // 3, TILE_W // 3 : 2 * TILE_W // 3, :3].reshape(-1, 3).mean(0)
    return int(c[0]), int(c[1]), int(c[2])

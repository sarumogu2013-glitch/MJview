"""合成データの生成（設計書 4.10「7章の牌画像から合成したデータを補助に使う」）。

実写データ（段階A1）が揃う前に学習・評価の流れを動かすためと、実写データの補助に使う。
2種類の場面を作る:

- board: 盤面カメラ想定。真上から見た河（6枚折り返し、リーチ宣言牌は横向き）・副露・山（裏向き）・ドラ表示牌
- hand : 手牌カメラ想定。立てた手牌13枚＋ツモ牌を正面〜斜め上から見た画像（牌の背の側面も描く）

同じ牌は1組136枚（赤5入り）から引くので、4枚を超えることはない。
ラベルは牌全体（見えている側面を含む）を囲む軸並行の枠。
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

from .dataset import Box, write_classes, write_data_yaml, write_labels
from .render import TILE_H, TILE_W, back_color, tile_faces
from .tiles import BACK_ID, wall_136

ASPECT = TILE_H / TILE_W  # 4/3


@dataclass
class Placed:
    cls: int
    cx: float  # 卓座標（牌の幅 = 1）
    cy: float
    angle: float  # 度。0 = 下の席から見て正立
    lift: float = 0.0  # 手牌モード: 牌の上に見える側面の高さ（牌の高さに対する比）


# ---------------------------------------------------------------- 場面の配置


def _rot(x: float, y: float, deg: float) -> tuple[float, float]:
    r = math.radians(deg)
    return x * math.cos(r) - y * math.sin(r), x * math.sin(r) + y * math.cos(r)


def layout_board(rng: np.random.Generator) -> list[Placed]:
    deck = wall_136(red_fives=int(rng.choice([0, 3, 3, 3])))
    rng.shuffle(deck)
    h = ASPECT
    gap = 0.04
    r0 = 3.4 + rng.uniform(-0.3, 0.6)  # 中央から河の1段目までの距離
    placed: list[Placed] = []
    for seat in range(4):
        phi = -90.0 * seat  # 0=下 1=右 2=上 3=左
        n_river = int(rng.integers(0, 22))
        riichi_at = int(rng.integers(0, n_river)) if n_river and rng.random() < 0.3 else -1
        col_x = -3.0 + 0.0
        row = 0
        k_in_row = 0
        for k in range(n_river):
            if k_in_row == 6 and row < 3:
                row += 1
                k_in_row = 0
                col_x = -3.0
            sideways = k == riichi_at
            tw = h if sideways else 1.0
            cx = col_x + tw / 2
            cy = r0 + row * (h + gap) + (h / 2 if not sideways else h - 0.5)
            col_x += tw + gap
            k_in_row += 1
            px, py = _rot(cx, cy, phi)
            jitter = rng.normal(0, 2.0)
            placed.append(Placed(deck.pop(), px, py, phi + (90 if sideways else 0) + jitter))
        # 山（裏向き）の一部とドラ表示牌
        wall_y = r0 + 4 * (h + gap) + 0.8
        if rng.random() < 0.7:
            n_wall = int(rng.integers(2, 12))
            x = -8.5 + rng.uniform(0, 2)
            dora_k = int(rng.integers(0, n_wall)) if rng.random() < 0.35 else -1
            for k in range(n_wall):
                cls = deck.pop() if k == dora_k else BACK_ID
                px, py = _rot(x + 0.5, wall_y + h / 2, phi)
                placed.append(Placed(cls, px, py, phi + rng.normal(0, 1.0)))
                x += 1.0 + 0.01
        # 副露（右手前）
        x = 9.0 - rng.uniform(0, 1.5)
        for _ in range(int(rng.choice([0, 0, 1, 1, 2]))):
            side = int(rng.integers(0, 3))
            for k in reversed(range(3)):
                sideways = k == side
                tw = h if sideways else 1.0
                x -= tw
                cy = wall_y + 1.6 * h + (h / 2 if not sideways else h - 0.5)
                px, py = _rot(x + tw / 2, cy, phi)
                placed.append(Placed(deck.pop(), px, py, phi + (90 if sideways else 0) + rng.normal(0, 1.5)))
                x -= gap
            x -= 0.4
    return placed


def layout_hand(rng: np.random.Generator) -> list[Placed]:
    deck = wall_136(red_fives=int(rng.choice([0, 3, 3, 3])))
    rng.shuffle(deck)
    n = int(rng.choice([13, 13, 13, 10, 7, 4]))
    tiles = sorted(deck[:n], key=lambda c: (c if c < 34 else {34: 4.5, 35: 13.5, 36: 22.5}.get(c, 99)))
    if rng.random() < 0.15:  # 理牌していない手
        rng.shuffle(tiles)
    lift = rng.uniform(0.05, 0.3)
    placed = []
    x = -n / 2
    for c in tiles:
        placed.append(Placed(int(c), x + 0.5, 0.0, rng.normal(0, 1.0), lift))
        x += 1.0 + rng.uniform(0.0, 0.03)
    if rng.random() < 0.6:  # ツモ牌（少し離す）
        x += rng.uniform(0.3, 0.8)
        placed.append(Placed(int(deck[n]), x + 0.5, 0.0, rng.normal(0, 2.0), lift))
    return placed


# ---------------------------------------------------------------- 描画


def _background(rng: np.random.Generator, w: int, h: int) -> np.ndarray:
    base = np.array(
        rng.choice(
            [
                (40, 110, 60),  # 緑のマット
                (30, 80, 120),  # 青
                (60, 60, 65),  # グレー
                (25, 25, 30),  # 黒
                (120, 40, 40),  # 赤
            ]
        ),
        dtype=np.float32,
    ) + rng.normal(0, 12, 3)
    small = rng.normal(0, 1, (max(2, h // 64), max(2, w // 64), 1)).astype(np.float32)
    noise = cv2.resize(small, (w, h), interpolation=cv2.INTER_CUBIC)[..., None] * rng.uniform(3, 12)
    fine = np.empty((h, w), np.float32)
    cv2.setRNGSeed(int(rng.integers(0, 2**31)))
    cv2.randn(fine, 0.0, float(rng.uniform(2, 7)))
    img = base[None, None, :] + noise + fine[..., None]
    # 照明のムラ（明るい中心＋周辺減光）
    cx, cy = rng.uniform(0.2, 0.8), rng.uniform(0.2, 0.8)
    dx = ((np.arange(w, dtype=np.float32) / w) - cx) ** 2
    dy = ((np.arange(h, dtype=np.float32) / h) - cy) ** 2
    d = np.sqrt(dy[:, None] + dx[None, :])
    img *= (1.15 - rng.uniform(0.2, 0.6) * d)[..., None]
    # 卓上の小物（点棒・サイコロ風の四角、影）
    for _ in range(int(rng.integers(0, 5))):
        color = tuple(float(v) for v in rng.uniform(0, 255, 3))
        p = rng.uniform(0, 1, 2) * (w, h)
        sz = rng.uniform(0.01, 0.06) * w
        rect = cv2.boxPoints(((p[0], p[1]), (sz, sz * rng.uniform(0.1, 1)), rng.uniform(0, 180)))
        cv2.fillPoly(img, [rect.astype(np.int32)], color)
    return img.clip(0, 255)


def _tile_quad(p: Placed, scale: float, lift: float = 0.0) -> np.ndarray:
    """卓座標の牌の四隅（左上, 右上, 右下, 左下）。lift>0 なら上に側面を足した外形。"""
    hw, hh = 0.5, ASPECT / 2
    top = -hh - lift * ASPECT
    pts = [(-hw, top), (hw, top), (hw, hh), (-hw, hh)]
    out = []
    for x, y in pts:
        rx, ry = _rot(x, y, p.angle)
        out.append((p.cx + rx, p.cy + ry))
    return np.array(out, dtype=np.float64) * scale


def _apply_h(hmat: np.ndarray, pts: np.ndarray) -> np.ndarray:
    return cv2.perspectiveTransform(pts.reshape(-1, 1, 2).astype(np.float64), hmat).reshape(-1, 2)


def _paste(canvas: np.ndarray, rgba: np.ndarray, quad: np.ndarray, shade: float = 1.0) -> None:
    hgt, wid = canvas.shape[:2]
    x0, y0 = np.floor(quad.min(0)).astype(int)
    x1, y1 = np.ceil(quad.max(0)).astype(int)
    x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, wid), min(y1, hgt)
    if x1 <= x0 or y1 <= y0:
        return
    sh, sw = rgba.shape[:2]
    src = np.array([(0, 0), (sw, 0), (sw, sh), (0, sh)], dtype=np.float32)
    m = cv2.getPerspectiveTransform(src, (quad - (x0, y0)).astype(np.float32))
    warped = cv2.warpPerspective(
        rgba, m, (x1 - x0, y1 - y0), flags=cv2.INTER_AREA if _shrink(quad, sw) else cv2.INTER_LINEAR
    ).astype(np.float32)
    a = warped[..., 3:4] / 255
    roi = canvas[y0:y1, x0:x1]
    roi[:] = roi * (1 - a) + warped[..., :3] * shade * a


def _shrink(quad: np.ndarray, sw: int) -> bool:
    return float(np.linalg.norm(quad[1] - quad[0])) < sw * 0.7


def _shadow(canvas: np.ndarray, quad: np.ndarray, rng: np.random.Generator) -> None:
    off = rng.uniform(1, 4, 2)
    q = quad + off
    hgt, wid = canvas.shape[:2]
    m = 8  # ぼかしの余白
    x0, y0 = np.floor(q.min(0)).astype(int) - m
    x1, y1 = np.ceil(q.max(0)).astype(int) + m
    x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, wid), min(y1, hgt)
    if x1 <= x0 or y1 <= y0:
        return
    mask = np.zeros((y1 - y0, x1 - x0), np.float32)
    cv2.fillPoly(mask, [(q - (x0, y0)).astype(np.int32)], 1.0)
    mask = cv2.GaussianBlur(mask, (0, 0), 2.0)
    canvas[y0:y1, x0:x1] *= (1 - 0.35 * mask)[..., None]


def _camera(rng: np.random.Generator, extent: np.ndarray, w: int, h: int, fill: float, keystone: float) -> np.ndarray:
    """卓座標（extent の外接矩形）→ 画像への射影。fill = 画像の短辺に対する占有率。"""
    (ex0, ey0), (ex1, ey1) = extent
    ew, eh = ex1 - ex0, ey1 - ey0
    s = fill * min(w / ew, h / eh)
    cx, cy = w / 2 + rng.uniform(-0.08, 0.08) * w, h / 2 + rng.uniform(-0.08, 0.08) * h
    src = np.array([(ex0, ey0), (ex1, ey0), (ex1, ey1), (ex0, ey1)], np.float32)
    rot = rng.uniform(-6, 6)
    dst = []
    for x, y in src:
        rx, ry = _rot((x - (ex0 + ex1) / 2) * s, (y - (ey0 + ey1) / 2) * s, rot)
        dst.append((cx + rx, cy + ry))
    dst = np.array(dst, np.float32)
    # 台形（斜めから見た歪み）: 上辺を縮める
    k = keystone * ew * s / 2
    dst[0, 0] += k
    dst[1, 0] -= k
    dst += rng.normal(0, 0.01 * min(w, h), dst.shape).astype(np.float32)
    return cv2.getPerspectiveTransform(src, dst).astype(np.float64)


def _degrade(img: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    img = img.astype(np.float32)
    img = img * rng.uniform(0.75, 1.25) + rng.uniform(-25, 25)  # 明るさ・コントラスト
    img *= np.array([rng.uniform(0.88, 1.12), 1.0, rng.uniform(0.88, 1.12)], np.float32)  # 色温度
    img = np.clip(img, 0, 255)
    img = 255 * (img / 255) ** rng.uniform(0.8, 1.25)
    r = rng.random()
    if r < 0.35:
        img = cv2.GaussianBlur(img, (0, 0), rng.uniform(0.4, 1.4))
    elif r < 0.55:  # 動きブレ
        k = int(rng.integers(3, 8))
        ker = np.zeros((k, k), np.float32)
        ker[k // 2, :] = 1 / k
        m = cv2.getRotationMatrix2D((k / 2 - 0.5, k / 2 - 0.5), rng.uniform(0, 180), 1.0)
        ker = cv2.warpAffine(ker, m, (k, k))
        img = cv2.filter2D(img, -1, ker / max(ker.sum(), 1e-6))
    noise = np.empty((img.shape[0], img.shape[1] * img.shape[2]), np.float32)  # 2次元にして全チャンネルに入れる
    cv2.randn(noise, 0.0, float(rng.uniform(1, 6)))
    img = img + noise.reshape(img.shape)
    img = np.clip(img, 0, 255).astype(np.uint8)
    ok, enc = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, int(rng.integers(60, 96))])
    return cv2.imdecode(enc, cv2.IMREAD_COLOR)


def render_scene(
    kind: str, rng: np.random.Generator, width: int = 1920, height: int = 1080, theme: str | None = None
) -> tuple[np.ndarray, list[Box]]:
    """1枚の合成画像（BGR uint8）とラベル（画素単位の枠）を作る。"""
    theme = theme or str(rng.choice(["regular", "regular", "black"]))
    faces = tile_faces(theme)
    canvas = _background(rng, width, height)
    if kind == "board":
        placed = layout_board(rng)
        fill = rng.uniform(0.85, 1.6)  # 1より大きいと打牌エリアに画角を詰めた状態（外側は切れる）
        keystone = rng.uniform(0.0, 0.08)
    elif kind == "hand":
        placed = layout_hand(rng)
        fill = rng.uniform(0.7, 0.97)
        keystone = rng.uniform(0.0, 0.12)
    else:
        raise ValueError(kind)
    if not placed:
        placed = layout_hand(rng)
    allq = np.concatenate([_tile_quad(p, 1.0, p.lift) for p in placed])
    extent = np.array([allq.min(0), allq.max(0)])
    if kind == "board":
        extent = np.array([[-9.0, -9.0], [9.0, 9.0]]) * rng.uniform(0.8, 1.2)
    hmat = _camera(rng, extent, width, height, fill, keystone)

    side_rgb = np.array(back_color(theme), np.float32)
    side_tile = np.zeros((TILE_H, TILE_W, 4), np.uint8)
    side_tile[..., :3] = side_rgb[::-1]  # BGR
    side_tile[..., 3] = 255

    boxes: list[Box] = []
    for p in placed:
        face_q = _apply_h(hmat, _tile_quad(p, 1.0))
        full_q = face_q
        if kind == "board":
            _shadow(canvas, face_q, rng)
        if p.lift > 0:  # 手牌: 牌の背（上側の側面）
            full_q = _apply_h(hmat, _tile_quad(p, 1.0, p.lift))
            top_q = np.array([full_q[0], full_q[1], face_q[1], face_q[0]])
            _paste(canvas, side_tile, top_q, shade=rng.uniform(0.75, 1.0))
        rgba = faces[p.cls][..., [2, 1, 0, 3]]  # RGB → BGR
        _paste(canvas, rgba, face_q, shade=rng.uniform(0.85, 1.05))
        x0, y0 = full_q.min(0)
        x1, y1 = full_q.max(0)
        vis_w = min(x1, width) - max(x0, 0)
        vis_h = min(y1, height) - max(y0, 0)
        if vis_w <= 0 or vis_h <= 0:
            continue
        if vis_w * vis_h < 0.6 * (x1 - x0) * (y1 - y0):  # 6割以上見えている牌だけラベルにする
            continue
        boxes.append(Box(p.cls, max(x0, 0), max(y0, 0), min(x1, width), min(y1, height)))
    # 画面外に切れて半端に見えている牌は、ラベルなしだと検出の学習を乱すので塗りつぶさずに残す（実写でも起きる）
    return _degrade(canvas, rng), boxes


def generate(
    out: Path,
    count: int,
    val_ratio: float = 0.15,
    test_ratio: float = 0.15,
    width: int = 1920,
    height: int = 1080,
    kinds: tuple[str, ...] = ("board", "hand"),
    seed: int = 0,
) -> dict[str, int]:
    """合成データセットを out に書き出す（images/<split>/<kind>/、labels/ 同様）。"""
    rng = np.random.default_rng(seed)
    stats = {"images": 0, "tiles": 0}
    for i in range(count):
        u = (i + 0.5) / count
        split = "val" if u < val_ratio else "test" if u < val_ratio + test_ratio else "train"
        kind = kinds[i % len(kinds)]
        img, boxes = render_scene(kind, rng, width, height)
        name = f"synth_{seed}_{i:06d}"
        ip = out / "images" / split / kind / f"{name}.jpg"
        ip.parent.mkdir(parents=True, exist_ok=True)
        cv2.imwrite(str(ip), img, [cv2.IMWRITE_JPEG_QUALITY, 95])
        write_labels(out / "labels" / split / kind / f"{name}.txt", boxes, width, height)
        stats["images"] += 1
        stats["tiles"] += len(boxes)
    write_classes(out)
    write_data_yaml(out)
    return stats

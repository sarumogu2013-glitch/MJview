#!/usr/bin/env python3
"""JanScope のアイコンを描く（オリジナル。外部の素材は使わない）。

    python3 tools/make_icon.py

できるもの（assets/icon/）:
    janscope.ico        … exe に埋め込む（16〜256px の複数サイズ）
    janscope-256.png    … 画面に出す用
    janscope-1024.png   … 元の大きさ

図柄: 雀卓の緑の角丸四角に、白い牌。牌の面に赤い「照準（スコープ）」＝ JanScope。
小さいサイズ（16〜32px）は線を太くして、照準の外側の輪と中心の点だけにする。
"""

import pathlib

from PIL import Image, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "icon"

GREEN_TOP = (22, 128, 92)
GREEN_BOTTOM = (10, 82, 60)
IVORY = (250, 247, 236)
TILE_EDGE = (214, 208, 190)
TILE_BACK = (36, 92, 170)   # 牌の裏（厚み）の青
RED = (214, 38, 46)


def draw(size: int) -> Image.Image:
    """size×size で描く（4倍で描いて縮める）"""
    small = size <= 32
    S = size * 4
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # 背景: 角丸四角（上から下へ少し暗くなる緑）
    r = int(S * 0.22)
    bg = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(bg)
    for y in range(S):
        t = y / (S - 1)
        c = tuple(int(GREEN_TOP[i] * (1 - t) + GREEN_BOTTOM[i] * t) for i in range(3))
        gd.line([(0, y), (S, y)], fill=c + (255,))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=r, fill=255)
    im.paste(bg, (0, 0), mask)

    d = ImageDraw.Draw(im)
    # 牌: 縦長。下に裏の青い厚みを少し見せる
    tw, th = S * 0.56, S * 0.70
    x0 = (S - tw) / 2
    y0 = (S - th) / 2 - S * 0.03
    tr = S * 0.08
    thick = S * (0.07 if not small else 0.08)
    # 影
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        [x0 + S * 0.02, y0 + S * 0.04, x0 + tw + S * 0.02, y0 + th + thick + S * 0.04], radius=tr, fill=(0, 0, 0, 90))
    im.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(S * 0.02)))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([x0, y0 + thick * 0.6, x0 + tw, y0 + th + thick], radius=tr, fill=TILE_BACK)
    d.rounded_rectangle([x0, y0, x0 + tw, y0 + th], radius=tr, fill=IVORY, outline=TILE_EDGE, width=max(1, int(S * 0.01)))

    # 照準
    cx, cy = x0 + tw / 2, y0 + th / 2
    R = tw * 0.36
    w = S * (0.05 if not small else 0.075)
    d.ellipse([cx - R, cy - R, cx + R, cy + R], outline=RED, width=int(w))
    if not small:
        # 十字（輪の内と外に少しずつ）
        g = R * 0.32
        L = R * 1.32
        lw = int(S * 0.032)
        for (ax, ay, bx, by) in [
            (cx, cy - L, cx, cy - g), (cx, cy + g, cx, cy + L),
            (cx - L, cy, cx - g, cy), (cx + g, cy, cx + L, cy),
        ]:
            d.line([(ax, ay), (bx, by)], fill=RED, width=lw)
        dot = R * 0.16
    else:
        dot = R * 0.30
    d.ellipse([cx - dot, cy - dot, cx + dot, cy + dot], fill=RED)

    return im.resize((size, size), Image.LANCZOS)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    big = draw(1024)
    big.save(OUT / "janscope-1024.png")
    draw(256).save(OUT / "janscope-256.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [draw(s) for s in sizes]
    frames[-1].save(OUT / "janscope.ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    # 確認用: 並べた画像
    sheet = Image.new("RGBA", (sum(sizes) + 10 * len(sizes) + 10, 276), (245, 245, 245, 255))
    x = 10
    for s, f in zip(sizes, frames):
        sheet.alpha_composite(f, (x, 266 - s))
        x += s + 10
    sheet.save(OUT.parent.parent / "target" / "icon-sheet.png") if (OUT.parent.parent / "target").is_dir() else None
    print("ok:", ", ".join(p.name for p in sorted(OUT.iterdir())))


if __name__ == "__main__":
    main()

from collections import Counter
from pathlib import Path

import numpy as np
import pytest

from janscope_train.crops import CROP_H, CROP_W, crop_tile
from janscope_train.dataset import Box, iou, read_labels, remap, write_labels
from janscope_train.evaluate import GtRecord, match, summarize
from janscope_train.synth import render_scene
from janscope_train.tiles import BACK_ID, CLASS_NAMES, NUM_CLASSES, base_kind, canonical_id, fluffy_name, wall_136
from janscope_train.render import DEFAULT_TILE_DIR


def test_classes():
    assert NUM_CLASSES == 38
    assert CLASS_NAMES[0] == "1m" and CLASS_NAMES[33] == "7z" and CLASS_NAMES[BACK_ID] == "back"
    assert canonical_id("Man5-Dora") == CLASS_NAMES.index("0m")
    assert canonical_id("中") == CLASS_NAMES.index("7z")
    assert canonical_id(" 5pr ") == CLASS_NAMES.index("0p")
    assert base_kind(CLASS_NAMES.index("0s")) == CLASS_NAMES.index("5s")
    with pytest.raises(KeyError):
        canonical_id("joker")


def test_every_class_has_an_svg():
    for name in CLASS_NAMES:
        for theme in ("regular", "black"):
            assert (DEFAULT_TILE_DIR / theme / f"{fluffy_name(name)}.svg").exists(), name


def test_wall():
    w = wall_136()
    assert len(w) == 136
    kinds = Counter(base_kind(t) for t in w)
    assert set(kinds.values()) == {4} and len(kinds) == 34


def test_label_roundtrip(tmp_path: Path):
    boxes = [Box(3, 10, 20, 50, 80), Box(37, 100.5, 0, 140, 60)]
    p = tmp_path / "a.txt"
    write_labels(p, boxes, 200, 100)
    back = read_labels(p, 200, 100)
    assert [b.cls for b in back] == [3, 37]
    for a, b in zip(boxes, back):
        assert iou(a, b) > 0.999


def test_remap(tmp_path: Path):
    src = tmp_path / "export"
    (src / "images" / "train").mkdir(parents=True)
    (src / "labels" / "train").mkdir(parents=True)
    (src / "images" / "train" / "x.jpg").write_bytes(b"")
    (src / "classes.txt").write_text("中\nMan1\nback\n", encoding="utf-8")
    (src / "labels" / "train" / "x.txt").write_text("0 0.5 0.5 0.1 0.1\n2 0.2 0.2 0.1 0.1\n1 0.3 0.3 0.1 0.1\n")
    dst = tmp_path / "ds"
    counts = remap(src, dst)
    assert counts == {"7z": 1, "back": 1, "1m": 1}
    ids = [int(ln.split()[0]) for ln in (dst / "labels" / "train" / "x.txt").read_text().splitlines()]
    assert ids == [33, 37, 0]
    assert (dst / "classes.txt").read_text().split() == list(CLASS_NAMES)


def test_crop_sideways_is_rotated():
    img = np.zeros((100, 200, 3), np.uint8)
    img[40:60, 50:80] = 255
    c = crop_tile(img, Box(0, 50, 40, 80, 60))  # 横長
    assert c.shape == (CROP_H, CROP_W, 3)


@pytest.mark.parametrize("kind", ["board", "hand"])
def test_synth_scene(kind):
    rng = np.random.default_rng(1)
    img, boxes = render_scene(kind, rng, 640, 360)
    assert img.shape == (360, 640, 3)
    assert boxes, "牌が1枚もない"
    counts = Counter(base_kind(b.cls) for b in boxes if b.cls != BACK_ID)
    assert max(counts.values()) <= 4
    for b in boxes:
        assert 0 <= b.x1 < b.x2 <= 640 and 0 <= b.y1 < b.y2 <= 360


def test_match_greedy_by_conf():
    gt = [Box(0, 0, 0, 10, 10), Box(1, 20, 0, 30, 10)]
    det = [Box(0, 1, 0, 11, 10, conf=0.5), Box(0, 0, 0, 10, 10, conf=0.9), Box(0, 50, 50, 60, 60, conf=0.99)]
    assert match(gt, det) == {0: 1}


def _rec(px, ok, det=True):
    return GtRecord("g", "i", 1.0, 100, 100, "1m", px, "1m" if ok else "2m", 0.9, det, "1m" if ok else "2m", 0.9)


def test_summary_required_px():
    recs = [_rec(10, i % 2 == 0) for i in range(40)] + [_rec(20, True) for _ in range(40)] + [_rec(50, True) for _ in range(40)]
    md, s = summarize(recs, {("g", 1.0): 0}, True, target=0.99, min_n=30)
    assert s["required_px"]["oracle_ok"] == 20  # 20px の区間から上はすべて正解（10px は半分しか当たらない）
    assert "必要な画素数" in md

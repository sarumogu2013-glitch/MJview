"""コマンドライン: python -m janscope_train <コマンド> ...

  synth            合成データセットを作る
  frames           動画から注釈付け用のフレームを切り出す（段階A1）
  remap            注釈ツールの YOLO 書き出しを正規のクラス番号に揃える（段階A2）
  stats            データセットの枚数・牌の数・大きさを表示
  train-detector   検出（YOLO, 1クラス）を学習
  train-classifier 分類（小型CNN, 38クラス）を学習
  evaluate         解像度ごとの精度評価（段階A3）
  predict          画像に認識結果を重ねて保存（目視確認用）
  export           ONNX に書き出し
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def _floats(s: str) -> list[float]:
    return [float(v) for v in s.split(",") if v.strip()]


def _ints(s: str) -> list[int]:
    return [int(v.lower().rstrip("p")) for v in s.split(",") if v.strip()]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="janscope_train", description="JanScope 牌認識の学習・評価ツール")
    sub = p.add_subparsers(dest="cmd", required=True)

    a = sub.add_parser("synth", help="合成データセットを作る")
    a.add_argument("out", type=Path)
    a.add_argument("--count", type=int, default=600)
    a.add_argument("--width", type=int, default=1920)
    a.add_argument("--height", type=int, default=1080)
    a.add_argument("--kinds", default="board,hand", help="board,hand のどちらか/両方")
    a.add_argument("--val", type=float, default=0.15)
    a.add_argument("--test", type=float, default=0.15)
    a.add_argument("--seed", type=int, default=0)

    a = sub.add_parser("frames", help="動画から注釈付け用のフレームを切り出す")
    a.add_argument("video", type=Path)
    a.add_argument("out", type=Path)
    a.add_argument("--every", type=float, default=1.0, help="何秒ごとに保存するか")
    a.add_argument("--skip-similar", type=float, default=0.0, help="直前との差がこれ未満なら保存しない（0〜255）")

    a = sub.add_parser("remap", help="注釈ツールの YOLO 書き出しを正規のクラス番号に揃える")
    a.add_argument("src", type=Path)
    a.add_argument("dst", type=Path)
    a.add_argument("--classes", type=Path, help="クラス名の一覧ファイル（省略時は src から探す）")

    a = sub.add_parser("stats", help="データセットの統計")
    a.add_argument("dataset", type=Path)

    a = sub.add_parser("train-detector", help="検出（YOLO）を学習")
    a.add_argument("dataset", type=Path)
    a.add_argument("--out", type=Path, default=Path("runs/detector"))
    a.add_argument("--model", default="yolo11n.pt", help="yolo11n.pt / yolo11s.pt / yolo11n.yaml（初期化から）など")
    a.add_argument("--epochs", type=int, default=100)
    a.add_argument("--imgsz", type=int, default=1280)
    a.add_argument("--batch", type=int, default=8)
    a.add_argument("--workers", type=int, default=2)
    a.add_argument("--device", default="auto")

    a = sub.add_parser("train-classifier", help="分類（小型CNN）を学習")
    a.add_argument("dataset", type=Path)
    a.add_argument("--out", type=Path, default=Path("runs/classifier"))
    a.add_argument("--arch", default="resnet18", choices=["resnet18", "mobilenet_v3_small"])
    a.add_argument("--epochs", type=int, default=30)
    a.add_argument("--batch", type=int, default=128)
    a.add_argument("--lr", type=float, default=2e-3)
    a.add_argument("--min-px", type=int, default=8, help="低解像度化の拡張で縮める下限（牌の短辺の画素数）")
    a.add_argument("--workers", type=int, default=2)
    a.add_argument("--device", default="auto")

    a = sub.add_parser("evaluate", help="解像度ごとの精度評価（段階A3）")
    src = a.add_mutually_exclusive_group(required=True)
    src.add_argument("--data", type=Path, help="データセット（images/<split>/, labels/<split>/）")
    src.add_argument("--video", type=Path, help="動画ファイル（--labels と一緒に）")
    a.add_argument("--split", default="test")
    a.add_argument("--labels", type=Path, help="動画のラベルのフォルダ（<動画名>_<フレーム番号6桁>.txt）")
    a.add_argument("--classifier", type=Path, required=True)
    a.add_argument("--detector", type=Path, help="省略時は分類（正解枠）だけを評価")
    sc = a.add_mutually_exclusive_group()
    sc.add_argument("--scales", type=_floats, help="縮小倍率（例 1,0.75,0.5,0.375,0.25）")
    sc.add_argument("--heights", type=_ints, help="縮小後の高さ（例 2160,1440,1080,720,540）")
    a.add_argument("--det-conf", type=float, default=0.25)
    a.add_argument("--target", type=float, default=0.99, help="必要画素数の判定に使う目標精度")
    a.add_argument("--min-n", type=int, default=30)
    a.add_argument("--out", type=Path, default=Path("runs/eval"))
    a.add_argument("--device", default="auto")

    a = sub.add_parser("predict", help="画像に認識結果を重ねて保存")
    a.add_argument("images", type=Path, nargs="+")
    a.add_argument("--detector", type=Path, required=True)
    a.add_argument("--classifier", type=Path, required=True)
    a.add_argument("--out", type=Path, default=Path("runs/predict"))
    a.add_argument("--imgsz", type=int)
    a.add_argument("--conf", type=float, default=0.25)
    a.add_argument("--device", default="auto")

    a = sub.add_parser("export", help="ONNX に書き出し")
    a.add_argument("--classifier", type=Path)
    a.add_argument("--detector", type=Path)
    a.add_argument("--out", type=Path, default=Path("runs/onnx"))
    a.add_argument("--imgsz", type=int, default=1280)

    args = p.parse_args(argv)
    return _COMMANDS[args.cmd](args) or 0


def _synth(args) -> None:
    from .synth import generate

    st = generate(args.out, args.count, args.val, args.test, args.width, args.height,
                  tuple(k.strip() for k in args.kinds.split(",")), args.seed)
    print(f"{args.out}: 画像 {st['images']} 枚、牌 {st['tiles']} 枚")


def _frames(args) -> None:
    from .frames import extract

    n = extract(args.video, args.out, args.every, args.skip_similar)
    print(f"{n} 枚保存 → {args.out}")


def _remap(args) -> None:
    from .dataset import remap

    counts = remap(args.src, args.dst, args.classes)
    print(f"{sum(counts.values())} 個のラベルを変換 → {args.dst}")
    for k, v in sorted(counts.items()):
        print(f"  {k}: {v}")


def _stats(args) -> None:
    import cv2
    import numpy as np

    from .dataset import label_path_for, list_images, read_labels
    from .tiles import CLASS_NAMES

    for split in ("train", "val", "test"):
        root = args.dataset / "images" / split
        if not root.is_dir():
            continue
        counts = [0] * len(CLASS_NAMES)
        sides = []
        imgs = list_images(root)
        for ip in imgs:
            h, w = cv2.imread(str(ip), cv2.IMREAD_COLOR).shape[:2]
            for b in read_labels(label_path_for(args.dataset, split, ip), w, h):
                counts[b.cls] += 1
                sides.append(b.short_side)
        print(f"[{split}] 画像 {len(imgs)} 枚 / 牌 {sum(counts)} 枚")
        if sides:
            q = np.percentile(sides, [5, 50, 95])
            print(f"  牌の短辺: 5% {q[0]:.1f}px / 中央 {q[1]:.1f}px / 95% {q[2]:.1f}px")
        missing = [CLASS_NAMES[i] for i, c in enumerate(counts) if c == 0]
        if missing:
            print(f"  ラベルがない種類: {' '.join(missing)}")


def _train_detector(args) -> None:
    from .detect import train

    best = train(args.dataset, args.out, args.model, args.epochs, args.imgsz, args.batch, args.device, args.workers)
    print(f"検出モデル: {best}")


def _train_classifier(args) -> None:
    from .classify import train

    train(args.dataset, args.out, args.arch, args.epochs, args.batch, args.lr, args.min_px, args.workers, args.device)


def _evaluate(args) -> None:
    from . import evaluate as ev
    from .classify import Classifier
    from .detect import Detector

    if args.video and not args.labels:
        sys.exit("--video には --labels が必要です")
    clf = Classifier(args.classifier, args.device)
    det = Detector(args.detector, args.device) if args.detector else None
    samples = ev.iter_video(args.video, args.labels) if args.video else ev.iter_dataset(args.data, args.split)
    scales = args.scales if args.scales or args.heights else [1.0, 0.75, 0.5, 0.375, 0.25]
    records, fp = ev.run(samples, clf, det, scales=scales if not args.heights else None, heights=args.heights,
                         det_conf=args.det_conf)
    if not records:
        sys.exit("評価する牌がありません（ラベルの場所を確認してください）")
    md, summary = ev.summarize(records, fp, det is not None, args.target, args.min_n)
    ev.write_outputs(args.out, records, md, summary)
    print(md)
    print(f"結果: {args.out}/summary.md, summary.json, records.csv")


def _predict(args) -> None:
    import cv2

    from .recognize import Recognizer, draw

    rec = Recognizer(args.detector, args.classifier, args.device)
    args.out.mkdir(parents=True, exist_ok=True)
    for ip in args.images:
        img = cv2.imread(str(ip), cv2.IMREAD_COLOR)
        if img is None:
            print(f"読めません: {ip}")
            continue
        res = rec(img, imgsz=args.imgsz, conf=args.conf)
        cv2.imwrite(str(args.out / f"{ip.stem}_pred.jpg"), draw(img, res))
        print(f"{ip}: {len(res)} 枚  " + " ".join(r.name for r in sorted(res, key=lambda r: (r.box.y1 // 50, r.box.x1))))


def _export(args) -> None:
    from .export import export_classifier, export_detector

    if not args.classifier and not args.detector:
        sys.exit("--classifier か --detector を指定してください")
    if args.classifier:
        print(export_classifier(args.classifier, args.out / "tile_classifier.onnx"))
    if args.detector:
        print(export_detector(args.detector, args.out / "tile_detector.onnx", args.imgsz))


_COMMANDS = {
    "synth": _synth,
    "frames": _frames,
    "remap": _remap,
    "stats": _stats,
    "train-detector": _train_detector,
    "train-classifier": _train_classifier,
    "evaluate": _evaluate,
    "predict": _predict,
    "export": _export,
}

# training/ — 牌認識の学習・評価ツール（段階A）

設計書 10章「段階A：牌認識の実現性検証（カメラ不要）」のためのツール。
**配布物（Windows zip）には含めない。**

認識は2段構成（設計書 4.10）:

```
画像 ─▶ ① 検出（Ultralytics YOLO、1クラス「牌」）─▶ 枠
     ─▶ ② 切り抜き（枠を少し広げ、横向きは縦にそろえ、96×128 に正規化）
     ─▶ ③ 分類（小型CNN: ResNet18 / MobileNetV3-Small、38クラス = 34種＋赤5×3＋裏向き）
```

## 準備

Python 3.10 以上。GPU（CUDA）があれば自動で使う。

```sh
cd training
python -m venv .venv
.venv/bin/pip install -r requirements.txt      # Windows: .venv\Scripts\pip install -r requirements.txt
.venv/bin/python -m pytest -q tests
```

以下、`python` は venv の python を指す。コマンドはすべて `training/` で実行する。

## 全体の流れ

| 段階 | やること | コマンド |
|---|---|---|
| A1 | カメラ1台で撮影（手牌カメラの位置・盤面カメラの位置の両方。距離・角度・照明を変えて複数パターン）。動画は注釈用にフレームを切り出す | `frames` |
| A2 | 注釈付け（Label Studio / CVAT）→ YOLO 形式で書き出し → クラス番号をそろえる → 学習 | `remap` → `train-detector` / `train-classifier` |
| A3 | 最大解像度の画像を縮小しながら評価し、「牌1枚が何画素あれば読めるか」を出す | `evaluate` |
| A4 | A3 の表を見て、カメラの解像度・画角・設置距離を決める | — |

実写データが揃う前でも、合成データ（`synth`）で一通り動かせる。

## データセットの形式

```
<dataset>/
  classes.txt                正規のクラス名（1m … 9m, 1p … 9p, 1s … 9s, 1z … 7z, 0m 0p 0s, back）
  images/train|val|test/<条件>/xxx.jpg
  labels/train|val|test/<条件>/xxx.txt     1行 = "class_id cx cy w h"（0〜1 に正規化）
```

- `<条件>` は任意のサブフォルダ（例: `board_80cm_light1/`, `hand_45deg/`）。**評価の表はこのフォルダ単位で分かれる**ので、撮影パターンごとに分けておくと「画角・距離ごとの精度表」になる
- 字牌の番号: 1z 東, 2z 南, 3z 西, 4z 北, 5z 白, 6z 發, 7z 中。赤5は 0m / 0p / 0s、裏向きは back

### 注釈付けのルール

- **牌全体**を囲む（立てた手牌は上に見える牌の背・側面も含める。横向きの牌は横長の枠）
- 手の陰などで**6割以上隠れた牌は付けない**
- ラベル名は上の正規名で付けるのが楽。ツールで別名（`Man1`, `中`, `5mr` など）を使っても `remap` で変換できる

### 注釈ツールからの取り込み

Label Studio / CVAT で「YOLO」形式で書き出し、次のように変換する（クラスの並びを正規の番号にそろえる）。

```sh
python -m janscope_train remap path/to/export datasets/real
python -m janscope_train stats datasets/real
```

書き出しには `images/`, `labels/` と、クラス名の一覧（`classes.txt` / `obj.names` / `data.yaml` のいずれか）が必要。
train / val / test に分けた状態で置く（同じ撮影の連続フレームが train と test に分かれないよう、**撮影単位で**分ける）。

## コマンド

### 合成データ（実写の補助・動作確認用）

```sh
python -m janscope_train synth datasets/synth --count 2000            # 1920×1080、盤面と手牌を半々
python -m janscope_train synth datasets/synth4k --count 500 --width 3840 --height 2160
```

`assets/tiles/` の牌画像（通常版・黒版）から、盤面カメラ想定（河・リーチの横向き牌・副露・山・ドラ表示牌）と
手牌カメラ想定（立てた手牌＋ツモ牌、斜め上から）の画像を作る。同じ牌は5枚以上出ない。
実際の牌セットとは見た目が違うので、**精度の判断は必ず実写で行う**。

### 動画からフレームを切り出す（A1）

```sh
python -m janscope_train frames video/board_80cm.mp4 datasets/raw/board_80cm --every 0.5 --skip-similar 3
```

ファイル名は `<動画名>_<フレーム番号6桁>.jpg`。評価の `--video` はこの番号でラベルと対応づける。

### 学習（A2）

```sh
# 検出: Ultralytics の重み（AGPL-3.0）を初期値に、1クラスで学習
python -m janscope_train train-detector datasets/real --out runs/detector --model yolo11n.pt --imgsz 1280 --epochs 100

# 分類: 初期化から学習（ImageNet の重みは配布条件が明記されていないので使わない）
python -m janscope_train train-classifier datasets/real --out runs/classifier --arch resnet18 --epochs 30
```

- 検出の `--imgsz` は学習時の入力の長辺。牌が小さいので、1080p なら 1280 程度を推奨（640 だと河の牌が潰れる）
- 分類は学習中に「一度縮めてから戻す」拡張を入れており（`--min-px`、既定 8px）、小さく写った牌にも強くなるようにしている
- 実写と合成を混ぜるときは、両方の `images/`, `labels/` を1つのデータセットにまとめる（条件フォルダで区別する）

### 解像度ごとの精度評価（A3）

```sh
# 4K で撮った画像を 2160 / 1440 / 1080 / 720 / 540 に縮小して評価
python -m janscope_train evaluate --data datasets/real --split test \
    --detector runs/detector/weights/best.pt --classifier runs/classifier/classifier.pt \
    --heights 2160,1440,1080,720,540 --out runs/eval_real

# 倍率で指定する場合
python -m janscope_train evaluate --data datasets/real --classifier runs/classifier/classifier.pt --scales 1,0.5,0.25

# 動画（ラベルは frames で切り出したフレーム番号に合わせて labels/ に置く）
python -m janscope_train evaluate --video video/board_80cm.mp4 --labels datasets/real/labels/test/board_80cm ...
```

出力（`--out`）:

| ファイル | 内容 |
|---|---|
| `summary.md` | 精度表（条件・解像度ごと / 牌の大きさごと / 必要な画素数 / 間違えやすい組み合わせ） |
| `summary.json` | 同じ内容の機械可読版 |
| `records.csv` | 正解の牌1枚ごとの記録（条件・倍率・牌の短辺の画素数・予測・確信度） |

表の見方:

- **分類（正解枠）**: 正解の枠で切り抜いて分類した正解率。検出の失敗を除いた、分類段だけの実力
- **検出の再現率 / 適合率**: IoU 0.5 以上で正解の枠と対応づけた割合
- **総合の認識率**: 正解の牌のうち、検出され、かつ種類も正しかった割合（＝牌1枚単位の認識率）
- **必要な画素数**: 「その大きさ以上の牌をまとめた精度」が目標（`--target`、既定 99%）以上になる最小の牌の短辺

縮小した画像は縮小したままの画素数で検出させる（検出器の入力を画像の長辺に合わせる）ので、
「カメラをその解像度にしたらどうなるか」の近似になる。実際のカメラの縮小（センサーのビニング・圧縮）とは差があるので、
A4 で決めた条件は段階C1 の実機で確認する。

### 目視確認

```sh
python -m janscope_train predict photo1.jpg photo2.jpg --detector runs/detector/weights/best.pt --classifier runs/classifier/classifier.pt
```

`runs/predict/` に、枠・牌名・確信度を重ねた画像を保存する（確信度 0.6 未満は赤）。

### ONNX 書き出し（本体の `ort` 用）

```sh
python -m janscope_train export --detector runs/detector/weights/best.pt --classifier runs/classifier/classifier.pt --out runs/onnx
```

`tile_detector.onnx` / `tile_classifier.onnx` と、入力の形式（大きさ・正規化・クラス名）を書いた同名の `.json` を出す。

## 合成データでの動作確認の結果

[docs/synthetic_baseline.md](docs/synthetic_baseline.md) に記録した（合成データなので実際の精度の目安にはならない。ツールが動くことの確認）。

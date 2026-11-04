# THIRD_PARTY_LICENSES

JanScope 本体は AGPL-3.0（[LICENSE](LICENSE)）。同梱・使用している第三者の素材とソフトウェアを記載する。
方針（設計書 7章）: ライセンスが明記され、AGPL-3.0 と両立するものだけを使う。表記がない・不明なものは使わない。

## 同梱している素材

### 牌画像: FluffyStuff/riichi-mahjong-tiles

- 取得元: https://github.com/FluffyStuff/riichi-mahjong-tiles
- 取得したコミット: `26e127ba2117f45cdce5ea0225748cc0cfad3169`
- 置き場所: `assets/tiles/regular/`（通常版）、`assets/tiles/black/`（黒版）の SVG
- ライセンス: CC0 1.0（パブリックドメイン）。取得時の LICENSE.md を `assets/tiles/LICENSE.md` に同梱
  > This work is in the public domain. For more information, visit https://creativecommons.org/publicdomain/zero/1.0/.
- 過去版は CC BY 4.0 だったため、念のためクレジットを記載する: Tiles by FluffyStuff

## 学習・評価ツール（`training/`）の依存

配布物（Windows zip）には含めない。開発時のみ使う。

| パッケージ | ライセンス | 用途 |
|---|---|---|
| ultralytics | AGPL-3.0 | 牌の検出（YOLO） |
| torch | BSD-3-Clause ほか寛容なライセンス | 学習 |
| torchvision | BSD-3-Clause | 分類モデルの構造（重みは使わず初期化から学習） |
| opencv-python-headless | Apache-2.0 | 画像処理 |
| numpy | BSD-3-Clause | 数値計算 |
| PyYAML | MIT | 設定ファイル |
| Pillow | MIT-CMU | 画像の読み込み |
| cairosvg | LGPL-3.0-or-later | 牌画像 SVG のラスタライズ（合成データ） |
| onnx | Apache-2.0 | ONNX 書き出し |
| onnxruntime | MIT | 書き出したモデルの確認 |
| pytest | MIT | テスト |

## 学習済みモデル

自前で学習したモデルの重みは AGPL-3.0 として公開する。既存の学習済み牌認識モデルは使わない。
検出の学習の初期値に Ultralytics 配布の重み（yolo11n.pt 等、AGPL-3.0）を使う場合がある。

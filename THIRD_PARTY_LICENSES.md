# THIRD_PARTY_LICENSES

JanScope 本体は MIT OR Apache-2.0（[LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE)）、`training/` は AGPL-3.0（[training/LICENSE](training/LICENSE)）。同梱・使用している第三者の素材とソフトウェアを記載する。
方針: ライセンスが明記され、本体の MIT OR Apache-2.0 と両立する（寛容な）ものだけを本体に使う。表記がない・不明なものは使わない。

## 同梱している素材

### 牌画像: FluffyStuff/riichi-mahjong-tiles

- 取得元: https://github.com/FluffyStuff/riichi-mahjong-tiles
- 取得したコミット: `26e127ba2117f45cdce5ea0225748cc0cfad3169`
- 置き場所: `assets/tiles/regular/`（通常版）、`assets/tiles/black/`（黒版）の SVG
- ライセンス: CC0 1.0（パブリックドメイン）。取得時の LICENSE.md を `assets/tiles/LICENSE.md` に同梱
  > This work is in the public domain. For more information, visit https://creativecommons.org/publicdomain/zero/1.0/.
- 過去版は CC BY 4.0 だったため、念のためクレジットを記載する: Tiles by FluffyStuff

### フォント: Noto Sans JP（notofonts/noto-cjk）

- 取得元: https://github.com/notofonts/noto-cjk （`Sans/SubsetOTF/JP/`）
- 取得したコミット: `f8d157532fbfaeda587e826d4cd5b21a49186f7c`
- 置き場所: `assets/fonts/NotoSansJP-Regular.otf`, `assets/fonts/NotoSansJP-Bold.otf`
- ライセンス: SIL Open Font License 1.1（全文を `assets/fonts/LICENSE-OFL.txt` に同梱）
  - フォント単体での販売は禁止。本ソフトに同梱して配布するのは可
  - 改変する場合は予約フォント名（Noto）を使わない

## 本体（Rust）の主な依存

| crate | ライセンス | 用途 |
|---|---|---|
| mahc 2.2.0 | BSD-3-Clause | 点数計算（役・符） |
| bevy 0.20 | MIT OR Apache-2.0 | 描画・ウィンドウ |
| bevy_egui 0.43 / egui 0.36 | MIT（egui は MIT OR Apache-2.0） | 操作ウィンドウ |
| resvg 0.49 | Apache-2.0 OR MIT | 牌画像（SVG）のラスタライズ |
| tokio / tokio-tungstenite / futures-util | MIT | 通信 |
| serde / serde_json / thiserror / rand / tracing | MIT OR Apache-2.0 | 共通 |

依存を追加するたびに、寛容なライセンス（MIT / Apache-2.0 / BSD / Zlib / ISC / Unicode / CC0 / OFL など）かを確認する。GPL・LGPL・AGPL のものは本体に入れない（`cargo tree` で一覧できる）。

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

既存の学習済み牌認識モデルは使わない。自前で学習した検出モデル（YOLO）の重みは AGPL-3.0 として本体とは別に配布し、分類モデル（小型CNN）の重みは MIT OR Apache-2.0 とする。
検出の学習の初期値に Ultralytics 配布の重み（yolo11n.pt 等、AGPL-3.0）を使う場合がある。

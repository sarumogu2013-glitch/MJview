# JanScope（仮称）

麻雀大会の中継システム。カメラで卓を「見て」、牌の自動読み取り・点数計算・中継画面の表示までを行う。

- 設計書: [docs/DESIGN.md](docs/DESIGN.md)（0.19.0）
- ライセンス: [AGPL-3.0](LICENSE)（同梱物の表記は [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)）

## 進み具合

設計書 10章の開発順（段階A: 牌認識の実現性検証 → 段階B: 手入力で動く本体 → 段階C: カメラ導入と自動読み取り）に沿って進める。

| 段階 | 内容 | 状態 |
|---|---|---|
| A1 | 検証用の画像・動画を撮影 | 撮影待ち（カメラ1台）。動画からのフレーム切り出しツールは用意済み |
| A2 | 注釈付けと学習・評価スクリプト（`training/`） | ツールを用意済み（合成データで動作確認） |
| A3 | 解像度ごとの精度評価CLI | ツールを用意済み（合成データで動作確認） |
| A4 | カメラの条件を決める | A1〜A3 の結果待ち |
| B〜C | 本体（Rust / Bevy） | 未着手 |

## ディレクトリ

| パス | 内容 |
|---|---|
| `docs/` | 設計書 |
| `assets/tiles/` | 牌画像（FluffyStuff/riichi-mahjong-tiles、CC0） |
| `training/` | 牌認識の学習・評価ツール（Python。配布物には含めない）。使い方は [training/README.md](training/README.md) |

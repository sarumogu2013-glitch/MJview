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
| B1 | ワークスペース・卓状態・操作ログ・Undo・サーバー⇔クライアント通信 | 済み（通信のテストあり） |
| B2 | 出力ウィンドウ・対局画面レイアウト | 済み（OBS での取り込みは実機で確認が必要） |
| B3 | 局進行・残り牌数・ドラ | 済み |
| B4 | アガリ入力 → 面子分解 → mahc → プレビュー → 放送、Python `mahjong` との突き合わせ | 済み（800件一致） |
| B5 | 選手用画面ロール（サーバー側で情報を絞る） | 済み（通信内容の記録テストあり） |
| B6 | 顔カメラの RTMP 取り込み、選手画面の自動・手動切替、黒枠＋obs-websocket 連携 | 済み（RTMP の受信と OBS 連携のテストあり。ソースごとの同期補正は B7 で） |
| B7 | 配信ディレイ（データと映像）、シーン切替とトランジション、テロップ、点数移動・アガリ演出・半荘結果・選手紹介 | 済み（OBS での見え方は実機で確認が必要） |
| B8〜B9 | 設定UI全タブ・大会通算・牌譜再生、リリース | 未着手 |
| C | カメラ導入と自動読み取り | 段階Aの後 |

## ディレクトリ

| パス | 内容 |
|---|---|
| `docs/` | 設計書 |
| `assets/tiles/` | 牌画像（FluffyStuff/riichi-mahjong-tiles、CC0） |
| `crates/core/` | 卓状態・ルール・局進行・点数計算・操作ログ・通信の型 |
| `crates/server/` | 処理サーバー `janscope-server`（ノートPC） |
| `crates/netclient/` | クライアント側の接続（自動再接続・時刻合わせ） |
| `crates/client/` | 配信クライアント `janscope-client`（出力・操作・選手用画面） |
| `assets/fonts/` | Noto Sans JP（OFL） |
| `tools/` | 開発用スクリプト（点数計算の突き合わせケース生成） |
| `training/` | 牌認識の学習・評価ツール（Python。配布物には含めない）。使い方は [training/README.md](training/README.md) |

## 動かし方（開発中）

```sh
# サーバー（ノートPC）: 接続コードが表示される
cargo run --release -p janscope-server -- --port 47800

# 配信クライアント（配信用PC）: 操作ウィンドウ＋出力ウィンドウ（1920×1080、OBS のゲームキャプチャで取り込む）
cargo run --release -p janscope-client -- --server <サーバーのIP>:47800 --code <接続コード>

# 選手用画面（配信用PCの別モニター、別プロセスで起動）
cargo run --release -p janscope-client -- --server <IP>:47800 --code <接続コード> --role player --fullscreen --monitor 1

# サーバーなしで画面だけ確認（見本の状態）
cargo run -p janscope-client -- --demo
```

主なキー: テンキー `−` / `+` で残り牌数、`F1`〜`F4` で選手画面の手動切替（`F5` で自動）、`Ctrl+Z` / `Ctrl+Y` で取り消し・やり直し。

顔カメラ（RTMP）を取り込むには ffmpeg が必要です。配布では LGPL ビルドの ffmpeg を `ffmpeg/ffmpeg.exe` として実行ファイルの隣に置きます（開発中は PATH の ffmpeg でも可）。設定は操作ウィンドウの「顔カメラ・OBS 連携・切替の設定」で変更し、`janscope-client.json` に保存されます。

テスト: `cargo test --workspace`（RTMP のテストは ffmpeg がある環境でだけ動きます）

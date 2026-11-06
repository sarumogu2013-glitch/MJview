# Claude 向けの決まりごと

## コミットの作者情報（必ず守る）

- メールアドレスは GitHub の非公開用アドレスだけを使う:
  `331298839+sarumogu2013-glitch@users.noreply.github.com`
- 個人のメールアドレス（Gmail など）は、コミットの作者・コミッター、ファイルの中身、PR やコメントのどこにも書かない
- コミットするときは次のように指定する:
  `git -c user.name="sarumogu2013-glitch" -c user.email="331298839+sarumogu2013-glitch@users.noreply.github.com" commit ...`
- push の前に `git log --format='%ae %ce' origin/<ブランチ>..HEAD` で、非公開用アドレスになっていることを確かめる

## 作業の進め方

- 自分の作業ブランチへの push は確認なしでよい。PR の作成・マージは確認してから
- 許可の確認が出にくいように、`cd <リポジトリのフォルダ> && git ...` の形で実行する（`git -C` は使わない）
- リポジトリ名は `sarumogu2013-glitch/MJview`（旧名 MJviwe。古い名前の URL は GitHub が転送する）
- 時刻は日本時間（JST, UTC+9）で伝える
- リリースは、ユーザーの動作確認を待たずに出してよい（ユーザーはソフトを更新して、その都度確認する）。版を上げて CHANGELOG.md に書き、release ワークフローを Actions から実行する

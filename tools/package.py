#!/usr/bin/env python3
"""配布用の zip を作る（設計書 9章: 解凍して exe を起動するだけで使える zip）。

    python3 tools/package.py --bin-dir target/release --suffix windows-x64 [--out dist] [--rust-licenses licenses.html]

できるもの:
    dist/janscope-<版>-<suffix>.zip         … 中身は janscope-<版>/ フォルダ1つ
    dist/janscope-<版>-<suffix>.zip.sha256  … 自動アップデートが確かめる SHA-256

zip に入れるもの: サーバー・クライアントの実行ファイル、assets/、ライセンス文、README、VERSION.txt（配布フォルダの印）。
入れないもの: 利用者の設定（janscope-client.json）・データ（data/）・ffmpeg（自分で置く。入れ替えで消えないように）、
training/ と検出モデル（AGPL-3.0。別に配布する）。
"""

import argparse
import hashlib
import pathlib
import re
import shutil
import sys
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINS = ["janscope-server", "janscope-client"]
# Windows ではランチャー（launcher/build.bat で作る JanScope.exe）も入れる
LAUNCHER = ROOT / "launcher" / "JanScope.exe"

FFMPEG_README = """顔カメラ（RTMP）の取り込みには FFmpeg が必要です。

このフォルダに ffmpeg.exe を置いてください（ffmpeg/ffmpeg.exe）。置かない場合は PATH の ffmpeg を使います。

- LGPL ビルドを使ってください（例: https://github.com/BtbN/FFmpeg-Builds/releases の「win64-lgpl」）。
  GPL ビルドでも動きますが、配布する場合は GPL の条件がかかります
- FFmpeg は JanScope とは別のプログラムとして呼び出すだけで、リンクしていません
- FFmpeg を一緒に配る場合は、FFmpeg のライセンス（LGPL-2.1 以降）の条件（ライセンス文の同梱とソースの入手方法の案内）に従ってください
- このフォルダは自動アップデートで上書きされません
"""

START_TXT = """JanScope {version}

このソフトウェアは AI（Anthropic の Claude）で作成しました。人による網羅的なコードレビューは行っていません。
大会など本番で使う前に、必ず実際の環境で動作と計算結果を確かめてください。

■ 起動
  JanScope.exe をダブルクリックすると、ツールバーのボタンから全部起動できます。
  （「ファイル → 接続の設定」でサーバーのアドレスと接続コードを入れておく。このPCでサーバーを起動すると、コードは自動で作ります）

  コマンドで起動する場合:
  サーバー（卓のノートPC）     : janscope-server.exe            （接続コードが表示されます）
  配信クライアント（配信用PC）  : janscope-client.exe --server <サーバーのIP>:47800 --code <接続コード>
  選手用画面                  : janscope-client.exe --server <IP>:47800 --code <接続コード> --role player --fullscreen --monitor 1
  サーバーなしで画面だけ確認    : janscope-client.exe --demo

■ 更新
  起動時に新しい版があるかだけ確かめます。入れ替えは、クライアントの「設定 → 更新」か
  janscope-server.exe --update で行います（配信中は行わないでください）。
  設定（janscope-client.json）・data フォルダ・ffmpeg フォルダはそのまま残ります。

■ ライセンス
  本体: MIT OR Apache-2.0（LICENSE-MIT / LICENSE-APACHE）
  同梱物・依存: THIRD_PARTY_LICENSES.md、licenses/rust-dependencies.html
  ソースコード: {repo}
"""


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'\[workspace\.package\][^\[]*?\nversion\s*=\s*"([^"]+)"', text, re.S)
    if not m:
        sys.exit("Cargo.toml に [workspace.package] version がありません")
    return m.group(1)


def repo_url() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'repository\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else ""


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(bin_dir: pathlib.Path, suffix: str, out: pathlib.Path, rust_licenses: pathlib.Path | None,
          version: str) -> pathlib.Path:
    exe = ".exe" if suffix.startswith("windows") else ""
    top = f"janscope-{version}"
    stage = out / "stage" / top
    if stage.parent.exists():
        shutil.rmtree(stage.parent)
    stage.mkdir(parents=True)

    for b in BINS:
        src = bin_dir / f"{b}{exe}"
        if not src.is_file():
            sys.exit(f"{src} がありません（cargo build --release を先に）")
        shutil.copy2(src, stage / src.name)
    if exe:
        if not LAUNCHER.is_file():
            sys.exit(f"{LAUNCHER} がありません（launcher/build.bat を先に）")
        shutil.copy2(LAUNCHER, stage / "JanScope.exe")
    shutil.copytree(ROOT / "assets", stage / "assets")
    for f in ["LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_LICENSES.md", "README.md"]:
        shutil.copy2(ROOT / f, stage / f)
    lic = stage / "licenses"
    lic.mkdir()
    if rust_licenses:
        shutil.copy2(rust_licenses, lic / "rust-dependencies.html")
    (stage / "VERSION.txt").write_text(version + "\n", encoding="utf-8")
    (stage / "はじめに.txt").write_text(START_TXT.format(version=version, repo=repo_url()), encoding="utf-8")
    # ffmpeg フォルダは案内だけ（利用者が置く。zip には ffmpeg 本体を入れない）
    (stage / "ffmpeg").mkdir()
    (stage / "ffmpeg" / "ffmpeg を置く場所.txt").write_text(FFMPEG_README, encoding="utf-8")

    out.mkdir(parents=True, exist_ok=True)
    zpath = out / f"janscope-{version}-{suffix}.zip"
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for p in sorted(stage.rglob("*")):
            if p.is_file():
                info = zipfile.ZipInfo.from_file(p, p.relative_to(stage.parent).as_posix())
                info.compress_type = zipfile.ZIP_DEFLATED
                with p.open("rb") as f:
                    z.writestr(info, f.read())
    (out / f"{zpath.name}.sha256").write_text(f"{sha256(zpath)}  {zpath.name}\n", encoding="utf-8")
    shutil.rmtree(stage.parent)
    return zpath


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin-dir", type=pathlib.Path, default=ROOT / "target" / "release")
    ap.add_argument("--suffix", default="windows-x64", help="windows-x64 / linux-x64")
    ap.add_argument("--out", type=pathlib.Path, default=ROOT / "dist")
    ap.add_argument("--rust-licenses", type=pathlib.Path, help="cargo about で作った依存ライセンスの HTML")
    ap.add_argument("--tag", help="リリースのタグ（v付き）。Cargo.toml の版と違えば止める")
    ap.add_argument("--print-version", action="store_true", help="Cargo.toml の版を表示して終わる")
    a = ap.parse_args()
    version = workspace_version()
    if a.print_version:
        print(version)
        return
    if a.tag and a.tag.lstrip("vV") != version:
        sys.exit(f"タグ {a.tag} と Cargo.toml の版 {version} が違います")
    z = build(a.bin_dir, a.suffix, a.out, a.rust_licenses, version)
    print(z)


if __name__ == "__main__":
    main()

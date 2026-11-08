//! janscope-server: 処理サーバー（ノートPCで起動）。
//!
//!     janscope-server [--port 47800] [--data ./data] [--code 123456] [--resume <file> | --resume-latest]
//!     janscope-server --check-update | --update

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use janscope_core::protocol::DEFAULT_PORT;
use janscope_server::{ServerConfig, VERSION, latest_log, new_code, start};

const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// 新しい版があるか確かめて表示する
fn print_update_check() -> bool {
    match janscope_update::Updater::github().check(VERSION) {
        Ok(janscope_update::Check::Available(r)) => {
            println!(" 新しい版 {} があります（今は {VERSION}）。", r.version);
            println!("   入れ替え: サーバーを止めてから janscope-server --update");
            println!("   変更点 : {}", r.page_url);
            true
        }
        Ok(janscope_update::Check::UpToDate { .. }) => {
            println!(" 最新の版です（{VERSION}）");
            false
        }
        Err(e) => {
            println!(" 新しい版を確かめられませんでした: {e}");
            false
        }
    }
}

/// 新しい版の情報を1行の JSON で出す（ランチャー JanScope.exe が読む）
fn print_update_info() {
    let mut v = serde_json::json!({ "current": VERSION });
    match janscope_update::Updater::github().check(VERSION) {
        Ok(janscope_update::Check::Available(r)) => {
            v["available"] = true.into();
            v["latest"] = r.version.to_string().into();
            v["notes"] = r.notes.into();
            v["url"] = r.page_url.into();
            v["has_file"] = r.zip.is_some().into();
        }
        Ok(janscope_update::Check::UpToDate { latest }) => {
            v["available"] = false.into();
            v["latest"] = latest.to_string().into();
        }
        Err(e) => {
            v["available"] = false.into();
            v["error"] = e.to_string().into();
        }
    }
    v["dist"] = janscope_update::is_dist_install().into();
    println!("{v}");
}

/// 新しい版を取ってきて入れ替える
/// `--update` のあとのオプション（ランチャーから呼ぶとき）
#[derive(Default)]
struct UpdateOpts {
    /// 入れ替える配布フォルダ（既定はこの exe のフォルダ）。ランチャーはこの exe を一時フォルダに写して動かす
    install_dir: Option<PathBuf>,
    /// この exe（ランチャー）が終わるのを待ってから入れ替える
    wait_exe: Option<PathBuf>,
    /// 終わったら起動し直す exe
    restart: Option<PathBuf>,
}

/// 実行中の exe は書き込みで開けない（Windows）。開けるようになったら終わったとみなす
fn wait_until_closed(exe: &std::path::Path) -> bool {
    println!("ランチャーが終わるのを待っています…");
    for _ in 0..120 {
        if !exe.exists() || std::fs::OpenOptions::new().append(true).open(exe).is_ok() {
            // 終わった直後はまだファイルをつかんでいることがあるので、少し待つ
            std::thread::sleep(std::time::Duration::from_millis(500));
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

fn run_update_with(opts: UpdateOpts) -> i32 {
    if let Some(w) = &opts.wait_exe {
        if !wait_until_closed(w) {
            eprintln!(
                "{} が終わらないので入れ替えません（ランチャーを閉じてから、もう一度更新してください）",
                w.display()
            );
            return finish_update(&opts, 1);
        }
    }
    let code = run_update(opts.install_dir.clone());
    finish_update(&opts, code)
}

/// 結果を見せて、ランチャーを起動し直す
fn finish_update(opts: &UpdateOpts, code: i32) -> i32 {
    let Some(r) = &opts.restart else { return code };
    if code == 0 {
        println!("ランチャーを起動し直します…");
        std::thread::sleep(std::time::Duration::from_secs(2));
    } else {
        println!("Enter を押すと閉じて、ランチャーを起動し直します（元の版のまま）");
        let mut l = String::new();
        let _ = std::io::stdin().read_line(&mut l);
    }
    let mut cmd = std::process::Command::new(r);
    if let Some(d) = r.parent() {
        cmd.current_dir(d);
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("ランチャーを起動できません（{}）: {e}", r.display());
    }
    code
}

fn run_update(install_dir: Option<PathBuf>) -> i32 {
    let Some(dir) = install_dir.or_else(janscope_update::install_dir) else {
        eprintln!("実行ファイルの場所が分かりません");
        return 1;
    };
    let up = janscope_update::Updater::github();
    let rel = match up.check(VERSION) {
        Ok(janscope_update::Check::Available(r)) => r,
        Ok(janscope_update::Check::UpToDate { latest }) => {
            println!("最新の版です（手元 {VERSION}、公開 {latest}）");
            return 0;
        }
        Err(e) => {
            eprintln!("新しい版を確かめられません: {e}");
            return 1;
        }
    };
    println!("{} を取ってきます…", rel.version);
    let mut last = 0;
    match janscope_update::download_and_apply(&up, &rel, &dir, |done, all| {
        let pct = done * 100 / all.max(1);
        if pct >= last + 10 {
            last = pct;
            println!("  {pct}%");
        }
    }) {
        Ok(a) => {
            println!(
                "{} に入れ替えました（置き換え {} 件、追加 {} 件）。起動し直してください",
                rel.version, a.replaced, a.added
            );
            0
        }
        Err(e) => {
            eprintln!("入れ替えられませんでした（元のままです）: {e}");
            1
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "janscope-server {VERSION}\n\n使い方: janscope-server [オプション]\n\
         \x20 --port <番号>      待ち受けポート（既定 {DEFAULT_PORT}）\n\
         \x20 --data <フォルダ>   操作ログの保存先（既定 ./data）\n\
         \x20 --code <6桁>       接続コード（既定: 起動ごとに乱数）\n\
         \x20 --resume <ファイル> 指定したログから続きを再開\n\
         \x20 --resume-latest    いちばん新しいログから続きを再開\n\
         \x20 --dummy            機能の確認用: ダミー対局データ（局の途中）で始める（data/dummy に保存）\n\
         \x20 --dummy-file <ファイル> 機能の確認用: 取り込んだ対局（雀魂の牌譜など）で始める\n\
         \x20 --no-update-check  起動時に新しい版を確かめない\n\
         \x20 --check-update     新しい版があるか確かめて終わる\n\
         \x20 --update           新しい版を取ってきて入れ替えて終わる（配信中は使わないこと）\n\
         \x20     [--install-dir <フォルダ>] [--wait-exe <exe>] [--restart <exe>]  ランチャー用: exe が終わるのを待ってから入れ替え、終わったら起動し直す\n\
         \x20 --update-info      新しい版の情報を JSON で出して終わる（ランチャー用）\n\n\
         ソース: {REPO_URL}（MIT OR Apache-2.0。同梱物のライセンスは THIRD_PARTY_LICENSES.md）"
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut port = DEFAULT_PORT;
    let mut data = PathBuf::from("data");
    let mut code = None;
    let mut resume = None;
    let mut resume_latest = false;
    let mut update_check = true;
    let mut dummy: Option<Vec<janscope_core::state::Action>> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => {
                port = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--data" => data = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--code" => code = Some(args.next().unwrap_or_else(|| usage())),
            "--resume" => resume = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--resume-latest" => resume_latest = true,
            "--no-update-check" => update_check = false,
            "--dummy" => dummy = Some(janscope_core::dummy::actions()),
            "--dummy-file" => {
                let p = args.next().unwrap_or_else(|| usage());
                match std::fs::read_to_string(&p)
                    .map_err(|e| e.to_string())
                    .and_then(|t| janscope_core::dummy::load(&t))
                {
                    Ok(d) => {
                        println!(" ダミーデータ: {}（{} 手）", d.title, d.actions.len());
                        dummy = Some(d.actions);
                    }
                    Err(e) => {
                        eprintln!("ダミーデータを読めません（{p}）: {e}");
                        std::process::exit(1);
                    }
                }
            }
            "--check-update" => {
                print_update_check();
                return;
            }
            "--update" => {
                let mut o = UpdateOpts::default();
                while let Some(x) = args.next() {
                    let v = args.next().map(PathBuf::from).unwrap_or_else(|| usage());
                    match x.as_str() {
                        "--install-dir" => o.install_dir = Some(v),
                        "--wait-exe" => o.wait_exe = Some(v),
                        "--restart" => o.restart = Some(v),
                        _ => usage(),
                    }
                }
                std::process::exit(run_update_with(o))
            }
            "--update-info" => {
                print_update_info();
                return;
            }
            _ => usage(),
        }
    }
    if let Some(dir) = janscope_update::install_dir() {
        janscope_update::cleanup_old(&dir);
    }
    if resume_latest {
        resume = latest_log(&data);
        if resume.is_none() {
            eprintln!(
                "再開できるログがありません（{}）。新しく始めます",
                data.join("logs").display()
            );
        }
    }
    // ダミーは本番のログ・大会通算と混ざらないように別のフォルダ
    if dummy.is_some() {
        data = data.join("dummy");
        println!(
            " ダミー対局データで始めます（機能の確認用。保存先 {}）",
            data.display()
        );
    }
    let cfg = ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
        code: code.unwrap_or_else(new_code),
        data_dir: data,
        resume,
        dummy,
    };
    let running = match start(cfg).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("起動できません: {e}");
            std::process::exit(1);
        }
    };
    println!("==============================================");
    println!(" JanScope サーバー {VERSION}");
    println!(" ポート      : {}", running.addr.port());
    println!(" 接続コード  : {}", running.code);
    if let Some(p) = &running.log_path {
        println!(" 操作ログ    : {}", p.display());
    }
    println!("==============================================");
    // 起動時に確かめるだけ（入れ替えは --update で人が行う）
    if update_check && janscope_update::is_dist_install() {
        tokio::task::spawn_blocking(print_update_check);
    }
    tokio::signal::ctrl_c().await.ok();
    println!("終了します");
}

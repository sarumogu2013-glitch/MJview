//! janscope-server: 処理サーバー（ノートPCで起動）。
//!
//!     janscope-server [--port 47800] [--data ./data] [--code 123456] [--resume <file> | --resume-latest]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use janscope_core::protocol::DEFAULT_PORT;
use janscope_server::{ServerConfig, VERSION, latest_log, new_code, start};

fn usage() -> ! {
    eprintln!(
        "janscope-server {VERSION}\n\n使い方: janscope-server [オプション]\n\
         \x20 --port <番号>      待ち受けポート（既定 {DEFAULT_PORT}）\n\
         \x20 --data <フォルダ>   操作ログの保存先（既定 ./data）\n\
         \x20 --code <6桁>       接続コード（既定: 起動ごとに乱数）\n\
         \x20 --resume <ファイル> 指定したログから続きを再開\n\
         \x20 --resume-latest    いちばん新しいログから続きを再開"
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
            _ => usage(),
        }
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
    let cfg = ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
        code: code.unwrap_or_else(new_code),
        data_dir: data,
        resume,
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
    tokio::signal::ctrl_c().await.ok();
    println!("終了します");
}

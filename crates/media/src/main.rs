//! janscope-media: メディアサーバー。カメラ（RTMP・Webカメラ）・画像・映像を受けて、映像サンドボックスの枠で切り出し、入力担当に渡す。
//!
//!     janscope-media [--rtmp-port 1935] [--http-port 47810] [--http-bind 127.0.0.1] [--data data] [--ffmpeg <パス>]

use std::net::TcpListener;
use std::path::PathBuf;

use janscope_media::{State, default_ffmpeg, now};
use janscope_video::sandbox::{DEFAULT_HTTP_PORT, DEFAULT_RTMP_PORT};

fn usage() -> ! {
    eprintln!(
        "janscope-media {}\n\nメディアサーバー: カメラ・画像・映像を受けて、映像サンドボックスの枠で切り出し、入力担当に渡します。\n\n\
         使い方: janscope-media [オプション]\n\
         \x20 --rtmp-port <番号>  カメラ（RTMP）の受け口（既定 {DEFAULT_RTMP_PORT}）\n\
         \x20 --http-port <番号>  入力担当に渡す口（既定 {DEFAULT_HTTP_PORT}）\n\
         \x20 --http-bind <IP>    入力担当に渡す口のアドレス（既定 127.0.0.1。別のPCの入力担当に渡すなら 0.0.0.0）\n\
         \x20 --data <フォルダ>    設定（sandbox.json）の置き場所（既定 data）\n\
         \x20 --ffmpeg <パス>     ffmpeg の実行ファイル（既定は隣の ffmpeg フォルダ、なければ PATH）",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

fn main() {
    let mut rtmp_port = DEFAULT_RTMP_PORT;
    let mut http_port = DEFAULT_HTTP_PORT;
    let mut bind = "127.0.0.1".to_string();
    let mut data = PathBuf::from("data");
    let mut ffmpeg = default_ffmpeg();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--rtmp-port" => rtmp_port = val().parse().unwrap_or_else(|_| usage()),
            "--http-port" => http_port = val().parse().unwrap_or_else(|_| usage()),
            "--http-bind" => bind = val(),
            "--data" => data = PathBuf::from(val()),
            "--ffmpeg" => ffmpeg = val(),
            _ => usage(),
        }
    }
    let st = State::new(rtmp_port, ffmpeg.clone(), data.join("sandbox.json"));
    println!(
        "{} JanScope メディアサーバー {}",
        now(),
        env!("CARGO_PKG_VERSION")
    );
    match st.rtmp.status().bind_error {
        Some(e) => println!("{} RTMP の受け口を開けません: {e}", now()),
        None => {
            let ip = janscope_video::rtmp_server::local_ip()
                .map(|i| i.to_string())
                .unwrap_or_else(|| "このPCのIP".into());
            println!(
                "{} カメラの送り先: rtmp://{ip}:{rtmp_port}/live/<キー>（キーは自由。届いたものは映像サンドボックスに並びます）",
                now()
            );
        }
    }
    println!("{} ffmpeg: {ffmpeg}", now());
    let l = match TcpListener::bind((bind.as_str(), http_port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "{} 入力担当に渡す口（{bind}:{http_port}）を開けません: {e}（別のメディアサーバーが動いていないか確かめてください）",
                now()
            );
            std::process::exit(1);
        }
    };
    println!(
        "{} 入力担当に渡す口: http://{bind}:{http_port}（入力担当の「映像サンドボックス」でつなぎます）",
        now()
    );
    {
        let st = st.clone();
        std::thread::spawn(move || janscope_media::pipeline::manage(st));
    }
    janscope_media::http_api::serve(l, st);
}

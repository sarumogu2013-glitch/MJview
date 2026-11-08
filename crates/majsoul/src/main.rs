//! janscope-majsoul: 雀魂の牌譜を取り込んで、JanScope のダミーデータ（対局履歴）にする。
//!
//!     janscope-majsoul [--out data/majsoul] [--keep-names] [--url URL] [--chrome PATH]
//!
//! Chrome が開くので、自分のアカウントでログインして、取り込みたい牌譜を開く（再生画面を開くだけでよい）。
//! 牌譜を開くたびに、`--out` のフォルダに JSON を保存する。サーバーの `--dummy-file` やクライアントの `--dummy-file` で使う。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use janscope_majsoul::capture::{self, Event, Shared};
use janscope_majsoul::convert;

fn usage() -> ! {
    eprintln!(
        "janscope-majsoul {}\n\n雀魂の牌譜を取り込んで、JanScope のダミーデータ（対局履歴）にします。\n\n\
         使い方: janscope-majsoul [オプション]\n\
         \x20 --out <フォルダ>   保存先（既定 data/majsoul）\n\
         \x20 --keep-names      名前をそのまま残す（既定は「選手A〜D」にする）\n\
         \x20 --url <URL>       雀魂の URL（既定 {}）\n\
         \x20 --chrome <パス>    Chrome の実行ファイル（既定は自動で探す）",
        env!("CARGO_PKG_VERSION"),
        capture::DEFAULT_URL
    );
    std::process::exit(2)
}

fn main() {
    let mut out = PathBuf::from("data").join("majsoul");
    let mut keep_names = false;
    let mut url = capture::DEFAULT_URL.to_string();
    let mut chrome = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--keep-names" => keep_names = true,
            "--url" => url = args.next().unwrap_or_else(|| usage()),
            "--chrome" => chrome = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }
    let Some(chrome) = chrome.or_else(capture::find_chrome) else {
        eprintln!(
            "Chrome が見つかりません。Google Chrome を入れるか、--chrome で場所を指定してください"
        );
        std::process::exit(1);
    };
    // ログインを覚えておくための、JanScope 専用の Chrome のプロファイル
    let profile = out.join("chrome-profile");
    let schema_file = out.join("liqi.json");
    println!("==============================================");
    println!(" 雀魂の牌譜の取り込み");
    println!(" 1. 開いた Chrome で、自分のアカウントでログインします");
    println!(" 2. 取り込みたい対局の牌譜を開きます（牌譜一覧 → 再生）");
    println!(" 3. 開くたびに、ここに保存先が出ます。終わったら Chrome を閉じてください");
    println!(
        " 名前は{}",
        if keep_names {
            "そのまま残します"
        } else {
            "「選手A〜D」にします"
        }
    );
    println!(" 保存先: {}", out.display());
    println!("==============================================");
    let browser = match capture::launch(&chrome, &profile) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let shared = Arc::new(Mutex::new(Shared::new(schema_file)));
    if shared.lock().unwrap().schema.is_some() {
        println!("（前に読み取った型定義を使います）");
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let child = Arc::new(Mutex::new(browser.child));
    {
        let (sh, tx, child, port, url) = (
            shared.clone(),
            tx.clone(),
            child.clone(),
            browser.port,
            url.clone(),
        );
        std::thread::spawn(move || {
            capture::watch_all(port, &url, sh, tx.clone(), || {
                matches!(child.lock().unwrap().try_wait(), Ok(None))
            });
            let _ = tx.send(Event::Info("__closed__".into()));
        });
    }
    drop(tx);
    let mut saved = 0;
    for ev in rx {
        match ev {
            Event::Schema(p) => println!("型定義を読み取りました（{}）", p.display()),
            Event::Info(m) if m == "__closed__" => break,
            Event::Info(m) => println!("{m}"),
            Event::GameRecord(res) => {
                let schema = match shared.lock().unwrap().schema.clone() {
                    Some(s) => s,
                    None => continue,
                };
                let result = (|| -> Result<(PathBuf, convert::Imported), String> {
                    let head = res.get("head").cloned().unwrap_or_default();
                    let data = capture::record_data(&res)?;
                    let recs = convert::records(&schema, &data)?;
                    let imp = convert::convert(&head, &recs, !keep_names)?;
                    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                    let name = if imp.uuid.is_empty() {
                        format!("majsoul-{saved}")
                    } else {
                        format!("majsoul-{}", imp.uuid)
                    };
                    let path = out.join(format!("{name}.json"));
                    let text = serde_json::to_string_pretty(&imp).map_err(|e| e.to_string())?;
                    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
                    Ok((path, imp))
                })();
                match result {
                    Ok((path, imp)) => {
                        saved += 1;
                        println!(
                            "保存しました: {}（{} 手）",
                            path.display(),
                            imp.actions.len()
                        );
                        for n in imp.notes.iter().take(5) {
                            println!("   メモ: {n}");
                        }
                        if imp.notes.len() > 5 {
                            println!("   メモ: ほか {} 件", imp.notes.len() - 5);
                        }
                    }
                    Err(e) => println!("この牌譜は取り込めませんでした: {e}"),
                }
            }
        }
    }
    println!("Chrome が閉じられたので終わります（取り込んだ牌譜: {saved} 件）");
}

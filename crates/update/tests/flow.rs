//! 確認 → 取得 → 展開 → 適用 → 後片付けを、手元に立てた小さな HTTP サーバーで通して確かめる。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use janscope_update::*;

/// パス → 中身 を返すだけの HTTP サーバー。`routes` は根元の URL を受け取って中身を作る。根元の URL を返す
fn serve(routes: impl FnOnce(&str) -> HashMap<String, Vec<u8>>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let routes = Arc::new(routes(&base));
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap() == 0 || h == "\r\n" {
                        break;
                    }
                }
                match routes.get(&path) {
                    Some(body) => {
                        write!(
                            s,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        s.write_all(body).unwrap();
                    }
                    None => {
                        write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                }
            });
        }
    });
    base
}

fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            w.start_file(*name, opt).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn sha(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("janscope-update-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SUFFIX: &str = "test-x64.zip";

/// リリース 0.2.0 を出しているサーバー
fn release_server(zip: Vec<u8>, sum: String) -> String {
    serve(move |base| {
        let name = "janscope-0.2.0-test-x64.zip";
        let json = format!(
            r#"{{"tag_name":"v0.2.0","body":"- 直したところ","html_url":"{base}/r","draft":false,"prerelease":false,
                "assets":[{{"name":"{name}","browser_download_url":"{base}/dl/{name}","size":1}},
                          {{"name":"{name}.sha256","browser_download_url":"{base}/dl/{name}.sha256","size":1}}]}}"#
        );
        HashMap::from([
            ("/repos/o/r/releases/latest".to_string(), json.into_bytes()),
            (format!("/dl/{name}"), zip),
            (
                format!("/dl/{name}.sha256"),
                format!("{sum}  {name}\n").into_bytes(),
            ),
        ])
    })
}

fn updater(base: &str) -> Updater {
    Updater::new(base, "o/r", false).with_suffix(SUFFIX)
}

fn install_with_old(name: &str) -> PathBuf {
    let d = tmpdir(name);
    std::fs::write(d.join(DIST_MARKER), "0.1.0\n").unwrap();
    std::fs::write(d.join("janscope-client.exe"), "old client").unwrap();
    std::fs::create_dir_all(d.join("assets/tiles")).unwrap();
    std::fs::write(d.join("assets/tiles/a.svg"), "old svg").unwrap();
    // 利用者のファイル（zip には入っていない）
    std::fs::write(d.join("janscope-client.json"), "{\"mine\":1}").unwrap();
    std::fs::create_dir_all(d.join("data/logs")).unwrap();
    std::fs::write(d.join("data/logs/x.jsonl"), "log").unwrap();
    d
}

fn new_zip() -> Vec<u8> {
    make_zip(&[
        ("janscope-0.2.0/VERSION.txt", b"0.2.0\n"),
        ("janscope-0.2.0/janscope-client.exe", b"new client"),
        ("janscope-0.2.0/janscope-server.exe", b"new server"),
        ("janscope-0.2.0/assets/tiles/a.svg", b"new svg"),
    ])
}

#[test]
fn check_finds_newer_and_up_to_date() {
    let z = new_zip();
    let base = release_server(z.clone(), sha(&z));
    let up = updater(&base);
    match up.check("0.1.0").unwrap() {
        Check::Available(r) => {
            assert_eq!(r.version.to_string(), "0.2.0");
            assert_eq!(r.notes, "- 直したところ");
            assert!(r.zip.is_some() && r.checksum.is_some());
        }
        c => panic!("{c:?}"),
    }
    assert!(matches!(up.check("0.2.0").unwrap(), Check::UpToDate { .. }));
    assert!(matches!(up.check("0.3.0").unwrap(), Check::UpToDate { .. }));
}

#[test]
fn full_update_keeps_user_files() {
    let z = new_zip();
    let base = release_server(z.clone(), sha(&z));
    let up = updater(&base);
    let Check::Available(rel) = up.check("0.1.0").unwrap() else {
        panic!()
    };
    let inst = install_with_old("full");
    let mut seen = 0;
    let r = download_and_apply(&up, &rel, &inst, |d, _| seen = d).unwrap();
    assert_eq!(seen, z.len() as u64);
    assert_eq!(
        r,
        Applied {
            replaced: 3,
            added: 1
        }
    );
    let read = |p: &str| std::fs::read_to_string(inst.join(p)).unwrap();
    assert_eq!(read("VERSION.txt"), "0.2.0\n");
    assert_eq!(read("janscope-client.exe"), "new client");
    assert_eq!(read("janscope-server.exe"), "new server");
    assert_eq!(read("assets/tiles/a.svg"), "new svg");
    // 利用者のファイルはそのまま
    assert_eq!(read("janscope-client.json"), "{\"mine\":1}");
    assert_eq!(read("data/logs/x.jsonl"), "log");
    // 古い版は .old に退避されていて、次の起動で消える
    assert_eq!(read("janscope-client.exe.old"), "old client");
    assert!(!inst.join(WORK_DIR).exists());
    assert_eq!(cleanup_old(&inst), 3);
    assert!(!inst.join("janscope-client.exe.old").exists());
    assert!(!inst.join("assets/tiles/a.svg.old").exists());
    let _ = std::fs::remove_dir_all(&inst);
}

#[test]
fn bad_checksum_is_rejected_and_nothing_changes() {
    let z = new_zip();
    let base = release_server(z.clone(), "00".repeat(32));
    let up = updater(&base);
    let Check::Available(rel) = up.check("0.1.0").unwrap() else {
        panic!()
    };
    let inst = install_with_old("badsum");
    let e = download_and_apply(&up, &rel, &inst, |_, _| {}).unwrap_err();
    assert!(matches!(e, Error::Checksum), "{e}");
    assert_eq!(
        std::fs::read_to_string(inst.join("janscope-client.exe")).unwrap(),
        "old client"
    );
    let _ = std::fs::remove_dir_all(&inst);
}

#[test]
fn not_applied_outside_dist_folder() {
    let z = new_zip();
    let base = release_server(z.clone(), sha(&z));
    let up = updater(&base);
    let Check::Available(rel) = up.check("0.1.0").unwrap() else {
        panic!()
    };
    let dev = tmpdir("dev");
    std::fs::write(dev.join("janscope-client"), "dev build").unwrap();
    let e = download_and_apply(&up, &rel, &dev, |_, _| {}).unwrap_err();
    assert!(matches!(e, Error::NotDist(_)), "{e}");
    assert_eq!(cleanup_old(&dev), 0);
    let _ = std::fs::remove_dir_all(&dev);
}

fn stage_bytes(name: &str, zip: Vec<u8>) -> (PathBuf, Result<Staged>) {
    let d = tmpdir(name);
    let p = d.join("x.zip");
    std::fs::write(&p, zip).unwrap();
    let r = stage(&p, &d.join("out"));
    (d, r)
}

#[test]
fn stage_rejects_traversal_and_missing_marker() {
    let (d, r) = stage_bytes(
        "trav",
        make_zip(&[("VERSION.txt", b"1"), ("../evil.txt", b"x")]),
    );
    assert!(matches!(r, Err(Error::Zip(_))));
    assert!(!d.parent().unwrap().join("evil.txt").exists());
    let _ = std::fs::remove_dir_all(&d);

    let (d, r) = stage_bytes("nomark", make_zip(&[("a/janscope-client.exe", b"x")]));
    assert!(matches!(r, Err(Error::Zip(_))));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stage_flat_zip() {
    let (d, r) = stage_bytes(
        "flat",
        make_zip(&[("VERSION.txt", b"1"), ("assets/x.txt", b"x")]),
    );
    let s = r.unwrap();
    let mut f: Vec<_> = s
        .files
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    f.sort();
    assert_eq!(f, vec!["VERSION.txt", "assets/x.txt"]);
    assert!(Path::new(&s.root.join("assets/x.txt")).is_file());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn http_errors_are_reported() {
    let base = serve(|_| HashMap::new());
    let e = updater(&base).check("0.1.0").unwrap_err();
    assert!(matches!(e, Error::NoRelease), "{e}");
    // つながらない
    let e = updater("http://127.0.0.1:9").check("0.1.0").unwrap_err();
    assert!(matches!(e, Error::Http(_)), "{e}");
}

//! 入力担当に映像と様子を渡す HTTP（std だけ。1つの接続に1つの要求、HTTP/1.0）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use janscope_video::sandbox::{MediaStatus, SandboxConfig, SourceStatus};

use crate::{State, now};

pub fn serve(l: TcpListener, st: Arc<State>) {
    for c in l.incoming().flatten() {
        let st = st.clone();
        std::thread::spawn(move || {
            let _ = handle(c, &st);
        });
    }
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        s,
        "HTTP/1.0 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)
}

fn json<T: serde::Serialize>(s: &mut TcpStream, v: &T) -> std::io::Result<()> {
    let b = serde_json::to_vec(v).unwrap_or_default();
    respond(s, 200, "application/json; charset=utf-8", &b)
}

/// 今の様子
pub fn status(st: &State) -> MediaStatus {
    let rs = st.rtmp.status();
    let cfg = st.config.lock().unwrap().clone();
    let errors = st.errors.lock().unwrap().clone();
    let sources = st
        .sources()
        .into_iter()
        .map(|s| {
            let bus = st.bus(&format!("preview/{}", s.id));
            let pubi = match &s.kind {
                janscope_video::sandbox::SourceKind::Rtmp { app, key } => rs
                    .publishers
                    .iter()
                    .find(|p| p.app == *app && p.key == *key)
                    .cloned(),
                _ => None,
            };
            SourceStatus {
                configured: cfg.sources.iter().any(|c| c.id == s.id),
                id: s.id.clone(),
                name: s.name.clone(),
                kind: s.kind.label().into(),
                live: bus.live(),
                fps: pubi.as_ref().and_then(|p| p.fps).unwrap_or(bus.fps()),
                width: pubi.as_ref().and_then(|p| p.width),
                height: pubi.as_ref().and_then(|p| p.height),
                error: errors.get(&s.id).cloned(),
            }
        })
        .collect();
    MediaStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        rtmp_port: rs.port,
        rtmp_error: rs.bind_error,
        sources,
        config: cfg,
    }
}

/// Webカメラの一覧（Windows は ffmpeg の DirectShow で調べる）
pub fn devices(st: &State) -> Vec<String> {
    if cfg!(windows) {
        let mut cmd = std::process::Command::new(&st.ffmpeg);
        cmd.args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let Ok(out) = cmd.output() else {
            return vec![];
        };
        let text = String::from_utf8_lossy(&out.stderr).into_owned();
        parse_dshow(&text)
    } else {
        let mut v: Vec<String> = std::fs::read_dir("/dev")
            .map(|d| {
                d.flatten()
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .filter(|p| p.starts_with("/dev/video"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// `[dshow @ ...] "名前" (video)` の行から名前を取り出す
pub fn parse_dshow(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("(video)"))
        .filter_map(|l| {
            let a = l.find('"')?;
            let b = l[a + 1..].find('"')? + a + 1;
            Some(l[a + 1..b].to_string())
        })
        .collect()
}

fn handle(mut s: TcpStream, st: &State) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts
        .next()
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len.min(4 * 1024 * 1024)];
    r.read_exact(&mut body)?;
    let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method.as_str(), seg.as_slice()) {
        ("GET", ["status"]) => json(&mut s, &status(st)),
        ("GET", ["config"]) => json(&mut s, &*st.config.lock().unwrap()),
        ("PUT", ["config"]) => match serde_json::from_slice::<SandboxConfig>(&body) {
            Ok(c) => match st.set_config(c) {
                Ok(()) => {
                    println!("{} 設定を変えました（枠・ソース）", now());
                    json(&mut s, &status(st))
                }
                Err(e) => respond(&mut s, 500, "text/plain; charset=utf-8", e.as_bytes()),
            },
            Err(e) => respond(
                &mut s,
                400,
                "text/plain; charset=utf-8",
                format!("設定を読めません: {e}").as_bytes(),
            ),
        },
        ("GET", ["devices"]) => json(&mut s, &devices(st)),
        ("GET", [kind @ ("preview" | "snapshot" | "region"), file]) => {
            let (name, ext) = file.rsplit_once('.').unwrap_or((file, ""));
            let bus_name = match *kind {
                "region" => format!("region/{name}"),
                _ => format!("preview/{name}"),
            };
            let bus = st.bus(&bus_name);
            if ext == "mjpg" {
                stream(&mut s, &bus)
            } else {
                match bus.latest() {
                    Some((_, f)) => respond(&mut s, 200, "image/jpeg", &f),
                    None => respond(
                        &mut s,
                        404,
                        "text/plain; charset=utf-8",
                        "まだ映像がありません".as_bytes(),
                    ),
                }
            }
        }
        ("GET", [""]) => respond(
            &mut s,
            200,
            "text/plain; charset=utf-8",
            "JanScope メディアサーバー（/status /config /preview/<ソース>.mjpg /region/<枠>.mjpg）"
                .as_bytes(),
        ),
        _ => respond(&mut s, 404, "text/plain; charset=utf-8", b"not found"),
    }
}

/// MJPEG の流れ（multipart/x-mixed-replace）。相手が切るまで
fn stream(s: &mut TcpStream, bus: &crate::Bus) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut seq = 0;
    if let Some((q, f)) = bus.latest() {
        seq = q;
        part(s, &f)?;
    }
    loop {
        if let Some((q, f)) = bus.wait_newer(seq, Duration::from_secs(5)) {
            seq = q;
            part(s, &f)?;
        } else {
            // 映像が止まっていても、相手が切ったか確かめる
            s.write_all(b"\r\n")?;
        }
    }
}

fn part(s: &mut TcpStream, f: &[u8]) -> std::io::Result<()> {
    write!(
        s,
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        f.len()
    )?;
    s.write_all(f)?;
    s.write_all(b"\r\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn dshow_names() {
        let t = "[dshow @ 0x1] \"USB Video\" (video)\n[dshow @ 0x1]   Alternative name \"@device\"\n[dshow @ 0x1] \"Mic\" (audio)\n";
        assert_eq!(super::parse_dshow(t), vec!["USB Video".to_string()]);
    }
}

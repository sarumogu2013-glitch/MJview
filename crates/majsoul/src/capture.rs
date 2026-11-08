//! Chrome を開いて、雀魂の通信から牌譜を読み取る。
//!
//! - Chrome を JanScope 専用のプロファイル（ログインはここに残る）と、リモートデバッグ（DevTools プロトコル）付きで起動する
//! - 開いているタブごとに DevTools の WebSocket につなぎ、Network の出来事を受け取る:
//!   - ゲームが読み込む型定義（`liqi*.json`）の中身 → protobuf の型（ファイルにも保存して次回も使う）
//!   - ゲームの WebSocket のフレーム（送った要求・受け取った応答）→ 牌譜の応答（`fetchGameRecord`）を見つける
//! - 対局をリアルタイムに読むことはしない。ユーザーが自分で開いた牌譜のデータだけを使う

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};

use crate::frames::{Message, Tracker};
use crate::proto::Schema;

/// 雀魂（日本のサーバー）
pub const DEFAULT_URL: &str = "https://game.mahjongsoul.com/";

/// Chrome の実行ファイルを探す（`JANSCOPE_CHROME` があればそれ）
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("JANSCOPE_CHROME") {
        return Some(PathBuf::from(p));
    }
    let mut cands: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                cands.push(Path::new(&base).join(r"Google\Chrome\Application\chrome.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        cands.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    } else {
        for n in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ] {
            for dir in std::env::var("PATH").unwrap_or_default().split(':') {
                cands.push(Path::new(dir).join(n));
            }
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

/// 起動した Chrome。drop しても Chrome は閉じない（ユーザーが閉じる）
pub struct Browser {
    pub child: Child,
    pub port: u16,
}

/// Chrome を空のページで起動して、DevTools のポートが分かるまで待つ（雀魂へは、通信を読み始めてから移る）
pub fn launch(chrome: &Path, profile: &Path) -> Result<Browser, String> {
    std::fs::create_dir_all(profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    let port_file = profile.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);
    let child = Command::new(chrome)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg("--remote-allow-origins=*")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .args(
            std::env::var("JANSCOPE_CHROME_ARGS")
                .unwrap_or_default()
                .split_whitespace(),
        )
        .arg("about:blank")
        .spawn()
        .map_err(|e| format!("Chrome を起動できません（{}）: {e}", chrome.display()))?;
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if let Ok(t) = std::fs::read_to_string(&port_file) {
            if let Some(p) = t.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
                return Ok(Browser { child, port: p });
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("Chrome の DevTools のポートが分かりません（すでに同じプロファイルの Chrome が開いていないか確かめてください）".into())
}

/// `http://127.0.0.1:port/json/list` を読む（小さな HTTP）
fn http_get(port: u16, path: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    // Chrome は接続を閉じないことがあるので、Content-Length ぶん読んだら終わる
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = s.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let len = head.lines().find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
            });
            if let Some(len) = len {
                if buf.len() >= pos + 4 + len {
                    return Ok(String::from_utf8_lossy(&buf[pos + 4..pos + 4 + len]).into_owned());
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default())
}

/// 開いているタブ（id と DevTools の WebSocket の宛先）
pub fn pages(port: u16) -> Vec<(String, String)> {
    let Ok(body) = http_get(port, "/json/list") else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return vec![];
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                .filter_map(|t| {
                    Some((
                        t.get("id")?.as_str()?.to_string(),
                        t.get("webSocketDebuggerUrl")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 読み取りの途中経過
#[derive(Debug)]
pub enum Event {
    /// 型定義を読み取った（保存した場所）
    Schema(PathBuf),
    /// 牌譜の応答を受け取った（`ResGameRecord` の中身）
    GameRecord(Value),
    /// 知らせ
    Info(String),
}

/// 型定義と、型定義が来る前に受け取ったフレーム
pub struct Shared {
    pub schema: Option<Arc<Schema>>,
    schema_file: PathBuf,
    /// 型定義がまだないときのフレーム（送った=true / 受け取った=false、中身）
    backlog: Vec<(bool, Vec<u8>)>,
    tracker: Tracker,
}

impl Shared {
    pub fn new(schema_file: PathBuf) -> Shared {
        let schema = std::fs::read_to_string(&schema_file)
            .ok()
            .and_then(|t| Schema::from_json(&t).ok())
            .map(Arc::new);
        Shared {
            schema,
            schema_file,
            backlog: vec![],
            tracker: Tracker::default(),
        }
    }

    fn frame(&mut self, sent: bool, data: Vec<u8>, tx: &Sender<Event>) {
        let Some(schema) = self.schema.clone() else {
            self.backlog.push((sent, data));
            if self.backlog.len() > 2000 {
                self.backlog.drain(..500);
            }
            return;
        };
        self.handle(&schema, sent, &data, tx);
    }

    fn handle(&mut self, schema: &Schema, sent: bool, data: &[u8], tx: &Sender<Event>) {
        if sent {
            self.tracker.on_sent(schema, data);
        } else if let Some(Message { name, body }) = self.tracker.on_received(schema, data) {
            if name.ends_with("fetchGameRecord") {
                let _ = tx.send(Event::GameRecord(body));
            }
        }
    }

    fn set_schema(&mut self, text: &str, tx: &Sender<Event>) {
        match Schema::from_json(text) {
            Ok(s) if s.types.contains_key("lq.Wrapper") => {
                let s = Arc::new(s);
                if let Some(p) = self.schema_file.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = std::fs::write(&self.schema_file, text);
                let _ = tx.send(Event::Schema(self.schema_file.clone()));
                for (sent, d) in std::mem::take(&mut self.backlog) {
                    self.handle(&s, sent, &d, tx);
                }
                self.schema = Some(s);
            }
            _ => {}
        }
    }
}

/// 1つのタブの DevTools につないで、閉じられるまで読む。`navigate` があれば、読み始めてからそのページへ移る
pub fn watch_page(
    ws_url: &str,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Event>,
    navigate: Option<String>,
) -> Result<(), String> {
    // 起動直後は DevTools がまだ受け付けないことがあるので、少し待ってつなぎ直す
    let mut tries = 0;
    let (mut ws, _) = loop {
        match tungstenite::connect(ws_url) {
            Ok(c) => break c,
            Err(e) if tries < 40 => {
                tries += 1;
                if std::env::var_os("JANSCOPE_MAJSOUL_DEBUG").is_some() {
                    eprintln!("[debug] connect retry {tries}: {e}");
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(format!("DevTools につながりません: {e}")),
        }
    };
    let send = |ws: &mut tungstenite::WebSocket<_>, id: u64, method: &str, params: Value| {
        let msg = json!({"id": id, "method": method, "params": params}).to_string();
        ws.send(tungstenite::Message::text(msg))
    };
    send(&mut ws, 1, "Network.enable", json!({"maxPostDataSize": 0})).map_err(|e| e.to_string())?;
    if let Some(url) = navigate {
        send(&mut ws, 2, "Page.navigate", json!({"url": url})).map_err(|e| e.to_string())?;
    }
    let mut next_id = 10u64;
    // 型定義の読み込み: requestId → 返事を待っている id
    let mut liqi_requests: HashSet<String> = HashSet::new();
    let mut body_calls: HashMap<u64, String> = HashMap::new();
    loop {
        let msg = match ws.read() {
            Ok(m) => m,
            Err(_) => return Ok(()), // タブが閉じられた
        };
        let tungstenite::Message::Text(t) = msg else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else {
            continue;
        };
        // 返事（getResponseBody）
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            if body_calls.remove(&id).is_some() {
                if let Some(r) = v.get("result") {
                    let body = r.get("body").and_then(Value::as_str).unwrap_or("");
                    let text = if r.get("base64Encoded").and_then(Value::as_bool) == Some(true) {
                        base64::engine::general_purpose::STANDARD
                            .decode(body)
                            .map(|b| String::from_utf8_lossy(&b).into_owned())
                            .unwrap_or_default()
                    } else {
                        body.to_string()
                    };
                    shared.lock().unwrap().set_schema(&text, &tx);
                }
            }
            continue;
        }
        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
        let p = v.get("params").cloned().unwrap_or(Value::Null);
        if std::env::var_os("JANSCOPE_MAJSOUL_DEBUG").is_some() && method.starts_with("Network.") {
            let url = p
                .pointer("/response/url")
                .or_else(|| p.pointer("/request/url"))
                .and_then(Value::as_str)
                .unwrap_or("");
            eprintln!("[debug] {method} {url}");
        }
        match method {
            "Network.responseReceived" => {
                let url = p
                    .pointer("/response/url")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let file = url
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("");
                if file.starts_with("liqi") && file.ends_with(".json") {
                    if let Some(id) = p.get("requestId").and_then(Value::as_str) {
                        liqi_requests.insert(id.to_string());
                    }
                }
            }
            "Network.loadingFinished" => {
                if let Some(id) = p.get("requestId").and_then(Value::as_str) {
                    if liqi_requests.remove(id) {
                        next_id += 1;
                        body_calls.insert(next_id, id.to_string());
                        let _ = send(
                            &mut ws,
                            next_id,
                            "Network.getResponseBody",
                            json!({"requestId": id}),
                        );
                    }
                }
            }
            "Network.webSocketFrameSent" | "Network.webSocketFrameReceived" => {
                // opcode 2 = バイナリ（payloadData は base64）
                if p.pointer("/response/opcode").and_then(Value::as_i64) != Some(2) {
                    continue;
                }
                let data = p
                    .pointer("/response/payloadData")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
                    continue;
                };
                shared
                    .lock()
                    .unwrap()
                    .frame(method.ends_with("Sent"), bytes, &tx);
            }
            _ => {}
        }
    }
}

/// 新しく開いたタブにもつなぐ（Chrome が閉じられるまで）。最初のタブは `url` へ移る
pub fn watch_all(
    port: u16,
    url: &str,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Event>,
    alive: impl Fn() -> bool,
) {
    let mut watching: HashSet<String> = HashSet::new();
    let mut first = Some(url.to_string());
    while alive() {
        for (id, ws) in pages(port) {
            if watching.insert(id) {
                let (sh, tx2, nav) = (shared.clone(), tx.clone(), first.take());
                std::thread::spawn(move || {
                    if let Err(e) = watch_page(&ws, sh, tx2.clone(), nav) {
                        let _ = tx2.send(Event::Info(e));
                    }
                });
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// 牌譜の応答の中身（bytes）。`data` が空なら `data_url` から取ってくる
pub fn record_data(res: &Value) -> Result<Vec<u8>, String> {
    let data = crate::proto::unb64(res.get("data").unwrap_or(&Value::Null));
    if !data.is_empty() {
        return Ok(data);
    }
    let url = res.get("data_url").and_then(Value::as_str).unwrap_or("");
    if url.is_empty() {
        return Err("牌譜の応答に中身がありません".into());
    }
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| format!("牌譜を取ってこられません: {e}"))?;
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("牌譜を取ってこられません: {e}"))?;
    Ok(buf)
}

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
        // Chrome のログはこの画面に出さない。この画面の入力（貼った URL）は取り込みが読む
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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

/// 牌譜の URL（共有の `…?paipu=…`）か牌譜の ID から、開く URL を作る。どちらでもなければ None
pub fn paipu_url(input: &str) -> Option<String> {
    let t = input.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return t.contains("paipu=").then(|| t.to_string());
    }
    let id = t.strip_prefix("paipu=").unwrap_or(t);
    let ok = id.len() >= 10
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| format!("{DEFAULT_URL}?paipu={id}"))
}

/// 動いている Chrome の DevTools のポート（プロファイルの DevToolsActivePort から）
pub fn running_port(profile: &Path) -> Option<u16> {
    let t = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port = t.lines().next()?.trim().parse::<u16>().ok()?;
    TcpStream::connect(("127.0.0.1", port)).ok()?;
    Some(port)
}

/// 動いている Chrome で、新しいタブにその URL を開く（新しいタブも読み取りの対象になる）
pub fn open_tab(port: u16, url: &str) -> Result<(), String> {
    let body = http(port, "PUT", &format!("/json/new?{url}"))?;
    if body.contains("webSocketDebuggerUrl") {
        Ok(())
    } else {
        Err(format!("タブを開けません: {}", body.trim()))
    }
}

/// `http://127.0.0.1:port/json/list` を読む（小さな HTTP）
fn http_get(port: u16, path: &str) -> Result<String, String> {
    http(port, "GET", path)
}

fn http(port: u16, method: &str, path: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
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
    /// 型定義を待っていることを知らせた
    told_waiting: bool,
    /// ゲームのページの場所（`https://game.mahjongsoul.com/` など。型定義を直接取ってくるのに使う）
    base: Option<String>,
    /// 型定義を直接取りに行っている・最後に取りに行った時刻
    fetching: bool,
    last_fetch: Option<Instant>,
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
            told_waiting: false,
            base: None,
            fetching: false,
            last_fetch: None,
        }
    }

    /// 型定義がまだなく、取りに行っていなければ、取りに行く場所を返す（20秒に1回まで）
    fn want_fetch(&mut self) -> Option<String> {
        if self.schema.is_some() || self.fetching {
            return None;
        }
        if self
            .last_fetch
            .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
        {
            return None;
        }
        self.fetching = true;
        self.last_fetch = Some(Instant::now());
        Some(self.base.clone().unwrap_or_else(|| DEFAULT_URL.to_string()))
    }

    fn frame(&mut self, sent: bool, data: Vec<u8>, tx: &Sender<Event>) {
        let Some(schema) = self.schema.clone() else {
            self.backlog.push((sent, data));
            if !self.told_waiting {
                self.told_waiting = true;
                let _ = tx.send(Event::Info(
                    "ゲームの通信を受け取っています。型定義がまだないので、届くまでためておきます"
                        .into(),
                ));
            }
            if self.backlog.len() > 2000 {
                self.backlog.drain(..500);
            }
            return;
        };
        self.handle(&schema, sent, &data, tx);
    }

    fn handle(&mut self, schema: &Schema, sent: bool, data: &[u8], tx: &Sender<Event>) {
        let info = |m: String| {
            let _ = tx.send(Event::Info(m));
        };
        if sent {
            if let Some(name) = self.tracker.on_sent(schema, data) {
                if name.ends_with("fetchGameRecord") {
                    info("牌譜を要求しました。応答を待っています…".into());
                }
            }
            return;
        }
        let Some(Message { name, body }) = self.tracker.on_received(schema, data) else {
            return;
        };
        let short = name.rsplit('.').next().unwrap_or(&name);
        if body.is_null() {
            if short == "fetchGameRecord" {
                info("牌譜の応答を読めませんでした（型定義が合っていない可能性があります。ページを再読み込みしてください）".into());
            }
            return;
        }
        let err = body
            .pointer("/error/code")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        match short {
            "fetchGameRecord" if err != 0 => {
                info(format!("牌譜を取得できませんでした（雀魂のエラー {err}）"))
            }
            "fetchGameRecord" => {
                info("牌譜を受け取りました。変換します…".into());
                let _ = tx.send(Event::GameRecord(body));
            }
            "fetchGameRecordList" | "fetchGameRecordsDetail" if err == 0 => {
                let n = body
                    .get("record_list")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                info(format!(
                    "牌譜一覧を読み込みました（{n} 件）。取り込みたい対局を開いてください"
                ));
            }
            "login" | "oauth2Login" | "emailLogin" | "loginBeat" if err == 0 => {
                info("雀魂にログインしました".into())
            }
            "login" | "oauth2Login" | "emailLogin" => {
                info(format!("ログインできませんでした（雀魂のエラー {err}）"))
            }
            _ => {}
        }
    }

    fn set_schema(&mut self, text: &str, tx: &Sender<Event>) {
        if self.schema.is_some() {
            return;
        }
        match Schema::from_json(text) {
            Err(e) => {
                let _ = tx.send(Event::Info(format!("型定義を読めませんでした: {e}")));
            }
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

/// 型定義（liqi.json）を雀魂のサーバーから直接取ってくる。
/// ゲームが型定義を読み込むところを見つけられなかったときに使う（`version.json` → `resversion<版>.json` → `<prefix>/res/proto/liqi.json`）
pub fn fetch_schema(base: &str) -> Result<String, String> {
    let get = |u: &str| -> Result<String, String> {
        ureq::get(u)
            .call()
            .map_err(|e| format!("{u}: {e}"))?
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("{u}: {e}"))
    };
    let json = |u: &str| -> Result<Value, String> {
        serde_json::from_str(&get(u)?).map_err(|e| format!("{u}: {e}"))
    };
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let v = json(&format!("{base}version.json?randv={t}"))?;
    let ver = v
        .get("version")
        .and_then(Value::as_str)
        .ok_or("version.json に版がありません")?;
    let r = json(&format!("{base}resversion{ver}.json"))?;
    let prefix = r
        .pointer("/res/res~1proto~1liqi.json/prefix")
        .and_then(Value::as_str)
        .ok_or("resversion に型定義（res/proto/liqi.json）がありません")?;
    get(&format!("{base}{prefix}/res/proto/liqi.json"))
}

/// 型定義を直接取ってきて入れる（別のスレッドで）
fn spawn_fetch(base: String, shared: Arc<Mutex<Shared>>, tx: Sender<Event>) {
    std::thread::spawn(move || {
        let _ = tx.send(Event::Info(format!(
            "ゲームが読み込んだ型定義が見つからないので、雀魂のサーバーから直接取ってきます（{base}）…"
        )));
        let r = fetch_schema(&base);
        let mut sh = shared.lock().unwrap();
        sh.fetching = false;
        match r {
            Ok(text) => sh.set_schema(&text, &tx),
            Err(e) => {
                let _ = tx.send(Event::Info(format!(
                    "型定義を取ってこられませんでした（20秒後にまた試します）: {e}"
                )));
            }
        }
    });
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
                if let Some(e) = v.pointer("/error/message").and_then(Value::as_str) {
                    let _ = tx.send(Event::Info(format!(
                        "ゲームが読み込んだ型定義の中身を取れませんでした（{e}）。あとで直接取りに行きます"
                    )));
                }
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
        let info = |m: String| {
            let _ = tx.send(Event::Info(m));
        };
        match method {
            "Network.webSocketCreated" => {
                info("ゲームのサーバーにつなぎました（通信を読んでいます）".into());
            }
            "Network.webSocketClosed" => info("ゲームのサーバーとの接続が切れました".into()),
            "Network.loadingFailed" => {
                if let Some(id) = p.get("requestId").and_then(Value::as_str) {
                    if liqi_requests.remove(id) {
                        info(
                            "型定義の読み込みに失敗しました（ページを再読み込みしてください）"
                                .into(),
                        );
                    }
                }
            }
            "Network.responseReceived" => {
                let url = p
                    .pointer("/response/url")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if p.get("type").and_then(Value::as_str) == Some("Document") {
                    let status = p
                        .pointer("/response/status")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    if status >= 400 {
                        info(format!("ページを開けませんでした（{status}）: {url}"));
                    } else if !url.starts_with("about:") {
                        info(format!("ページを開きました: {url}"));
                        // ゲームのページの場所（最初に開いたページ、または雀魂のページ）
                        let game = ["mahjongsoul", "maj-soul", "majsoul"]
                            .iter()
                            .any(|h| url.contains(h));
                        let mut sh = shared.lock().unwrap();
                        if url.starts_with("http") && (game || sh.base.is_none()) {
                            let path = url.split(['?', '#']).next().unwrap_or(url);
                            if let Some(i) = path.rfind('/') {
                                if i > path.find("//").map(|j| j + 1).unwrap_or(0) {
                                    sh.base = Some(path[..=i].to_string());
                                } else {
                                    sh.base = Some(format!("{path}/"));
                                }
                            }
                        }
                    }
                }
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
                        info("型定義（liqi.json）を読み込んでいます…".into());
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
                let fetch = {
                    let mut sh = shared.lock().unwrap();
                    sh.frame(method.ends_with("Sent"), bytes, &tx);
                    sh.want_fetch()
                };
                if let Some(base) = fetch {
                    spawn_fetch(base, shared.clone(), tx.clone());
                }
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
                    let _ = tx2.send(Event::Info("Chrome のタブにつなぎました".into()));
                    match watch_page(&ws, sh, tx2.clone(), nav) {
                        Ok(()) => {
                            let _ = tx2.send(Event::Info("タブが閉じられました".into()));
                        }
                        Err(e) => {
                            let _ = tx2.send(Event::Info(e));
                        }
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

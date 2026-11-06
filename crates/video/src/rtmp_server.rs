//! 本ソフトに内蔵した RTMP の受け口。1つのポート（既定 1935）で、何台分のカメラでもストリームキーで分けて受ける。
//!
//! - カメラ（GoPro・OBS など）は `rtmp://<このPCのIP>:1935/<app>/<キー>` に送る
//! - 受けた映像（H.264 など。FLV のタグそのまま）は、キーごとに登録した受け手（[`Subscription`]）へ渡す。
//!   受け手は ffmpeg に FLV として流し込んでデコードする（[`crate::FaceStream`]）
//! - 受け手のいないキーから届いた映像も、一覧（[`ServerStatus::publishers`]）には出す（設定の手がかりにする）
//! - 同じキーで2台目が送ってきたら断る
//! - 音は使わないので受け取って捨てる

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult,
};

/// 受け手に渡す、1回分の配信（カメラが送り始めてから止めるまで）。
pub struct Session {
    /// FLV のバイト列（先頭はファイルヘッダー）。送り終わると切れる
    pub flv: Receiver<Vec<u8>>,
}

/// 送ってきているカメラ1台分の様子。
#[derive(Clone, Debug, PartialEq)]
pub struct PublisherInfo {
    pub app: String,
    pub key: String,
    pub remote: SocketAddr,
    /// 受け手がいる（どこかの席に割り当てられている）
    pub registered: bool,
    pub since: Instant,
    /// カメラが知らせてきた解像度・フレームレート・ビットレート（知らせてこないカメラもある）
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub encoder: Option<String>,
    /// 実際に届いている映像のビットレート（直近の数秒）
    pub kbps: u32,
    pub video_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerStatus {
    pub port: u16,
    /// ポートを開けられなかったときの理由
    pub bind_error: Option<String>,
    pub publishers: Vec<PublisherInfo>,
}

struct Pub {
    info: PublisherInfo,
    window_start: Instant,
    window_bytes: u64,
    /// いま映像を流している受け手への道（受け手がいないか、受け手が止まったら None）
    tx: Option<SyncSender<Vec<u8>>>,
    /// H.264 の初期化データ（シーケンスヘッダー）のタグ。途中から受け手を付けるときに最初に送る
    seq_header: Option<Vec<u8>>,
}

#[derive(Default)]
struct Inner {
    port: u16,
    bind_error: Mutex<Option<String>>,
    /// "app/key" → (登録の番号, 受け手)
    subs: Mutex<HashMap<String, (u64, Sender<Session>)>>,
    next_token: std::sync::atomic::AtomicU64,
    /// "app/key" → 送ってきているカメラ
    pubs: Mutex<HashMap<String, Pub>>,
}

/// 受け口への参照。全部の参照がなくなると受け口を閉じる。
#[derive(Clone)]
pub struct RtmpServer {
    inner: Arc<Inner>,
}

/// キーの受け手。drop すると登録を外す。
pub struct Subscription {
    server: RtmpServer,
    id: String,
    token: u64,
    pub sessions: Receiver<Session>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // 後から同じキーを登録し直した受け手の分は消さない（設定を適用し直したとき、新しい受け手が先にできる）
        let mut subs = self.server.inner.subs.lock().unwrap();
        if subs.get(&self.id).is_some_and(|(t, _)| *t == self.token) {
            subs.remove(&self.id);
        }
    }
}

fn servers() -> &'static Mutex<HashMap<u16, Weak<Inner>>> {
    static S: OnceLock<Mutex<HashMap<u16, Weak<Inner>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn stream_id(app: &str, key: &str) -> String {
    format!("{app}/{key}")
}

impl RtmpServer {
    /// `port` の受け口を使う（なければ開く）。開けなかったときも返り、理由は [`RtmpServer::status`] に出る
    pub fn shared(port: u16) -> RtmpServer {
        let mut map = servers().lock().unwrap();
        if let Some(inner) = map.get(&port).and_then(Weak::upgrade) {
            return RtmpServer { inner };
        }
        let inner = Arc::new(Inner {
            port,
            ..Default::default()
        });
        // 前の受け口が閉じきるまで少し待つことがある
        let mut bound = TcpListener::bind(("0.0.0.0", port));
        for _ in 0..20 {
            match &bound {
                Err(e) if e.kind() == ErrorKind::AddrInUse => {
                    std::thread::sleep(Duration::from_millis(50));
                    bound = TcpListener::bind(("0.0.0.0", port));
                }
                _ => break,
            }
        }
        match bound {
            Ok(l) => {
                let _ = l.set_nonblocking(true);
                let weak = Arc::downgrade(&inner);
                std::thread::Builder::new()
                    .name(format!("janscope-rtmp-{port}"))
                    .spawn(move || accept_loop(l, weak))
                    .expect("thread");
            }
            Err(e) => {
                let msg = if e.kind() == ErrorKind::AddrInUse {
                    format!(
                        "ポート {port} は他のソフトが使っています（OBS・SRS・別の JanScope など）。そちらを止めるか、設定で別のポートにしてください"
                    )
                } else {
                    format!("ポート {port} を開けません: {e}")
                };
                *inner.bind_error.lock().unwrap() = Some(msg);
            }
        }
        map.insert(port, Arc::downgrade(&inner));
        RtmpServer { inner }
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// `app/key` の映像を受け取る。同じキーの受け手は1つだけ（後から登録したほうが勝つ）
    pub fn subscribe(&self, app: &str, key: &str) -> Subscription {
        let (tx, rx) = std::sync::mpsc::channel();
        let id = stream_id(app, key);
        let token = self
            .inner
            .next_token
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .subs
            .lock()
            .unwrap()
            .insert(id.clone(), (token, tx));
        // もう送ってきているカメラなら、次のキーフレームからこの受け手に流す（前の受け手への道は切る）
        if let Some(p) = self.inner.pubs.lock().unwrap().get_mut(&id) {
            p.info.registered = true;
            p.tx = None;
        }
        Subscription {
            server: self.clone(),
            id,
            token,
            sessions: rx,
        }
    }

    pub fn status(&self) -> ServerStatus {
        let subs = self.inner.subs.lock().unwrap();
        let mut pubs: Vec<PublisherInfo> = self
            .inner
            .pubs
            .lock()
            .unwrap()
            .iter_mut()
            .map(|(id, p)| {
                let el = p.window_start.elapsed();
                if el >= Duration::from_secs(2) {
                    p.info.kbps = (p.window_bytes * 8 / el.as_millis().max(1) as u64) as u32;
                    p.window_start = Instant::now();
                    p.window_bytes = 0;
                }
                let mut i = p.info.clone();
                i.registered = subs.contains_key(id);
                i
            })
            .collect();
        pubs.sort_by(|a, b| (&a.app, &a.key).cmp(&(&b.app, &b.key)));
        ServerStatus {
            port: self.inner.port,
            bind_error: self.inner.bind_error.lock().unwrap().clone(),
            publishers: pubs,
        }
    }
}

/// いま開いている受け口の様子（画面の表示用）。開いていなければ None
pub fn server_status(port: u16) -> Option<ServerStatus> {
    let inner = servers()
        .lock()
        .unwrap()
        .get(&port)
        .and_then(Weak::upgrade)?;
    Some(RtmpServer { inner }.status())
}

fn accept_loop(l: TcpListener, inner: Weak<Inner>) {
    loop {
        match l.accept() {
            Ok((s, addr)) => {
                let w = inner.clone();
                let _ = std::thread::Builder::new()
                    .name("janscope-rtmp-conn".into())
                    .spawn(move || {
                        let _ = handle_conn(s, addr, w);
                    });
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if inner.strong_count() == 0 {
                    return; // 誰も使っていないので閉じる
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// FLV のファイルヘッダー（映像のみ）と、最初の PreviousTagSize0
pub fn flv_header() -> Vec<u8> {
    let mut v = b"FLV\x01\x01\x00\x00\x00\x09".to_vec();
    v.extend_from_slice(&[0, 0, 0, 0]);
    v
}

/// FLV のタグ1つ（tag_type: 8=音, 9=映像, 18=スクリプト）と、続く PreviousTagSize
pub fn flv_tag(tag_type: u8, timestamp: u32, data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let mut v = Vec::with_capacity(data.len() + 15);
    v.push(tag_type);
    v.extend_from_slice(&len.to_be_bytes()[1..]);
    v.extend_from_slice(&timestamp.to_be_bytes()[1..]);
    v.push((timestamp >> 24) as u8);
    v.extend_from_slice(&[0, 0, 0]);
    v.extend_from_slice(data);
    v.extend_from_slice(&(len + 11).to_be_bytes());
    v
}

/// 接続1本分の、いま送っている配信
struct Active {
    id: String,
}

/// H.264 のキーフレームか（FLV の映像タグの先頭: 上位4ビットが 1）
fn is_keyframe(data: &[u8]) -> bool {
    data.first().is_some_and(|b| b >> 4 == 1)
}

/// H.264 のシーケンスヘッダーか（AVC で AVCPacketType = 0）
fn is_seq_header(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] & 0x0F == 7 && data[1] == 0
}

/// 受け手がいて道がなければ、道を作って受け手に渡す（FLV ヘッダーとシーケンスヘッダーを先に入れる）
fn attach(inner: &Inner, id: &str, p: &mut Pub) {
    if p.tx.is_some() {
        return;
    }
    let subs = inner.subs.lock().unwrap();
    let Some((_, sub)) = subs.get(id) else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(512);
    let _ = tx.send(flv_header());
    if let Some(h) = &p.seq_header {
        let _ = tx.send(h.clone());
    }
    if sub.send(Session { flv: rx }).is_ok() {
        p.tx = Some(tx);
    }
}

fn send_all(
    s: &mut TcpStream,
    results: Vec<ServerSessionResult>,
) -> std::io::Result<Vec<ServerSessionEvent>> {
    let mut events = Vec::new();
    for r in results {
        match r {
            ServerSessionResult::OutboundResponse(p) => s.write_all(&p.bytes)?,
            ServerSessionResult::RaisedEvent(e) => events.push(e),
            ServerSessionResult::UnhandleableMessageReceived(_) => {}
        }
    }
    Ok(events)
}

fn handle_conn(mut s: TcpStream, addr: SocketAddr, inner: Weak<Inner>) -> std::io::Result<()> {
    let _ = s.set_nodelay(true);
    let _ = s.set_nonblocking(false);
    // 10 秒何も来なければ切る（カメラが黙って消えたとき）
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buf = vec![0u8; 64 * 1024];

    // ハンドシェイク
    let mut hs = Handshake::new(PeerType::Server);
    let rest = loop {
        let n = s.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        match hs
            .process_bytes(&buf[..n])
            .map_err(|e| std::io::Error::other(format!("{e:?}")))?
        {
            HandshakeProcessResult::InProgress { response_bytes } => {
                s.write_all(&response_bytes)?
            }
            HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            } => {
                s.write_all(&response_bytes)?;
                break remaining_bytes;
            }
        }
    };

    let (mut session, first) = ServerSession::new(ServerSessionConfig::new())
        .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
    let mut pending = send_all(&mut s, first)?;
    let mut active: Option<Active> = None;
    let mut input = rest;

    let result = (|| -> std::io::Result<()> {
        loop {
            if !input.is_empty() {
                let r = session
                    .handle_input(&input)
                    .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
                pending.extend(send_all(&mut s, r)?);
                input.clear();
            }
            for ev in std::mem::take(&mut pending) {
                let Some(inner) = inner.upgrade() else {
                    return Ok(());
                };
                match ev {
                    ServerSessionEvent::ConnectionRequested { request_id, .. } => {
                        let r = session
                            .accept_request(request_id)
                            .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
                        pending.extend(send_all(&mut s, r)?);
                    }
                    ServerSessionEvent::PublishStreamRequested {
                        request_id,
                        app_name,
                        stream_key,
                        ..
                    } => {
                        let id = stream_id(&app_name, &stream_key);
                        let busy = inner.pubs.lock().unwrap().contains_key(&id);
                        if busy || active.is_some() || stream_key.is_empty() {
                            let r = session
                                .reject_request(
                                    request_id,
                                    "NetStream.Publish.BadName",
                                    "このストリームキーには別のカメラが送っています",
                                )
                                .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
                            send_all(&mut s, r)?;
                            return Ok(());
                        }
                        let r = session
                            .accept_request(request_id)
                            .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
                        pending.extend(send_all(&mut s, r)?);
                        let now = Instant::now();
                        let registered = inner.subs.lock().unwrap().contains_key(&id);
                        inner.pubs.lock().unwrap().insert(
                            id.clone(),
                            Pub {
                                info: PublisherInfo {
                                    app: app_name,
                                    key: stream_key,
                                    remote: addr,
                                    registered,
                                    since: now,
                                    width: None,
                                    height: None,
                                    fps: None,
                                    encoder: None,
                                    kbps: 0,
                                    video_bytes: 0,
                                },
                                window_start: now,
                                window_bytes: 0,
                                tx: None,
                                seq_header: None,
                            },
                        );
                        active = Some(Active { id });
                    }
                    ServerSessionEvent::StreamMetadataChanged { metadata, .. } => {
                        if let Some(a) = &active {
                            if let Some(p) = inner.pubs.lock().unwrap().get_mut(&a.id) {
                                p.info.width = metadata.video_width.or(p.info.width);
                                p.info.height = metadata.video_height.or(p.info.height);
                                p.info.fps = metadata.video_frame_rate.or(p.info.fps);
                                p.info.encoder = metadata.encoder.clone().or(p.info.encoder.take());
                            }
                        }
                    }
                    ServerSessionEvent::VideoDataReceived {
                        data, timestamp, ..
                    } => {
                        if let Some(a) = &active {
                            let tag = flv_tag(9, timestamp.value, &data);
                            let tx = {
                                let mut pubs = inner.pubs.lock().unwrap();
                                let Some(p) = pubs.get_mut(&a.id) else {
                                    continue;
                                };
                                p.info.video_bytes += data.len() as u64;
                                p.window_bytes += data.len() as u64;
                                let seq = is_seq_header(&data);
                                if seq {
                                    p.seq_header = Some(tag.clone());
                                }
                                // 受け手を付けるのはキーフレーム（かシーケンスヘッダー）から
                                if p.tx.is_none() && (is_keyframe(&data) || seq) {
                                    attach(&inner, &a.id, p);
                                    if seq && p.tx.is_some() {
                                        continue; // シーケンスヘッダーは attach で送り済み
                                    }
                                }
                                p.tx.clone()
                            };
                            // 受け手が遅いときは待つ（TCP で相手も待つ）。受け手が止まったら道を切る（次のキーフレームで付け直す）
                            if let Some(tx) = tx {
                                if tx.send(tag).is_err() {
                                    if let Some(p) = inner.pubs.lock().unwrap().get_mut(&a.id) {
                                        p.tx = None;
                                    }
                                }
                            }
                        }
                    }
                    ServerSessionEvent::PublishStreamFinished { .. } => {
                        if let Some(a) = active.take() {
                            inner.pubs.lock().unwrap().remove(&a.id);
                        }
                    }
                    _ => {}
                }
            }
            let n = match s.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) => return Err(e),
            };
            input.extend_from_slice(&buf[..n]);
        }
    })();
    if let (Some(a), Some(inner)) = (active.take(), inner.upgrade()) {
        inner.pubs.lock().unwrap().remove(&a.id);
    }
    result
}

/// このPCの LAN の IP（カメラに案内する用）。経路表から、外へ出るときに使う IP を調べる（実際には何も送らない）
pub fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// `rtmp://host:port/app/key` を分ける（host は何でもよい）。port を省いたら 1935
pub fn parse_listen_url(url: &str) -> Option<(u16, String, String)> {
    let rest = url.strip_prefix("rtmp://")?;
    let (hostport, path) = rest.split_once('/')?;
    let port = match hostport.rsplit_once(':') {
        Some((_, p)) => p.parse().ok()?,
        None => 1935,
    };
    let (app, key) = path.trim_end_matches('/').split_once('/')?;
    if app.is_empty() || key.is_empty() {
        return None;
    }
    Some((port, app.to_string(), key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            parse_listen_url("rtmp://0.0.0.0:1935/live/seatA"),
            Some((1935, "live".into(), "seatA".into()))
        );
        assert_eq!(
            parse_listen_url("rtmp://pc/live/cam1"),
            Some((1935, "live".into(), "cam1".into()))
        );
        assert_eq!(
            parse_listen_url("rtmp://pc:1940/app/k/"),
            Some((1940, "app".into(), "k".into()))
        );
        assert_eq!(parse_listen_url("rtmp://pc:1935/live"), None);
        assert_eq!(parse_listen_url("http://pc/live/a"), None);
    }

    #[test]
    fn flv_tags() {
        let t = flv_tag(9, 0x0102_0304, &[0xAA, 0xBB]);
        assert_eq!(t[0], 9);
        assert_eq!(&t[1..4], &[0, 0, 2]);
        assert_eq!(&t[4..8], &[0x02, 0x03, 0x04, 0x01]);
        assert_eq!(&t[11..13], &[0xAA, 0xBB]);
        assert_eq!(&t[13..], &13u32.to_be_bytes());
        assert_eq!(flv_header().len(), 13);
    }

    #[test]
    fn busy_port_is_reported() {
        let hold = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = hold.local_addr().unwrap().port();
        let s = RtmpServer::shared(port);
        let st = s.status();
        assert!(st.bind_error.unwrap().contains(&port.to_string()));
    }
}

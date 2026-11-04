//! クライアント側の接続（設計書 3.2）。描画（Bevy）とは別スレッドで動き、チャンネルで受け渡す。
//!
//! - 切れたら自動で再接続する（0.5秒から最大5秒まで間隔を延ばす）
//! - 連番の抜けに気づいたらスナップショットを取り直す
//! - 定期的に Ping を送り、サーバー時刻との差（配信ディレイの基準）を測る
//! - 接続コード違い・版違いで拒否されたら再接続しない（設定を直してもらう）

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use janscope_core::engine::Overlay;
use janscope_core::protocol::{ClientMsg, ClientRole, PROTOCOL_VERSION, ServerMsg, now_ms};
use janscope_core::scoring::WinScore;
use janscope_core::view::TableStateView;
use tokio::sync::mpsc as tmpsc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Debug)]
pub struct ConnectConfig {
    /// "192.168.1.10:47800" のような接続先
    pub addr: String,
    pub code: String,
    pub role: ClientRole,
    pub client_version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    Connecting {
        attempt: u32,
    },
    Connected {
        server_version: String,
    },
    Disconnected {
        reason: String,
    },
    /// 拒否された（再接続しない）
    Rejected {
        reason: String,
    },
    /// 状態の更新（スナップショットのときは overlays が空、snapshot = true）
    State {
        seq: u64,
        at: i64,
        view: Box<TableStateView>,
        overlays: Vec<Overlay>,
        snapshot: bool,
        by: Option<ClientRole>,
    },
    ActionResult {
        request_id: u64,
        error: Option<String>,
    },
    Preview {
        request_id: u64,
        result: Result<Box<WinScore>, String>,
    },
    Notice {
        message: String,
    },
    /// サーバー時刻 − クライアント時刻（ms）と往復時間（ms）
    Clock {
        offset_ms: i64,
        rtt_ms: i64,
    },
}

/// サーバーとの接続。drop すると止まる。
pub struct NetClient {
    out: tmpsc::UnboundedSender<ClientMsg>,
    events: mpsc::Receiver<NetEvent>,
    stop: Arc<AtomicBool>,
    next_id: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NetClient {
    pub fn connect(cfg: ConnectConfig) -> Self {
        let (out_tx, out_rx) = tmpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("janscope-net".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio");
                rt.block_on(run(cfg, out_rx, ev_tx, stop2));
            })
            .expect("thread");
        NetClient {
            out: out_tx,
            events: ev_rx,
            stop,
            next_id: 1,
            thread: Some(thread),
        }
    }

    /// 届いた出来事をすべて取り出す（描画のフレームごとに呼ぶ）。
    pub fn poll(&self) -> Vec<NetEvent> {
        self.events.try_iter().collect()
    }

    /// 出来事を待つ（テスト用）。
    pub fn recv_timeout(&self, d: Duration) -> Option<NetEvent> {
        self.events.recv_timeout(d).ok()
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 操作を送る。戻り値の request_id で ActionResult と対応づける。
    pub fn send_action(&mut self, action: janscope_core::state::Action) -> u64 {
        let request_id = self.id();
        let _ = self.out.send(ClientMsg::Action { request_id, action });
        request_id
    }
    pub fn undo(&mut self) -> u64 {
        let request_id = self.id();
        let _ = self.out.send(ClientMsg::Undo { request_id });
        request_id
    }
    pub fn redo(&mut self) -> u64 {
        let request_id = self.id();
        let _ = self.out.send(ClientMsg::Redo { request_id });
        request_id
    }
    pub fn preview(&mut self, input: janscope_core::scoring::WinInput) -> u64 {
        let request_id = self.id();
        let _ = self
            .out
            .send(ClientMsg::PreviewRequest { request_id, input });
        request_id
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 送信側を閉じるとループが抜ける
        let (dummy, _) = tmpsc::unbounded_channel();
        self.out = dummy;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

enum End {
    Retry(String),
    Rejected(String),
    Stopped,
}

async fn run(
    cfg: ConnectConfig,
    mut out: tmpsc::UnboundedReceiver<ClientMsg>,
    ev: mpsc::Sender<NetEvent>,
    stop: Arc<AtomicBool>,
) {
    let mut attempt = 0u32;
    let mut delay = Duration::from_millis(500);
    while !stop.load(Ordering::SeqCst) {
        attempt += 1;
        let _ = ev.send(NetEvent::Connecting { attempt });
        match session(&cfg, &mut out, &ev, &stop).await {
            (End::Stopped, _) => return,
            (End::Rejected(reason), _) => {
                let _ = ev.send(NetEvent::Rejected { reason });
                return;
            }
            (End::Retry(reason), was_connected) => {
                let _ = ev.send(NetEvent::Disconnected { reason });
                if was_connected {
                    delay = Duration::from_millis(500);
                }
            }
        }
        // 切断中に溜まった操作は送らない（古い操作を後から適用しないため）
        while let Ok(m) = out.try_recv() {
            if let ClientMsg::Action { request_id, .. }
            | ClientMsg::Undo { request_id }
            | ClientMsg::Redo { request_id } = m
            {
                let _ = ev.send(NetEvent::ActionResult {
                    request_id,
                    error: Some("サーバーに接続していません".into()),
                });
            }
        }
        let until = tokio::time::Instant::now() + delay;
        while tokio::time::Instant::now() < until {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

async fn session(
    cfg: &ConnectConfig,
    out: &mut tmpsc::UnboundedReceiver<ClientMsg>,
    ev: &mpsc::Sender<NetEvent>,
    stop: &AtomicBool,
) -> (End, bool) {
    let url = if cfg.addr.starts_with("ws://") {
        cfg.addr.clone()
    } else {
        format!("ws://{}", cfg.addr)
    };
    let conn = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(&url),
    )
    .await;
    let mut ws = match conn {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return (End::Retry(format!("接続できません: {e}")), false),
        Err(_) => return (End::Retry("接続がタイムアウトしました".into()), false),
    };
    let join = ClientMsg::Join {
        role: cfg.role,
        code: cfg.code.clone(),
        client_version: cfg.client_version.clone(),
        protocol: PROTOCOL_VERSION,
    };
    if let Err(e) = ws.send(Message::text(join.to_json())).await {
        return (End::Retry(e.to_string()), false);
    }
    let mut connected = false;
    let mut last_seq: Option<u64> = None;
    let mut ping = tokio::time::interval(Duration::from_secs(2));
    let mut check_stop = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = check_stop.tick() => {
                if stop.load(Ordering::SeqCst) {
                    let _ = ws.close(None).await;
                    return (End::Stopped, connected);
                }
            }
            _ = ping.tick() => {
                let m = ClientMsg::Ping { client_time: now_ms() };
                if let Err(e) = ws.send(Message::text(m.to_json())).await {
                    return (End::Retry(e.to_string()), connected);
                }
            }
            m = out.recv() => {
                let Some(m) = m else { return (End::Stopped, connected) };
                if let Err(e) = ws.send(Message::text(m.to_json())).await {
                    return (End::Retry(e.to_string()), connected);
                }
            }
            incoming = ws.next() => {
                let text = match incoming {
                    None => return (End::Retry("サーバーが切断しました".into()), connected),
                    Some(Err(e)) => return (End::Retry(e.to_string()), connected),
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) => return (End::Retry("サーバーが切断しました".into()), connected),
                    Some(Ok(_)) => continue,
                };
                let msg: ServerMsg = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = ev.send(NetEvent::Notice { message: format!("読めないメッセージ: {e}") });
                        continue;
                    }
                };
                match msg {
                    ServerMsg::Hello { server_version, .. } => {
                        connected = true;
                        let _ = ev.send(NetEvent::Connected { server_version });
                    }
                    ServerMsg::Rejected { reason } => return (End::Rejected(reason), connected),
                    ServerMsg::Snapshot { seq, server_time, view } => {
                        last_seq = Some(seq);
                        let _ = ev.send(NetEvent::State { seq, at: server_time, view: Box::new(view), overlays: vec![], snapshot: true, by: None });
                    }
                    ServerMsg::Event { seq, at, view, overlays, by } => {
                        if let Some(prev) = last_seq {
                            if seq <= prev {
                                continue;
                            }
                            if seq != prev + 1 {
                                let _ = ws.send(Message::text(ClientMsg::RequestSnapshot.to_json())).await;
                            }
                        }
                        last_seq = Some(seq);
                        let _ = ev.send(NetEvent::State { seq, at, view: Box::new(view), overlays, snapshot: false, by });
                    }
                    ServerMsg::ActionResult { request_id, error } => {
                        let _ = ev.send(NetEvent::ActionResult { request_id, error });
                    }
                    ServerMsg::PreviewResult { request_id, result } => {
                        let _ = ev.send(NetEvent::Preview { request_id, result: result.map(Box::new) });
                    }
                    ServerMsg::Pong { client_time, server_time } => {
                        let now = now_ms();
                        let rtt = now - client_time;
                        let _ = ev.send(NetEvent::Clock { offset_ms: server_time - (client_time + now) / 2, rtt_ms: rtt });
                    }
                    ServerMsg::Notice { message } => {
                        let _ = ev.send(NetEvent::Notice { message });
                    }
                }
            }
        }
    }
}

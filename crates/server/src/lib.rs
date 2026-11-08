//! JanScope 処理サーバー（設計書 3.1・3.2）。
//!
//! 正となる卓状態を持ち、WebSocket でクライアント（配信・入力担当・選手用画面）に配る。
//! - 接続コードが合わないクライアントは受け付けない
//! - ロールごとに送る情報を絞る（選手用画面には公開情報の型 `PublicView` だけ）
//! - 操作はログに追記し、落ちても続きから再開できる
//!
//! カメラ映像の取り込み・牌認識・管理画面は段階Cで追加する。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use janscope_core::engine::{Overlay, preview_win};
use janscope_core::log::GameLog;
use janscope_core::protocol::{ClientMsg, ClientRole, PROTOCOL_VERSION, ServerMsg, now_ms};
use janscope_core::view::{FullView, PublicView, TableStateView};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast};
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    /// 接続コード（クライアントが入力する）
    pub code: String,
    /// 操作ログの保存先フォルダ
    pub data_dir: PathBuf,
    /// 再開するログファイル（None なら新しいログを作る）
    pub resume: Option<PathBuf>,
    /// 機能の確認用: ダミー対局データ（core::dummy）を入れた状態で始める
    pub dummy: bool,
}

/// 6桁の接続コードを作る。
pub fn new_code() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

/// 最新のログファイル（続きから再開の候補）。
pub fn latest_log(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("logs");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    v.sort();
    v.pop()
}

/// 状態の変更通知（全クライアントに配る。ロールごとの絞り込みは送信直前に行う）。
#[derive(Clone, Debug)]
struct Update {
    seq: u64,
    at: i64,
    full: Arc<FullView>,
    overlays: Vec<Overlay>,
    by: Option<ClientRole>,
}

struct Shared {
    log: GameLog,
    seq: u64,
    tournament: janscope_core::tournament::Tournament,
    tournament_path: PathBuf,
}

pub struct Server {
    shared: Arc<Mutex<Shared>>,
    tx: broadcast::Sender<Update>,
    code: String,
    shutdown: tokio::sync::watch::Receiver<bool>,
}

pub struct Running {
    pub addr: SocketAddr,
    pub log_path: Option<PathBuf>,
    pub code: String,
    handle: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl Running {
    /// サーバーを止める（待ち受けと、接続中のすべてのクライアントを切る）。
    pub fn stop(self) {
        let _ = self.shutdown.send(true);
        self.handle.abort();
    }
}

fn full_view(sh: &Shared) -> FullView {
    FullView {
        state: sh.log.state().clone(),
        can_undo: sh.log.can_undo(),
        can_redo: sh.log.can_redo(),
        tournament: sh.tournament.clone(),
    }
}

impl Shared {
    /// 今の半荘のかぎ（操作ログのファイル名＋何回目の半荘開始か）
    fn game_key(&self) -> String {
        let file = self
            .log
            .path()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let n = self
            .log
            .actions()
            .iter()
            .filter(|a| matches!(a.action, janscope_core::state::Action::StartGame { .. }))
            .count();
        format!("{file}#{n}")
    }

    /// 終局していれば大会通算に記録する（終局を取り消したら記録も消す）。
    fn sync_tournament(&mut self) {
        let key = self.game_key();
        let state = self.log.state().clone();
        if self.tournament.sync_game(&key, &state, now_ms()) {
            self.save_tournament();
        }
    }

    fn save_tournament(&self) {
        if let Some(dir) = self.tournament_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&self.tournament) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&self.tournament_path, s) {
                    warn!("大会通算を保存できません: {e}");
                }
            }
            Err(e) => warn!("大会通算を保存できません: {e}"),
        }
    }
}

fn view_for(role: ClientRole, full: &FullView) -> TableStateView {
    match role {
        ClientRole::PlayerScreen => {
            TableStateView::Public(Box::new(PublicView::from_state(&full.state)))
        }
        _ => TableStateView::Full(Box::new(full.clone())),
    }
}

/// サーバーを起動する（待ち受けを始めたら戻る）。
pub async fn start(cfg: ServerConfig) -> std::io::Result<Running> {
    let log = match &cfg.resume {
        Some(p) => GameLog::open(p).map_err(std::io::Error::other)?,
        None => {
            let name = format!("{}.jsonl", chrono_like_stamp());
            GameLog::create(&cfg.data_dir.join("logs").join(name)).map_err(std::io::Error::other)?
        }
    };
    let mut log = log;
    if cfg.dummy && cfg.resume.is_none() {
        let at = now_ms();
        for a in janscope_core::dummy::actions() {
            log.act(a, at).map_err(std::io::Error::other)?;
        }
    }
    let log_path = log.path().map(Path::to_path_buf);
    let listener = TcpListener::bind(cfg.bind).await?;
    let addr = listener.local_addr()?;
    let (tx, _) = broadcast::channel(256);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server = Arc::new(Server {
        shared: Arc::new(Mutex::new({
            let tournament_path = cfg.data_dir.join("tournament.json");
            let tournament = std::fs::read_to_string(&tournament_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let mut sh = Shared {
                log,
                seq: 0,
                tournament,
                tournament_path,
            };
            sh.sync_tournament();
            sh
        })),
        tx,
        code: cfg.code.clone(),
        shutdown: shutdown_rx,
    });
    info!("待ち受け: {addr}  接続コード: {}", cfg.code);
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let s = server.clone();
                    tokio::spawn(async move {
                        if let Err(e) = s.handle(stream, peer).await {
                            info!("{peer}: 切断 ({e})");
                        }
                    });
                }
                Err(e) => warn!("accept: {e}"),
            }
        }
    });
    Ok(Running {
        addr,
        log_path,
        code: cfg.code,
        handle,
        shutdown: shutdown_tx,
    })
}

/// ファイル名用の時刻（YYYYMMDD-HHMMSS、UTC）。
fn chrono_like_stamp() -> String {
    let secs = now_ms() / 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // 1970-01-01 からの日数 → 年月日（Howard Hinnant の civil_from_days）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        tod / 3600,
        tod % 3600 / 60,
        tod % 60
    )
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

async fn send(ws: &mut Ws, msg: &ServerMsg) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    ws.send(Message::text(msg.to_json())).await
}

impl Server {
    async fn handle(
        self: Arc<Self>,
        stream: TcpStream,
        peer: SocketAddr,
    ) -> anyhow_like::Result<()> {
        stream.set_nodelay(true).ok();
        let mut ws = tokio_tungstenite::accept_async(stream).await?;
        send(
            &mut ws,
            &ServerMsg::Hello {
                server_version: VERSION.into(),
                protocol: PROTOCOL_VERSION,
                server_time: now_ms(),
            },
        )
        .await?;
        // 最初のメッセージは Join（10秒以内）
        let first = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
        let join = match first {
            Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<ClientMsg>(&t).ok(),
            _ => None,
        };
        let Some(ClientMsg::Join {
            role,
            code,
            client_version,
            protocol,
        }) = join
        else {
            send(
                &mut ws,
                &ServerMsg::Rejected {
                    reason: "最初に Join を送ってください".into(),
                },
            )
            .await
            .ok();
            return Ok(());
        };
        if code.trim() != self.code {
            warn!("{peer}: 接続コードが違います");
            send(
                &mut ws,
                &ServerMsg::Rejected {
                    reason: "接続コードが違います".into(),
                },
            )
            .await
            .ok();
            ws.close(None).await.ok();
            return Ok(());
        }
        if protocol != PROTOCOL_VERSION {
            send(&mut ws, &ServerMsg::Rejected {
                reason: format!("通信の版が違います（サーバー {PROTOCOL_VERSION} / クライアント {protocol}）。同じ版のソフトを使ってください"),
            })
            .await
            .ok();
            return Ok(());
        }
        if client_version != VERSION {
            send(&mut ws, &ServerMsg::Notice {
                message: format!("サーバー（{VERSION}）とクライアント（{client_version}）のバージョンが違います"),
            })
            .await?;
        }
        info!(
            "{peer}: {} として接続 (client {client_version})",
            role.name_ja()
        );

        let mut rx = self.tx.subscribe();
        let mut shutdown = self.shutdown.clone();
        self.send_snapshot(&mut ws, role).await?;
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    ws.close(None).await.ok();
                    return Ok(());
                }
                incoming = ws.next() => {
                    let Some(msg) = incoming else { return Ok(()) };
                    match msg? {
                        Message::Text(t) => match serde_json::from_str::<ClientMsg>(&t) {
                            Ok(m) => self.on_message(&mut ws, role, m).await?,
                            Err(e) => send(&mut ws, &ServerMsg::Notice { message: format!("読めないメッセージ: {e}") }).await?,
                        },
                        Message::Close(_) => return Ok(()),
                        _ => {}
                    }
                }
                upd = rx.recv() => match upd {
                    Ok(u) => {
                        let msg = ServerMsg::Event { seq: u.seq, at: u.at, view: view_for(role, &u.full), overlays: u.overlays, by: u.by };
                        send(&mut ws, &msg).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => self.send_snapshot(&mut ws, role).await?,
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }

    async fn send_snapshot(&self, ws: &mut Ws, role: ClientRole) -> anyhow_like::Result<()> {
        let (seq, full) = {
            let sh = self.shared.lock().await;
            (sh.seq, full_view(&sh))
        };
        send(
            ws,
            &ServerMsg::Snapshot {
                seq,
                server_time: now_ms(),
                view: view_for(role, &full),
            },
        )
        .await?;
        Ok(())
    }

    async fn on_message(
        &self,
        ws: &mut Ws,
        role: ClientRole,
        m: ClientMsg,
    ) -> anyhow_like::Result<()> {
        match m {
            ClientMsg::Join { .. } => {}
            ClientMsg::Ping { client_time } => {
                send(
                    ws,
                    &ServerMsg::Pong {
                        client_time,
                        server_time: now_ms(),
                    },
                )
                .await?;
            }
            ClientMsg::RequestSnapshot => self.send_snapshot(ws, role).await?,
            ClientMsg::PreviewRequest { request_id, input } => {
                if !role.can_operate() {
                    return Ok(());
                }
                let result = {
                    let sh = self.shared.lock().await;
                    preview_win(sh.log.state(), &input).map_err(|e| e.to_string())
                };
                send(ws, &ServerMsg::PreviewResult { request_id, result }).await?;
            }
            ClientMsg::Action { request_id, .. }
            | ClientMsg::Undo { request_id }
            | ClientMsg::Redo { request_id }
                if !role.can_operate() =>
            {
                send(
                    ws,
                    &ServerMsg::ActionResult {
                        request_id,
                        error: Some("このロールでは操作できません".into()),
                    },
                )
                .await?;
            }
            ClientMsg::Action { request_id, action } => {
                let r = self.mutate(role, |log, at| log.act(action, at)).await;
                send(
                    ws,
                    &ServerMsg::ActionResult {
                        request_id,
                        error: r.err(),
                    },
                )
                .await?;
            }
            ClientMsg::Undo { request_id } => {
                let r = self
                    .mutate(role, |log, at| log.undo(at).map(|_| vec![]))
                    .await;
                send(
                    ws,
                    &ServerMsg::ActionResult {
                        request_id,
                        error: r.err(),
                    },
                )
                .await?;
            }
            ClientMsg::Tournament { request_id, op } => {
                let r = if !role.can_operate() {
                    Err("このロールでは操作できません".to_string())
                } else {
                    self.tournament_op(role, op).await
                };
                send(
                    ws,
                    &ServerMsg::ActionResult {
                        request_id,
                        error: r.err(),
                    },
                )
                .await?;
            }
            ClientMsg::Redo { request_id } => {
                let r = self.mutate(role, |log, at| log.redo(at)).await;
                send(
                    ws,
                    &ServerMsg::ActionResult {
                        request_id,
                        error: r.err(),
                    },
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn tournament_op(
        &self,
        role: ClientRole,
        op: janscope_core::protocol::TournamentOp,
    ) -> Result<(), String> {
        use janscope_core::protocol::TournamentOp;
        let at = now_ms();
        let mut sh = self.shared.lock().await;
        match op {
            TournamentOp::Rename { name } => sh.tournament.name = name,
            TournamentOp::RemoveGame { key } => {
                if !sh.tournament.remove(&key) {
                    return Err("その半荘の記録はありません".into());
                }
            }
        }
        sh.save_tournament();
        sh.seq += 1;
        let upd = Update {
            seq: sh.seq,
            at,
            full: Arc::new(full_view(&sh)),
            overlays: vec![],
            by: Some(role),
        };
        let _ = self.tx.send(upd);
        Ok(())
    }

    /// ログを変更し、成功したら全クライアントに通知する。
    async fn mutate(
        &self,
        role: ClientRole,
        f: impl FnOnce(&mut GameLog, i64) -> Result<Vec<Overlay>, janscope_core::log::LogError>,
    ) -> Result<(), String> {
        let at = now_ms();
        let mut sh = self.shared.lock().await;
        let overlays = f(&mut sh.log, at).map_err(|e| e.to_string())?;
        sh.sync_tournament();
        sh.seq += 1;
        let upd = Update {
            seq: sh.seq,
            at,
            full: Arc::new(full_view(&sh)),
            overlays,
            by: Some(role),
        };
        // 受け手がいなくてもよい
        let _ = self.tx.send(upd);
        Ok(())
    }
}

/// 小さなエラー型（依存を増やさないため）。
mod anyhow_like {
    pub type Result<T> = std::result::Result<T, Error>;
    #[derive(Debug)]
    pub struct Error(pub String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl From<tokio_tungstenite::tungstenite::Error> for Error {
        fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
            Error(e.to_string())
        }
    }
}

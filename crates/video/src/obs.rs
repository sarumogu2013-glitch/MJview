//! OBS との連携（obs-websocket v5、OBS 28 以降に標準で入っている操作機能）。設計書 4.11 の代替モード:
//! 顔カメラを本ソフトで取り込めないとき、選手画面を黒枠にして OBS 側で顔映像を重ね、
//! 自動／手動切替の結果を OBS のシーン切替、またはソースの表示・非表示で反映する。
//!
//! 別スレッドで接続し、切れたら自動で再接続する。`show_seat` を呼ぶと最新の席だけを送る。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ObsSwitchMode {
    /// 席ごとのシーンに切り替える
    #[default]
    Scene,
    /// 1つのシーンの中で、席ごとのソースの表示・非表示を切り替える
    SourceVisibility,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObsConfig {
    pub enabled: bool,
    /// 例: ws://127.0.0.1:4455
    pub url: String,
    pub password: String,
    pub mode: ObsSwitchMode,
    /// 席A〜Dのシーン名（Scene モード）
    pub scenes: [String; 4],
    /// ソースを置いたシーン名（SourceVisibility モード）
    pub source_scene: String,
    /// 席A〜Dのソース名（SourceVisibility モード）
    pub sources: [String; 4],
}

impl Default for ObsConfig {
    fn default() -> Self {
        ObsConfig {
            enabled: false,
            url: "ws://127.0.0.1:4455".into(),
            password: String::new(),
            mode: ObsSwitchMode::Scene,
            scenes: ["顔A", "顔B", "顔C", "顔D"].map(String::from),
            source_scene: "対局".into(),
            sources: ["顔カメラA", "顔カメラB", "顔カメラC", "顔カメラD"].map(String::from),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObsStatus {
    Disabled,
    Connecting,
    Connected { obs_version: String },
    Error(String),
}

struct Shared {
    status: ObsStatus,
    want: Option<u8>,
    sent: Option<u8>,
    last_error: Option<String>,
}

pub struct ObsLink {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    wake: Arc<tokio::sync::Notify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// obs-websocket v5 の認証文字列: base64(sha256(base64(sha256(password + salt)) + challenge))
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

impl ObsLink {
    pub fn start(cfg: ObsConfig) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: if cfg.enabled {
                ObsStatus::Connecting
            } else {
                ObsStatus::Disabled
            },
            want: None,
            sent: None,
            last_error: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(tokio::sync::Notify::new());
        let thread = if cfg.enabled {
            let (s, st, w) = (shared.clone(), stop.clone(), wake.clone());
            Some(
                std::thread::Builder::new()
                    .name("janscope-obs".into())
                    .spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("tokio");
                        rt.block_on(run(cfg, s, st, w));
                    })
                    .expect("thread"),
            )
        } else {
            None
        };
        ObsLink {
            shared,
            stop,
            wake,
            thread,
        }
    }

    /// 映す席（0〜3）を OBS に反映する。同じ席なら送らない。
    pub fn show_seat(&self, seat: u8) {
        let mut sh = self.shared.lock().unwrap();
        if sh.want != Some(seat) {
            sh.want = Some(seat);
            drop(sh);
            self.wake.notify_one();
        }
    }

    pub fn status(&self) -> ObsStatus {
        self.shared.lock().unwrap().status.clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.lock().unwrap().last_error.clone()
    }

    /// 最後に OBS が受け付けた席（テスト・表示用）。
    pub fn applied_seat(&self) -> Option<u8> {
        self.shared.lock().unwrap().sent
    }
}

impl Drop for ObsLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

async fn run(
    cfg: ObsConfig,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    wake: Arc<tokio::sync::Notify>,
) {
    let mut delay = Duration::from_millis(500);
    while !stop.load(Ordering::SeqCst) {
        shared.lock().unwrap().status = ObsStatus::Connecting;
        match session(&cfg, &shared, &stop, &wake).await {
            Ok(()) => return,
            Err(e) => {
                let mut sh = shared.lock().unwrap();
                sh.status = ObsStatus::Error(e.clone());
                sh.last_error = Some(e);
                sh.sent = None;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wake.notified() => {}
        }
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn recv_op(ws: &mut Ws, op: u64) -> Result<Value, String> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .map_err(|_| "OBS からの応答がありません".to_string())?
            .ok_or("OBS が切断しました")?
            .map_err(|e| e.to_string())?;
        let Message::Text(t) = m else { continue };
        let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
        if v["op"].as_u64() == Some(op) {
            return Ok(v["d"].clone());
        }
    }
}

async fn request(ws: &mut Ws, id: &mut u64, ty: &str, data: Value) -> Result<Value, String> {
    *id += 1;
    let rid = id.to_string();
    let msg = json!({"op": 6, "d": {"requestType": ty, "requestId": rid, "requestData": data}});
    ws.send(Message::text(msg.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    loop {
        let d = recv_op(ws, 7).await?;
        if d["requestId"].as_str() != Some(rid.as_str()) {
            continue;
        }
        let st = &d["requestStatus"];
        if st["result"].as_bool() == Some(true) {
            return Ok(d["responseData"].clone());
        }
        return Err(format!(
            "OBS が {ty} を受け付けませんでした（{}）: {}",
            st["code"],
            st["comment"].as_str().unwrap_or("")
        ));
    }
}

async fn session(
    cfg: &ObsConfig,
    shared: &Mutex<Shared>,
    stop: &AtomicBool,
    wake: &tokio::sync::Notify,
) -> Result<(), String> {
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(&cfg.url),
    )
    .await
    .map_err(|_| format!("OBS（{}）に接続できません: タイムアウト", cfg.url))?
    .map_err(|e| format!("OBS（{}）に接続できません: {e}", cfg.url))?;
    let hello = recv_op(&mut ws, 0).await?;
    let mut identify = json!({"rpcVersion": 1, "eventSubscriptions": 0});
    if let Some(a) = hello.get("authentication") {
        let (Some(ch), Some(salt)) = (a["challenge"].as_str(), a["salt"].as_str()) else {
            return Err("OBS の認証情報が読めません".into());
        };
        identify["authentication"] = json!(auth_string(&cfg.password, salt, ch));
    }
    ws.send(Message::text(json!({"op": 1, "d": identify}).to_string()))
        .await
        .map_err(|e| e.to_string())?;
    match recv_op(&mut ws, 2).await {
        Ok(_) => {}
        Err(_) => return Err("OBS の認証に失敗しました（パスワードを確認してください）".into()),
    }
    let ver = hello["obsWebSocketVersion"]
        .as_str()
        .unwrap_or("?")
        .to_string();
    {
        let mut sh = shared.lock().unwrap();
        sh.status = ObsStatus::Connected { obs_version: ver };
        sh.sent = None;
    }
    let mut id = 0u64;
    // SourceVisibility モードで使う sceneItemId
    let mut item_ids: [Option<i64>; 4] = [None; 4];
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = ws.close(None).await;
            return Ok(());
        }
        let (want, sent) = {
            let sh = shared.lock().unwrap();
            (sh.want, sh.sent)
        };
        if let Some(seat) = want.filter(|w| Some(*w) != sent) {
            let i = seat as usize;
            match cfg.mode {
                ObsSwitchMode::Scene => {
                    request(
                        &mut ws,
                        &mut id,
                        "SetCurrentProgramScene",
                        json!({"sceneName": cfg.scenes[i]}),
                    )
                    .await?;
                }
                ObsSwitchMode::SourceVisibility => {
                    for k in 0..4 {
                        if item_ids[k].is_none() {
                            let r = request(
                                &mut ws,
                                &mut id,
                                "GetSceneItemId",
                                json!({"sceneName": cfg.source_scene, "sourceName": cfg.sources[k]}),
                            )
                            .await?;
                            item_ids[k] = r["sceneItemId"].as_i64();
                        }
                    }
                    for k in 0..4 {
                        request(
                            &mut ws,
                            &mut id,
                            "SetSceneItemEnabled",
                            json!({"sceneName": cfg.source_scene, "sceneItemId": item_ids[k], "sceneItemEnabled": k == i}),
                        )
                        .await?;
                    }
                }
            }
            shared.lock().unwrap().sent = Some(seat);
            continue;
        }
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            m = ws.next() => match m {
                None | Some(Err(_)) => return Err("OBS との接続が切れました".into()),
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_matches_reference() {
        // obs-websocket v5 のプロトコル文書の例（Python の hashlib/base64 でも同じ値になることを確認済み）
        let salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
        let challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";
        assert_eq!(
            auth_string("supersecretpassword", salt, challenge),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
        assert_ne!(
            auth_string("wrong", salt, challenge),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }
}

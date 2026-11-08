//! JanScope メディアサーバー（janscope-media）。
//!
//! 映像の流れ: カメラ・画像・映像 → **メディアサーバー**（受ける・枠で切り出す）→ 入力担当（映像サンドボックス・配信画面）
//!
//! - RTMP の受け口（既定 1935）を持つ。カメラから届いた映像は、ソースとして自動で並ぶ
//! - Webカメラ・画像・映像ファイル・URL もソースとして足せる（入力担当の映像サンドボックスから）
//! - ソースごとに ffmpeg を1つ起動し、確認用の縮小映像（5fps）と、枠ごとの切り出し映像を同時に作る（デコードは1回）
//! - 入力担当には HTTP で渡す（既定 http://127.0.0.1:47810）
//!   - `GET /status` 様子と設定（JSON）・`GET /config`・`PUT /config` 設定（枠・ソース）
//!   - `GET /preview/<ソース>.mjpg` 確認用の映像・`GET /snapshot/<ソース>.jpg` その1枚
//!   - `GET /region/<枠>.mjpg` 切り出した映像（face-A・hand-A・river-A・board）・`GET /region/<枠>.jpg` その1枚
//!   - `GET /devices` Webカメラの一覧
//! - 入力担当やサーバーを起動し直しても、カメラはつながったまま

pub mod bus;
pub mod http_api;
pub mod pipeline;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use janscope_video::rtmp_server::RtmpServer;
use janscope_video::sandbox::{SandboxConfig, SourceDef, SourceKind, safe_id};

pub use bus::Bus;

/// メディアサーバー全体の状態
pub struct State {
    pub config: Mutex<SandboxConfig>,
    /// 設定が変わるたびに増える（ffmpeg を起動し直す合図）
    pub version: AtomicU64,
    pub buses: Mutex<HashMap<String, Arc<Bus>>>,
    /// ソースごとの最後のエラー
    pub errors: Mutex<HashMap<String, String>>,
    pub rtmp: RtmpServer,
    pub ffmpeg: String,
    pub config_path: PathBuf,
}

impl State {
    pub fn new(rtmp_port: u16, ffmpeg: String, config_path: PathBuf) -> Arc<State> {
        let mut config: SandboxConfig = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        config.normalize();
        Arc::new(State {
            config: Mutex::new(config),
            version: AtomicU64::new(1),
            buses: Mutex::new(HashMap::new()),
            errors: Mutex::new(HashMap::new()),
            rtmp: RtmpServer::shared(rtmp_port),
            ffmpeg,
            config_path,
        })
    }

    /// 名前の映像の通り道（なければ作る）
    pub fn bus(&self, name: &str) -> Arc<Bus> {
        self.buses
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Bus::default()))
            .clone()
    }

    /// 今のソース（手で足したもの＋RTMP で届いているもの）
    pub fn sources(&self) -> Vec<SourceDef> {
        let cfg = self.config.lock().unwrap();
        let mut v = cfg.sources.clone();
        for p in self.rtmp.status().publishers {
            let known = v.iter().any(|s| {
                matches!(&s.kind, SourceKind::Rtmp { app, key } if *app == p.app && *key == p.key)
            });
            if !known {
                v.push(SourceDef {
                    id: format!("rtmp-{}", safe_id(&p.key)),
                    name: format!("RTMP {}/{}", p.app, p.key),
                    kind: SourceKind::Rtmp {
                        app: p.app.clone(),
                        key: p.key.clone(),
                    },
                });
            }
        }
        v
    }

    /// 設定を差し替えて保存する（ffmpeg は起動し直される）
    pub fn set_config(&self, mut c: SandboxConfig) -> Result<(), String> {
        c.normalize();
        for s in &mut c.sources {
            s.id = safe_id(&s.id);
        }
        if let Some(p) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let text = serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, text)
            .map_err(|e| format!("{}: {e}", self.config_path.display()))?;
        *self.config.lock().unwrap() = c;
        self.version.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// ffmpeg の場所: 実行ファイルの隣の ffmpeg/ffmpeg(.exe)、なければ PATH の ffmpeg
pub fn default_ffmpeg() -> String {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("ffmpeg").join(name);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "ffmpeg".into()
}

/// ログの時刻（時:分:秒）
pub fn now() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 日本時間で出す（大会の運用は日本）
    let t = (s + 9 * 3600) % 86400;
    format!("[{:02}:{:02}:{:02}]", t / 3600, t / 60 % 60, t % 60)
}

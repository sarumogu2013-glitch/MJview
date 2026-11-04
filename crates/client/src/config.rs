//! クライアントの設定（顔カメラ・OBS 連携・選手画面の切替）。保存形式は JSON、編集は操作ウィンドウから行う。
//! 置き場所: 環境変数 JANSCOPE_CONFIG、なければ作業フォルダの janscope-client.json。

use std::path::PathBuf;

use bevy::prelude::*;
use janscope_core::face::FaceSwitchConfig;
use janscope_video::RtmpMode;
use janscope_video::obs::ObsConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FaceOutput {
    /// 顔カメラの映像を本ソフトで取り込んで表示
    #[default]
    Video,
    /// 黒枠にして OBS 側で顔映像を重ねる
    Black,
    /// クロマキー色（緑）の枠にして OBS 側で抜く
    Chroma,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaceSource {
    pub enabled: bool,
    pub url: String,
    pub mode: RtmpMode,
}

impl Default for FaceSource {
    fn default() -> Self {
        FaceSource {
            enabled: false,
            url: String::new(),
            mode: RtmpMode::Pull,
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    pub face_output: FaceOutput,
    /// 席A〜Dの顔カメラ
    pub faces: [FaceSource; 4],
    /// ffmpeg の実行ファイル（LGPL ビルドを推奨）
    pub ffmpeg: String,
    pub face_switch: FaceSwitchConfig,
    pub obs: ObsConfig,
    /// 配信ディレイ（秒）。出力ウィンドウ全体を遅らせる
    pub delay_secs: f64,
    /// 顔カメラごとの同期補正（ms、正の値でその映像をさらに遅らせる）
    pub sync_offset_ms: [i32; 4],
}

impl Default for ClientConfig {
    fn default() -> Self {
        let faces = std::array::from_fn(|i| FaceSource {
            enabled: false,
            url: format!("rtmp://0.0.0.0:1935/live/seat{}", (b'A' + i as u8) as char),
            mode: RtmpMode::Listen,
        });
        ClientConfig {
            face_output: FaceOutput::Video,
            faces,
            ffmpeg: default_ffmpeg(),
            face_switch: FaceSwitchConfig::default(),
            obs: ObsConfig::default(),
            delay_secs: 0.0,
            sync_offset_ms: [0; 4],
        }
    }
}

/// 実行ファイルの隣の ffmpeg/ffmpeg(.exe) があればそれ、なければ PATH の ffmpeg。
fn default_ffmpeg() -> String {
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

pub fn config_path() -> PathBuf {
    std::env::var("JANSCOPE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("janscope-client.json"))
}

impl ClientConfig {
    pub fn load() -> Self {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                warn!(
                    "設定ファイル {} が読めないので既定値を使います: {e}",
                    p.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let p = config_path();
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, s).map_err(|e| format!("{} に保存できません: {e}", p.display()))
    }
}

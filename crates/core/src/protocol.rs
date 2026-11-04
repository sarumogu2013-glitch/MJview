//! サーバー⇔クライアントの通信メッセージ（設計書 3.2・6章）。WebSocket のテキストフレームに JSON で載せる。

use serde::{Deserialize, Serialize};

use crate::engine::Overlay;
use crate::scoring::{WinInput, WinScore};
use crate::state::Action;
use crate::view::TableStateView;

/// 通信の版。合わないときは接続時に警告する。
pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47800;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientRole {
    /// 配信（出力ウィンドウ・操作ウィンドウ）
    Broadcast,
    /// 入力担当
    Input,
    /// 選手用画面（公開情報だけを受け取る）
    PlayerScreen,
}

impl ClientRole {
    pub fn name_ja(self) -> &'static str {
        match self {
            ClientRole::Broadcast => "配信",
            ClientRole::Input => "入力担当",
            ClientRole::PlayerScreen => "選手用画面",
        }
    }
    /// 操作を送れるロールか。
    pub fn can_operate(self) -> bool {
        !matches!(self, ClientRole::PlayerScreen)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMsg {
    Join {
        role: ClientRole,
        code: String,
        client_version: String,
        protocol: u32,
    },
    Action {
        request_id: u64,
        action: Action,
    },
    Undo {
        request_id: u64,
    },
    Redo {
        request_id: u64,
    },
    /// アガリの計算だけ（状態は変えない）
    PreviewRequest {
        request_id: u64,
        input: WinInput,
    },
    Ping {
        client_time: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMsg {
    Hello {
        server_version: String,
        protocol: u32,
        server_time: i64,
    },
    /// 接続を受け付けなかった（接続コード違いなど）。この後切断する
    Rejected {
        reason: String,
    },
    /// 状態の全体。接続直後・取り直しのとき
    Snapshot {
        seq: u64,
        server_time: i64,
        view: TableStateView,
    },
    /// 状態の変更。seq は1ずつ増える（抜けたらスナップショットを取り直す）
    Event {
        seq: u64,
        at: i64,
        view: TableStateView,
        overlays: Vec<Overlay>,
    },
    ActionResult {
        request_id: u64,
        error: Option<String>,
    },
    PreviewResult {
        request_id: u64,
        result: Result<WinScore, String>,
    },
    Pong {
        client_time: i64,
        server_time: i64,
    },
    /// 他の端末の操作で、同じ項目が後から上書きされたときの通知など
    Notice {
        message: String,
    },
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serialize")
    }
}

impl ServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serialize")
    }
}

/// 現在時刻（UNIX ミリ秒）。
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

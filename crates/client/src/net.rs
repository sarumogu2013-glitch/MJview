//! サーバーとの接続を Bevy の Resource にする。毎フレーム届いた出来事を取り込む。

use std::sync::Mutex;

use bevy::prelude::*;
use janscope_core::engine::Overlay;
use janscope_core::protocol::ClientRole;
use janscope_core::scoring::{WinInput, WinScore};
use janscope_core::state::Action;
use janscope_core::view::{FullView, PublicView, TableStateView};
use janscope_netclient::{ConnectConfig, NetClient, NetEvent};

#[derive(Resource)]
pub struct Net(Mutex<NetClient>);

impl Net {
    pub fn connect(addr: &str, code: &str, role: ClientRole) -> Self {
        Net(Mutex::new(NetClient::connect(ConnectConfig {
            addr: addr.into(),
            code: code.into(),
            role,
            client_version: janscope_core::VERSION.into(),
        })))
    }
    pub fn act(&self, a: Action) -> u64 {
        self.0.lock().unwrap().send_action(a)
    }
    pub fn undo(&self) -> u64 {
        self.0.lock().unwrap().undo()
    }
    pub fn redo(&self) -> u64 {
        self.0.lock().unwrap().redo()
    }
    pub fn preview(&self, input: WinInput) -> u64 {
        self.0.lock().unwrap().preview(input)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Link {
    #[default]
    Connecting,
    Connected,
    Disconnected(String),
    Rejected(String),
}

/// 受け取った卓の状態と接続の状態。
#[derive(Resource, Default)]
pub struct Table {
    pub link: Link,
    pub server_version: Option<String>,
    pub seq: u64,
    pub full: Option<FullView>,
    pub public: Option<PublicView>,
    /// 前回の描画から届いた演出の出来事（出力側で消費する）
    pub overlays: Vec<(i64, Overlay)>,
    /// サーバー時刻 − 手元の時刻（ms）
    pub clock_offset_ms: i64,
    pub rtt_ms: i64,
    /// 最後に届いた操作結果のエラー（操作ウィンドウに出す）
    pub last_error: Option<String>,
    pub notices: Vec<String>,
    pub preview: Option<(u64, Result<Box<WinScore>, String>)>,
    /// 状態が変わったフレームで true
    pub changed: bool,
    /// 他の端末（入力担当など）の操作で変わったとき、そのロール
    pub last_by: Option<ClientRole>,
}

impl Table {
    /// 公開情報（配信ロールでも全情報から作る）。
    pub fn public_view(&self) -> Option<PublicView> {
        if let Some(p) = &self.public {
            return Some(p.clone());
        }
        self.full.as_ref().map(|f| PublicView::from_state(&f.state))
    }
}

pub fn poll_net(net: Res<Net>, mut table: ResMut<Table>) {
    table.changed = false;
    let events = net.0.lock().unwrap().poll();
    for e in events {
        match e {
            NetEvent::Connecting { .. } => {
                if !matches!(table.link, Link::Disconnected(_)) {
                    table.link = Link::Connecting;
                }
            }
            NetEvent::Connected { server_version } => {
                table.link = Link::Connected;
                table.server_version = Some(server_version);
            }
            NetEvent::Disconnected { reason } => table.link = Link::Disconnected(reason),
            NetEvent::Rejected { reason } => table.link = Link::Rejected(reason),
            NetEvent::State { seq, at, view, overlays, by, .. } => {
                table.seq = seq;
                match *view {
                    TableStateView::Full(f) => table.full = Some(*f),
                    TableStateView::Public(p) => table.public = Some(*p),
                }
                for o in overlays {
                    table.overlays.push((at, o));
                }
                table.last_by = by;
                table.changed = true;
            }
            NetEvent::ActionResult { error, .. } => table.last_error = error,
            NetEvent::Preview { request_id, result } => table.preview = Some((request_id, result)),
            NetEvent::Notice { message } => table.notices.push(message),
            NetEvent::Clock { offset_ms, rtt_ms } => {
                table.clock_offset_ms = offset_ms;
                table.rtt_ms = rtt_ms;
            }
        }
    }
}

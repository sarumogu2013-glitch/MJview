//! JanScope の中核: 卓状態・大会ルール・点数計算（mahc）・操作ログ・通信の型。
//!
//! サーバー（処理サーバー）とクライアント（配信・入力担当・選手用画面）の両方が使う。描画や通信の実装は持たない。

pub mod delay;
pub mod dummy;
pub mod engine;
pub mod face;
pub mod hands;
pub mod log;
pub mod paifu;
pub mod protocol;
pub mod rules;
pub mod scene;
pub mod scoring;
pub mod state;
pub mod tile;
pub mod tournament;
pub mod view;

pub use engine::{EngineError, Overlay, apply, preview_win};
pub use log::GameLog;
pub use rules::GameConfig;
pub use state::{Action, Phase, Player, TableState};
pub use tile::{SeatId, Tile, Wind};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

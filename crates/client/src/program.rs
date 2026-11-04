//! 出力ウィンドウに出す状態（配信ディレイ後）。設計書 4.11:
//! 操作ウィンドウ・入力担当・選手用画面は常にライブ（net::Table）。出力ウィンドウだけが「今 − ディレイ秒」の状態を描く。

use std::time::{Duration, Instant};

use bevy::prelude::*;
use janscope_core::delay::DelayLine;
use janscope_core::engine::Overlay;
use janscope_core::view::FullView;

use crate::config::ClientConfig;

/// ディレイ後の状態（出力ウィンドウ・選手画面の切替が読む）。
#[derive(Resource, Default)]
pub struct Program {
    pub full: Option<FullView>,
    /// 出力側で状態が変わるたびに増える
    pub seq: u64,
    /// まだ処理していない演出の出来事
    pub overlays: Vec<(i64, Overlay)>,
}

#[derive(Clone)]
pub struct DelayedItem {
    pub full: FullView,
    pub overlays: Vec<(i64, Overlay)>,
}

/// 届いた状態を時刻付きで溜める。
#[derive(Resource, Default)]
pub struct Delayed {
    pub line: DelayLine<DelayedItem>,
}

impl Delayed {
    pub fn push(&mut self, item: DelayedItem) {
        self.line.push(Instant::now(), item);
    }
}

pub fn delay_of(cfg: &ClientConfig) -> Duration {
    Duration::from_secs_f64(cfg.delay_secs.max(0.0))
}

/// ディレイ秒を過ぎたものを出力側の状態に移す。
pub fn release(mut delayed: ResMut<Delayed>, mut program: ResMut<Program>, cfg: Res<ClientConfig>) {
    for item in delayed.line.drain_due(Instant::now(), delay_of(&cfg)) {
        program.full = Some(item.full);
        program.overlays.extend(item.overlays);
        program.seq += 1;
    }
}

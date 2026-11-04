//! 選手画面（顔カメラ）の実行時の状態: 切替（core::face）・顔カメラの取り込み・OBS 連携をまとめる。

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use janscope_core::engine::Overlay;
use janscope_core::face::{FaceContext, FaceSwitcher, TriggerKind};
use janscope_core::tile::SeatId;
use janscope_video::obs::{ObsLink, ObsStatus};
use janscope_video::{FaceStream, StreamConfig, StreamStatus, decode_rgba};

use crate::config::{ClientConfig, FaceOutput};
use crate::program::Program;

pub const FACE_W: u32 = 1066;
pub const FACE_H: u32 = 600;

#[derive(Resource)]
pub struct FaceRuntime {
    pub sw: FaceSwitcher,
    pub streams: [Option<FaceStream>; 4],
    pub images: [Handle<Image>; 4],
    last_seq: [u64; 4],
    /// 一度でも映像が入った席（画像の中身が有効）
    has_frame: [bool; 4],
    pub obs: Option<ObsLink>,
    pub current: SeatId,
    pub output: FaceOutput,
    pub delay_secs: f64,
    pub sync_offset_ms: [i32; 4],
}

impl FaceRuntime {
    pub fn status(&self, seat: SeatId) -> Option<StreamStatus> {
        self.streams[seat.index()].as_ref().map(|s| s.status())
    }

    pub fn live(&self, seat: SeatId) -> bool {
        self.has_frame[seat.index()] && self.status(seat) == Some(StreamStatus::Live)
    }

    pub fn obs_status(&self) -> ObsStatus {
        self.obs
            .as_ref()
            .map(|o| o.status())
            .unwrap_or(ObsStatus::Disabled)
    }

    /// 設定を反映する（顔カメラの取り込みと OBS 連携を起動し直す）。
    pub fn apply(&mut self, cfg: &ClientConfig) {
        self.sw.config = cfg.face_switch.clone();
        self.output = cfg.face_output;
        self.delay_secs = cfg.delay_secs;
        self.sync_offset_ms = cfg.sync_offset_ms;
        self.streams = std::array::from_fn(|i| {
            let f = &cfg.faces[i];
            (cfg.face_output == FaceOutput::Video && f.enabled && !f.url.trim().is_empty()).then(
                || {
                    FaceStream::start(StreamConfig {
                        url: f.url.trim().to_string(),
                        mode: f.mode,
                        width: FACE_W,
                        height: FACE_H,
                        fps: 30,
                        ffmpeg: cfg.ffmpeg.clone(),
                    })
                },
            )
        });
        let max_off = cfg.sync_offset_ms.iter().copied().max().unwrap_or(0).max(0) as f64 / 1000.0;
        let retain = std::time::Duration::from_secs_f64(cfg.delay_secs.max(0.0) + max_off + 3.0);
        for s in self.streams.iter().flatten() {
            s.set_retain(retain);
        }
        self.has_frame = [false; 4];
        self.last_seq = [0; 4];
        self.obs = (cfg.face_output != FaceOutput::Video && cfg.obs.enabled)
            .then(|| ObsLink::start(cfg.obs.clone()));
    }
}

pub fn setup_face(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cfg: Res<ClientConfig>,
) {
    let make = || {
        Image::new_fill(
            Extent3d {
                width: FACE_W,
                height: FACE_H,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        )
    };
    let imgs: [Handle<Image>; 4] = std::array::from_fn(|_| images.add(make()));
    let mut rt = FaceRuntime {
        sw: FaceSwitcher::new(cfg.face_switch.clone(), SeatId(0)),
        streams: [None, None, None, None],
        images: imgs,
        last_seq: [0; 4],
        has_frame: [false; 4],
        obs: None,
        current: SeatId(0),
        output: cfg.face_output,
        delay_secs: cfg.delay_secs,
        sync_offset_ms: cfg.sync_offset_ms,
    };
    rt.apply(&cfg);
    commands.insert_resource(rt);
}

/// 演出の出来事を切替のきっかけにする。
pub fn face_triggers(table: &Program, rt: &mut FaceRuntime, overlays: &[(i64, Overlay)], now: f64) {
    let _ = table;
    for (_, o) in overlays {
        match o {
            Overlay::Riichi { seat } => rt.sw.trigger(TriggerKind::Riichi, vec![*seat], now),
            Overlay::Kan { seat } => rt.sw.trigger(TriggerKind::Kan, vec![*seat], now),
            Overlay::Win { winners, loser } => {
                if let Some(l) = loser {
                    rt.sw.trigger(TriggerKind::DealIn, vec![*l], now);
                }
                rt.sw.trigger(TriggerKind::Win, winners.clone(), now);
            }
            Overlay::HandStarted | Overlay::GameStarted => rt.sw.clear_triggers(),
            _ => {}
        }
    }
}

/// 毎フレーム: 映す席を決め、その席の最新の映像を画像に書き込み、OBS に反映する。
pub fn face_update(
    time: Res<Time>,
    table: Res<Program>,
    mut rt: ResMut<FaceRuntime>,
    mut images: ResMut<Assets<Image>>,
) {
    let now = time.elapsed_secs_f64();
    let dealer = table
        .full
        .as_ref()
        .map(|f| f.state.dealer())
        .unwrap_or(SeatId(0));
    let any_stream = rt.streams.iter().any(|s| s.is_some());
    let has_signal: [bool; 4] = std::array::from_fn(|i| {
        if rt.output != FaceOutput::Video || !any_stream {
            true
        } else {
            rt.streams[i]
                .as_ref()
                .is_some_and(|s| s.status() == StreamStatus::Live)
        }
    });
    let ctx = FaceContext { dealer, has_signal };
    let cur = rt.sw.update(now, &ctx);
    rt.current = cur;
    // 映している席の映像だけをデコードする（操作ウィンドウのプレビューは段階B8で足す）
    let i = cur.index();
    // 配信ディレイ＋この席の同期補正だけ前の映像を出す
    let back = std::time::Duration::from_secs_f64(
        (rt.delay_secs + rt.sync_offset_ms[i] as f64 / 1000.0).max(0.0),
    );
    let target = std::time::Instant::now()
        .checked_sub(back)
        .unwrap_or_else(std::time::Instant::now);
    if let Some(f) = rt.streams[i].as_ref().and_then(|s| s.frame_at(target)) {
        if f.seq != rt.last_seq[i] {
            rt.last_seq[i] = f.seq;
            match decode_rgba(&f.data) {
                Ok((w, h, px)) if w == FACE_W && h == FACE_H => {
                    if let Some(mut img) = images.get_mut(&rt.images[i]) {
                        img.data = Some(px);
                        rt.has_frame[i] = true;
                    }
                }
                Ok((w, h, _)) => warn!("顔カメラ{}の大きさが違います: {w}x{h}", cur.label()),
                Err(e) => warn!("顔カメラ{}の映像を読めません: {e}", cur.label()),
            }
        }
    }
    if let Some(o) = &rt.obs {
        o.show_seat(cur.0);
    }
}

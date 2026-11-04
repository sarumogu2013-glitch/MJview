//! シーン（設計書 4.3）: 対局 ／ 全画面映像 ／ 選手紹介 ／ 待機・休憩 ／ 半荘結果、とトランジション・テロップ。
//! 牌譜再生・通算順位のシーンは段階B8で足す。

use std::io::Write;

use bevy::prelude::*;
use janscope_core::scene::{Curtain, SceneKind, SceneSwitcher};
use janscope_core::state::TableState;
use janscope_core::tile::SeatId;

use crate::config::FaceOutput;
use crate::face::FaceRuntime;
use crate::output::OutputCamera;
use crate::ui_kit::*;

#[derive(Resource)]
pub struct Scenes {
    pub sw: SceneSwitcher,
    /// テロップ（None なら出さない）
    pub telop: Option<String>,
    /// 待機・休憩シーンの文字
    pub waiting_text: String,
    pub curtain: Curtain,
}

impl Default for Scenes {
    fn default() -> Self {
        Scenes {
            sw: SceneSwitcher::default(),
            telop: None,
            waiting_text: "まもなく再開します".into(),
            curtain: Curtain::None,
        }
    }
}

impl Scenes {
    /// プレビューを放送に出す（映像の切替操作はログに残す。Undo の対象ではない）。
    pub fn take(&mut self, now: f64) {
        let target = self.sw.preview;
        if self.sw.take(now) {
            video_log(&format!(
                "切替 {} ({}, {:.1}秒)",
                target.name_ja(),
                self.sw.transition.name_ja(),
                self.sw.duration
            ));
        }
    }
}

/// 映像の切替操作の記録（設計書 5.5: ログに残すが Undo の対象にはしない）。
pub fn video_log(msg: &str) {
    let path =
        std::env::var("JANSCOPE_VIDEO_LOG").unwrap_or_else(|_| "janscope-video-log.txt".into());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}\t{msg}", janscope_core::protocol::now_ms());
    }
}

#[derive(Component)]
pub struct CurtainNode;

/// トランジションを進め、幕（全画面の黒）を更新する。
pub fn update_scenes(
    mut commands: Commands,
    time: Res<Time>,
    mut scenes: ResMut<Scenes>,
    cam: Query<Entity, With<OutputCamera>>,
    mut q: Query<(&mut Node, &mut BackgroundColor), With<CurtainNode>>,
) {
    let now = time.elapsed_secs_f64();
    scenes.curtain = scenes.sw.update(now);
    let Ok(cam) = cam.single() else { return };
    let Ok((mut node, mut bg)) = q.single_mut() else {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(0.0),
                height: Val::Px(1080.0),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(100),
            UiTargetCamera(cam),
            CurtainNode,
        ));
        return;
    };
    match scenes.curtain {
        Curtain::None => node.width = Val::Px(0.0),
        Curtain::Fade(a) => {
            node.left = Val::Px(0.0);
            node.width = Val::Px(1920.0);
            bg.0 = Color::srgba(0.0, 0.0, 0.0, a);
        }
        Curtain::Wipe { left, right } => {
            node.left = Val::Px(left * 1920.0);
            node.width = Val::Px((right - left) * 1920.0);
            bg.0 = Color::BLACK;
        }
    }
}

/// このシーンで映像が必要な席（顔カメラのデコード対象）。
pub fn seats_needed(scene: SceneKind) -> [bool; 4] {
    match scene {
        SceneKind::Fullscreen(s) => std::array::from_fn(|i| i == s.index()),
        SceneKind::PlayerIntro | SceneKind::FinalResult => [true; 4],
        _ => [false; 4],
    }
}

fn face_image(
    p: &mut Kids,
    k: &Kit,
    rt: &FaceRuntime,
    seat: SeatId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    name: &str,
) {
    let bg = if rt.output == FaceOutput::Chroma {
        Color::srgb(0.0, 1.0, 0.0)
    } else {
        k.theme.face_bg
    };
    p.spawn((abs(x, y, w, h), BackgroundColor(bg)))
        .with_children(|q| {
            if rt.output == FaceOutput::Video && rt.live(seat) {
                q.spawn((
                    abs(0.0, 0.0, w, h),
                    ImageNode {
                        image: rt.images[seat.index()].clone(),
                        ..default()
                    },
                ));
            } else if rt.output == FaceOutput::Video {
                q.spawn(center(abs(0.0, 0.0, w, h))).with_children(|c| {
                    k.text(
                        c,
                        name.to_string(),
                        h * 0.1,
                        Color::srgb(0.3, 0.32, 0.35),
                        true,
                    )
                });
            }
        });
}

pub fn draw_fullscreen(p: &mut Kids, k: &Kit, s: &TableState, rt: &FaceRuntime, seat: SeatId) {
    let name = s.seats[seat.index()].player.name.clone();
    face_image(p, k, rt, seat, 0.0, 0.0, 1920.0, 1080.0, &name);
    p.spawn((
        row(abs(0.0, 960.0, 1920.0, 120.0), 28.0),
        BackgroundColor(k.theme.band),
    ))
    .with_children(|b| {
        b.spawn(Node {
            width: Val::Px(30.0),
            ..default()
        });
        k.text(b, s.seat_wind(seat).name_ja(), 64.0, k.theme.text, true);
        k.text(b, name, 72.0, k.theme.text, true);
        if let Some(a) = &s.seats[seat.index()].player.affiliation {
            k.text(b, a.clone(), 40.0, k.theme.text_dim, false);
        }
    });
}

pub fn draw_intro(p: &mut Kids, k: &Kit, s: &TableState, rt: &FaceRuntime) {
    let th = k.theme;
    p.spawn(center(abs(0.0, 30.0, 1920.0, 100.0)))
        .with_children(|c| k.text(c, "選手紹介", 64.0, th.text, true));
    let ranks = s.ranks();
    for (i, seat) in SeatId::ALL.iter().enumerate() {
        let x = 40.0 + i as f32 * 470.0;
        let st = &s.seats[seat.index()];
        p.spawn((abs(x, 170.0, 430.0, 840.0), BackgroundColor(th.panel)))
            .with_children(|card| {
                face_image(
                    card,
                    k,
                    rt,
                    *seat,
                    0.0,
                    0.0,
                    430.0,
                    430.0 * 9.0 / 16.0 * 1.6,
                    &st.player.name,
                );
                card.spawn(Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(400.0),
                    left: Val::Px(24.0),
                    width: Val::Px(382.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|col| {
                    k.text(
                        col,
                        format!("席{}  {}", seat.label(), s.seat_wind(*seat).name_ja()),
                        30.0,
                        th.text_dim,
                        false,
                    );
                    k.text(col, st.player.name.clone(), 56.0, th.text, true);
                    if let Some(a) = &st.player.affiliation {
                        k.text(col, a.clone(), 32.0, th.text_dim, false);
                    }
                    if s.phase != janscope_core::state::Phase::Setup {
                        k.text(
                            col,
                            format!(
                                "{}点  {}位",
                                janscope_core::scoring::fmt_pts(st.score),
                                ranks[seat.index()]
                            ),
                            36.0,
                            th.text,
                            true,
                        );
                    }
                });
            });
    }
}

pub fn draw_waiting(p: &mut Kids, k: &Kit, text: &str) {
    p.spawn((
        center(abs(0.0, 0.0, 1920.0, 1080.0)),
        BackgroundColor(k.theme.panel),
    ))
    .with_children(|c| {
        c.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(30.0),
            ..default()
        })
        .with_children(|col| {
            k.text(col, "JanScope", 40.0, k.theme.text_dim, false);
            k.text(col, text.to_string(), 96.0, k.theme.text, true);
        });
    });
}

pub fn draw_final(p: &mut Kids, k: &Kit, s: &TableState, rt: &FaceRuntime) {
    let th = k.theme;
    let title = if s.final_results.is_some() {
        "半荘結果"
    } else {
        "途中経過"
    };
    p.spawn(center(abs(0.0, 30.0, 1920.0, 110.0)))
        .with_children(|c| k.text(c, title, 72.0, th.text, true));
    // 終局していなければ現在の順位でポイントを仮計算
    let results = s
        .final_results
        .clone()
        .unwrap_or_else(|| janscope_core::engine::final_results(s));
    for (i, r) in results.iter().enumerate() {
        let y = 180.0 + i as f32 * 210.0;
        let st = &s.seats[r.seat.index()];
        p.spawn((
            row(abs(160.0, y, 1600.0, 190.0), 30.0),
            BackgroundColor(if i == 0 { th.panel_alt } else { th.panel }),
        ))
        .with_children(|rw| {
            rw.spawn(Node {
                width: Val::Px(20.0),
                ..default()
            });
            rw.spawn(Node {
                width: Val::Px(130.0),
                ..default()
            })
            .with_children(|c| {
                k.text(
                    c,
                    format!("{}位", r.rank),
                    72.0,
                    if i == 0 { th.riichi } else { th.text },
                    true,
                )
            });
            rw.spawn(Node {
                width: Val::Px(300.0),
                height: Val::Px(169.0),
                ..default()
            })
            .with_children(|c| {
                face_image(c, k, rt, r.seat, 0.0, 0.0, 300.0, 169.0, &st.player.name);
            });
            rw.spawn(Node {
                width: Val::Px(520.0),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|c| {
                k.text(c, st.player.name.clone(), 60.0, th.text, true);
                if let Some(a) = &st.player.affiliation {
                    k.text(c, a.clone(), 28.0, th.text_dim, false);
                }
            });
            rw.spawn(Node {
                width: Val::Px(280.0),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_children(|c| {
                k.text(
                    c,
                    janscope_core::scoring::fmt_pts(r.score),
                    60.0,
                    th.text,
                    true,
                )
            });
            rw.spawn(Node {
                width: Val::Px(260.0),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_children(|c| {
                let col = if r.points >= 0.0 { th.plus } else { th.minus };
                k.text(c, format!("{:+.1}", r.points), 64.0, col, true);
            });
        });
    }
}

/// テロップ（対局シーンはスコア表の下端、ほかのシーンは画面下）。
pub fn draw_telop(p: &mut Kids, k: &Kit, text: &str, game: bool) {
    let (x, y, w, h) = if game {
        (1066.0, 340.0, 854.0, 60.0)
    } else {
        (0.0, 980.0, 1920.0, 100.0)
    };
    p.spawn((
        center(abs(x, y, w, h)),
        BackgroundColor(Color::srgb(0.05, 0.15, 0.35)),
        GlobalZIndex(50),
    ))
    .with_children(|c| {
        k.text(
            c,
            text.to_string(),
            if game { 30.0 } else { 48.0 },
            Color::WHITE,
            true,
        )
    });
}

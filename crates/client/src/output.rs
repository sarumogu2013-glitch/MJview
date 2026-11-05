//! 配信用の出力ウィンドウ（設計書 4.2 対局画面レイアウト 1920×1080）。
//!
//! 状態が変わるたびに画面全体を組み直す（部品が少なく変更も1秒に数回なので十分軽い）。
//! 顔カメラの映像（RTMP）は段階B6、手元枠の手牌・河の自動読み取りは段階Cで入る。それまでは黒枠と「手動」表示。

use bevy::prelude::*;
use janscope_core::engine::Overlay;
use janscope_core::scoring::{Meld, MeldKind};
use janscope_core::state::{HandOutcome, Phase, TableState, WinRecord};
use janscope_core::tile::{SeatId, Tile};

use crate::assets::{Fonts, TileImages};
use crate::config::FaceOutput;
use crate::face::FaceRuntime;
use crate::program::Program;
use crate::theme::Theme;
use crate::ui_kit::*;

/// 出力ウィンドウのカメラ（UI の描画先）。
#[derive(Component)]
pub struct OutputCamera;

#[derive(Component)]
pub struct OutputRoot;

/// アガリ演出の表示（設計書 4.2「アガリ演出」の簡易版）。
#[derive(Resource, Default)]
pub struct WinShow {
    pub records: Vec<WinRecord>,
    pub started: f64,
    /// 1件あたりの表示秒数
    pub per: f64,
}

impl WinShow {
    pub fn current(&self, now: f64) -> Option<&WinRecord> {
        if self.records.is_empty() {
            return None;
        }
        let i = ((now - self.started) / self.per).floor();
        if i < 0.0 {
            return None;
        }
        self.records.get(i as usize)
    }
}

/// 組み直しが必要か（前回の表示内容のかぎ）。
#[derive(Resource, Default)]
pub struct OutputDirty {
    last_key: String,
}

/// 演出の出来事を受けて、選手画面・アガリ演出を切り替える。
pub fn handle_overlays(
    cfg: Res<crate::config::ClientConfig>,
    mut table: ResMut<Program>,
    mut face: ResMut<FaceRuntime>,
    mut win: ResMut<WinShow>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let overlays: Vec<_> = table.overlays.drain(..).collect();
    crate::face::face_triggers(&table, &mut face, &overlays, now);
    // アガリ演出は選手画面のアガリ表示と同じ秒数で、アガった人ごとに順に出す
    // アガリ演出の1人あたりの秒数（設定の演出タブ）
    let per = cfg.prefs.presentation.win_secs.max(3.0);
    for (_, o) in overlays {
        match o {
            Overlay::Win { .. } => {
                if let Some(st) = table.full.as_ref().map(|f| &f.state) {
                    if let Some(h) = st.history.last() {
                        if let HandOutcome::Win(rs) = &h.outcome {
                            win.records = rs.clone();
                            win.started = now;
                            win.per = per;
                        }
                    }
                }
            }
            Overlay::HandStarted | Overlay::GameStarted => win.records.clear(),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn rebuild_output(
    mut commands: Commands,
    table: Res<Program>,
    face: Res<FaceRuntime>,
    win: Res<WinShow>,
    mut dirty: ResMut<OutputDirty>,
    roots: Query<Entity, With<OutputRoot>>,
    cam: Query<Entity, With<OutputCamera>>,
    fonts: Option<Res<Fonts>>,
    tiles: Option<Res<TileImages>>,
    theme: Res<Theme>,
    time: Res<Time>,
    scenes: Res<crate::scenes::Scenes>,
    mut anim: ResMut<ScoreAnim>,
    replay: Res<crate::replay::Replay>,
    cfg: Res<crate::config::ClientConfig>,
) {
    let (Some(fonts), Some(tiles), Ok(cam)) = (fonts, tiles, cam.single()) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let st = table.full.as_ref().map(|f| &f.state);
    // 点数移動のアニメーション（前の表示から新しい点数へ数える）
    if let Some(s) = st {
        let to = s.scores();
        if to != anim.to {
            anim.from = anim.shown(now);
            anim.to = to;
            anim.start = now;
            if !anim.initialized {
                anim.from = to;
                anim.initialized = true;
            }
        }
    }
    anim.duration = cfg.prefs.presentation.score_anim_secs;
    let shown = anim.shown(now);
    let step = anim.step(now);
    let face_seat = st.map(|_| face.current);
    let live = face_seat.is_some_and(|f| face.live(f));
    let showing = win.current(now).map(|r| r.input.winner);
    let key = format!(
        "{}|{:?}|{:?}|{:?}|{:?}",
        table.seq,
        face_seat,
        (live, face.output),
        showing,
        (
            table.full.is_some(),
            scenes.sw.program,
            &scenes.telop,
            &scenes.waiting_text,
            step,
            (
                replay.hand,
                replay.step,
                replay.viewpoint,
                replay.open_all,
                replay.file.len(),
                replay.paifu.is_some()
            ),
            format!("{:?}", cfg.prefs)
        )
    );
    if key == dirty.last_key {
        return;
    }
    dirty.last_key = key;
    for e in &roots {
        commands.entity(e).despawn();
    }
    let kit = Kit {
        fonts: &fonts,
        tiles: &tiles,
        theme: &theme,
        prefs: &cfg.prefs,
    };
    commands
        .spawn((
            Node {
                width: Val::Px(1920.0),
                height: Val::Px(1080.0),
                ..default()
            },
            BackgroundColor(theme.bg),
            UiTargetCamera(cam),
            OutputRoot,
        ))
        .with_children(|p| match st {
            None => {
                p.spawn(center(abs(0.0, 0.0, 1920.0, 1080.0)))
                    .with_children(|q| {
                        kit.text(
                            q,
                            "サーバーからの状態を待っています",
                            48.0,
                            theme.text_dim,
                            false,
                        );
                    });
            }
            Some(s) if scenes.sw.program != janscope_core::scene::SceneKind::Game => {
                use janscope_core::scene::SceneKind;
                match scenes.sw.program {
                    SceneKind::Fullscreen(seat) => {
                        crate::scenes::draw_fullscreen(p, &kit, s, &face, seat)
                    }
                    SceneKind::PlayerIntro => crate::scenes::draw_intro(p, &kit, s, &face),
                    SceneKind::Waiting => {
                        crate::scenes::draw_waiting(p, &kit, &scenes.waiting_text)
                    }
                    SceneKind::FinalResult => crate::scenes::draw_final(p, &kit, s, &face),
                    SceneKind::Replay => crate::replay::draw_replay(p, &kit, &replay),
                    SceneKind::Standings => crate::scenes::draw_standings(
                        p,
                        &kit,
                        &table.full.as_ref().unwrap().tournament,
                    ),
                    SceneKind::Game => {}
                }
                if let Some(t) = &scenes.telop {
                    crate::scenes::draw_telop(p, &kit, t, false);
                }
            }
            Some(s) => {
                let face_seat = face_seat.unwrap();
                let lay = &cfg.prefs.layout;
                if lay.show_face {
                    draw_face(p, &kit, s, face_seat, &face);
                }
                if lay.show_round {
                    draw_round(p, &kit, s);
                }
                if lay.show_scores {
                    draw_scores(p, &kit, s, face_seat, shown);
                }
                if lay.show_dice {
                    draw_dice(p, &kit, s);
                }
                if lay.show_dora {
                    draw_dora(p, &kit, s);
                }
                if lay.show_wall {
                    draw_wall(p, &kit, s);
                }
                if lay.show_hand_frames {
                    for (i, seat) in SeatId::ALL.iter().enumerate() {
                        let rec = win.current(now).filter(|r| r.input.winner == *seat);
                        draw_hand_frame(
                            p,
                            &kit,
                            s,
                            *seat,
                            i as f32 * 480.0,
                            face_seat == *seat,
                            rec,
                        );
                    }
                }
                if let Some(r) = win.current(now) {
                    draw_win(p, &kit, s, r);
                } else if let Some(t) = &scenes.telop {
                    crate::scenes::draw_telop(p, &kit, t, true);
                }
            }
        });
}

fn wind_dealer(s: &TableState, seat: SeatId) -> (String, bool) {
    (s.seat_wind(seat).name_ja().to_string(), s.dealer() == seat)
}

fn draw_face(p: &mut Kids, k: &Kit, s: &TableState, seat: SeatId, rt: &FaceRuntime) {
    let th = k.theme;
    let bg = match rt.output {
        FaceOutput::Chroma => Color::srgb(0.0, 1.0, 0.0),
        _ => th.face_bg,
    };
    p.spawn((abs(0.0, 0.0, 1066.0, 600.0), BackgroundColor(bg)))
        .with_children(|q| {
            match rt.output {
                FaceOutput::Video if rt.live(seat) => {
                    q.spawn((
                        abs(0.0, 0.0, 1066.0, 600.0),
                        ImageNode {
                            image: rt.images[seat.index()].clone(),
                            ..default()
                        },
                    ));
                }
                FaceOutput::Video => {
                    // 信号なし: 選手名入りの代替表示
                    q.spawn(center(abs(0.0, 0.0, 1066.0, 510.0)))
                        .with_children(|c| {
                            c.spawn(Node {
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Center,
                                ..default()
                            })
                            .with_children(|col| {
                                let configured = rt.status(seat).is_some();
                                k.text(
                                    col,
                                    if configured {
                                        "NO SIGNAL"
                                    } else {
                                        "顔カメラ"
                                    },
                                    40.0,
                                    Color::srgb(0.35, 0.37, 0.40),
                                    true,
                                );
                                k.text(
                                    col,
                                    s.seats[seat.index()].player.name.clone(),
                                    32.0,
                                    Color::srgb(0.30, 0.32, 0.35),
                                    false,
                                );
                            });
                        });
                }
                // 黒枠・クロマキー: 何も描かない（OBS 側で顔映像を重ねる）
                _ => {}
            }
            q.spawn((
                row(abs(0.0, 510.0, 1066.0, 90.0), 24.0),
                BackgroundColor(th.band),
            ))
            .with_children(|b| {
                let (w, dealer) = wind_dealer(s, seat);
                b.spawn(Node {
                    width: Val::Px(12.0),
                    ..default()
                });
                k.text(b, w, 44.0, th.text, true);
                if dealer {
                    dealer_badge(b, k, 30.0);
                }
                k.text(
                    b,
                    s.seats[seat.index()].player.name.clone(),
                    48.0,
                    th.text,
                    true,
                );
                b.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });
                if s.seats[seat.index()].riichi {
                    riichi_badge(b, k, 34.0);
                }
                b.spawn(Node {
                    width: Val::Px(12.0),
                    ..default()
                });
            });
        });
}

fn dealer_badge(p: &mut Kids, k: &Kit, size: f32) {
    p.spawn((
        center(Node {
            padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        }),
        BackgroundColor(k.theme.dealer),
    ))
    .with_children(|q| k.text(q, "親", size, Color::WHITE, true));
}

fn riichi_badge(p: &mut Kids, k: &Kit, size: f32) {
    p.spawn((
        center(Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(0.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        }),
        BackgroundColor(k.theme.riichi),
    ))
    .with_children(|q| k.text(q, "リーチ", size, Color::BLACK, true));
}

fn draw_round(p: &mut Kids, k: &Kit, s: &TableState) {
    let th = k.theme;
    let pr = &k.prefs.presentation;
    p.spawn((
        row(abs(1066.0, 0.0, 854.0, 100.0), 36.0),
        BackgroundColor(th.panel_alt),
    ))
    .with_children(|q| {
        q.spawn(Node {
            width: Val::Px(8.0),
            ..default()
        });
        let label = match s.phase {
            Phase::Setup => "対局前".to_string(),
            Phase::GameOver => "終局".to_string(),
            _ => s.round_label(),
        };
        k.text(q, label, 52.0, th.text, true);
        k.text(q, format!("{}本場", s.honba), 40.0, th.text, false);
        let kyotaku = if pr.kyotaku_as_points {
            format!(
                "供託 {}",
                janscope_core::scoring::fmt_pts(s.kyotaku as i32 * 1000)
            )
        } else {
            format!("供託 {}本", s.kyotaku)
        };
        k.text(q, kyotaku, 36.0, th.text_dim, false);
        if pr.show_all_last && s.is_all_last() && s.phase != Phase::GameOver {
            q.spawn((
                center(Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                }),
                BackgroundColor(th.warn),
            ))
            .with_children(|b| k.text(b, "オーラス", 32.0, Color::WHITE, true));
        }
    });
}

fn draw_scores(p: &mut Kids, k: &Kit, s: &TableState, face_seat: SeatId, shown: [i32; 4]) {
    let th = k.theme;
    let ranks = s.ranks();
    let top = s.rank_order()[0];
    let top_score = s.seats[top.index()].score;
    p.spawn((abs(1066.0, 100.0, 854.0, 300.0), BackgroundColor(th.panel)))
        .with_children(|q| {
            // 並び順: 席順固定／順位順（設定のレイアウトタブ）
            let order: Vec<SeatId> = match k.prefs.layout.score_order {
                crate::prefs::ScoreOrder::Seat => SeatId::ALL.to_vec(),
                crate::prefs::ScoreOrder::Rank => s.rank_order(),
            };
            for (i, seat) in order.iter().enumerate() {
                let st = &s.seats[seat.index()];
                let bg = if *seat == face_seat {
                    th.panel_alt
                } else {
                    th.panel
                };
                q.spawn((
                    row(abs(0.0, i as f32 * 75.0, 854.0, 75.0), 16.0),
                    BackgroundColor(bg),
                ))
                .with_children(|r| {
                    // 選手画面に映っている選手の行は左端に色帯
                    r.spawn((
                        Node {
                            width: Val::Px(8.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(if *seat == face_seat { th.highlight } else { bg }),
                    ));
                    let (w, dealer) = wind_dealer(s, *seat);
                    r.spawn(Node {
                        width: Val::Px(44.0),
                        ..default()
                    })
                    .with_children(|c| k.text(c, w, 36.0, th.text, true));
                    r.spawn(Node {
                        width: Val::Px(52.0),
                        ..default()
                    })
                    .with_children(|c| {
                        if dealer {
                            dealer_badge(c, k, 26.0);
                        }
                    });
                    r.spawn(Node {
                        width: Val::Px(250.0),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|c| {
                        k.text(c, st.player.name.clone(), 34.0, th.text, true);
                        if let Some(a) = st
                            .player
                            .affiliation
                            .as_ref()
                            .filter(|_| k.prefs.layout.show_affiliation)
                        {
                            k.text(c, a.clone(), 18.0, th.text_dim, false);
                        }
                    });
                    r.spawn(Node {
                        width: Val::Px(90.0),
                        ..default()
                    })
                    .with_children(|c| {
                        if st.riichi {
                            riichi_badge(c, k, 20.0);
                        }
                    });
                    r.spawn(Node {
                        width: Val::Px(160.0),
                        justify_content: JustifyContent::FlexEnd,
                        ..default()
                    })
                    .with_children(|c| {
                        k.text(
                            c,
                            janscope_core::scoring::fmt_pts(shown[seat.index()]),
                            40.0,
                            th.text,
                            true,
                        )
                    });
                    r.spawn(Node {
                        width: Val::Px(130.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexEnd,
                        ..default()
                    })
                    .with_children(|c| {
                        let d = s.last_deltas[seat.index()];
                        if d != 0 {
                            k.text(
                                c,
                                delta_text(d),
                                26.0,
                                if d > 0 { th.plus } else { th.minus },
                                true,
                            );
                        } else if *seat != top && k.prefs.layout.show_score_diff {
                            k.text(
                                c,
                                delta_text(st.score - top_score),
                                22.0,
                                th.text_dim,
                                false,
                            );
                        }
                    });
                    r.spawn(Node {
                        width: Val::Px(60.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    })
                    .with_children(|c| {
                        k.text(
                            c,
                            format!("{}位", ranks[seat.index()]),
                            28.0,
                            th.text_dim,
                            false,
                        )
                    });
                });
            }
        });
}

fn draw_dice(p: &mut Kids, k: &Kit, s: &TableState) {
    let th = k.theme;
    p.spawn((
        center(abs(1066.0, 400.0, 260.0, 200.0)),
        BackgroundColor(th.panel_alt),
    ))
    .with_children(|q| {
        q.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|c| {
            k.text(c, "サイコロ", 22.0, th.text_dim, false);
            let hide = s.phase != Phase::InHand
                && k.prefs.layout.dice_prev == crate::prefs::DicePrev::Hidden;
            if s.dice.is_empty() || hide {
                k.text(c, "—", 48.0, th.text_dim, false);
            }
            for &(a, b) in s.dice.iter().filter(|_| !hide) {
                let faded = s.phase != Phase::InHand;
                c.spawn(row(Node::default(), 10.0)).with_children(|r| {
                    k.die(r, a, 64.0, faded);
                    k.die(r, b, 64.0, faded);
                    k.text(r, format!("{}", a + b), 34.0, th.text, true);
                });
            }
        });
    });
}

fn draw_dora(p: &mut Kids, k: &Kit, s: &TableState) {
    use crate::prefs::DoraMode;
    let th = k.theme;
    let mode = k.prefs.layout.dora_mode;
    p.spawn((
        center(abs(1326.0, 400.0, 334.0, 200.0)),
        BackgroundColor(th.panel),
    ))
    .with_children(|q| {
        q.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|c| {
            let label = match mode {
                DoraMode::Indicator => "ドラ表示牌",
                DoraMode::Dora => "ドラ",
                DoraMode::Both => "ドラ表示牌 → ドラ",
            };
            k.text(c, label, 22.0, th.text_dim, false);
            let n = s.dora_indicators.len().max(1) as f32;
            // 槓ドラで増えたら縮小して並べる（最大5枚）。両方のときは2段で小さく
            let full = if mode == DoraMode::Both { 56.0 } else { 80.0 };
            let w = (290.0 / n - 6.0).min(full);
            let rows: Vec<Vec<janscope_core::tile::Tile>> = match mode {
                DoraMode::Indicator => vec![s.dora_indicators.clone()],
                DoraMode::Dora => vec![
                    s.dora_indicators
                        .iter()
                        .map(|t| t.dora_from_indicator())
                        .collect(),
                ],
                DoraMode::Both => vec![
                    s.dora_indicators.clone(),
                    s.dora_indicators
                        .iter()
                        .map(|t| t.dora_from_indicator())
                        .collect(),
                ],
            };
            for tiles in rows {
                c.spawn(row(Node::default(), 6.0)).with_children(|r| {
                    if tiles.is_empty() {
                        k.tile(r, None, w, false, true);
                    }
                    for t in &tiles {
                        k.tile(r, Some(*t), w, false, false);
                    }
                });
            }
        });
    });
}

fn draw_wall(p: &mut Kids, k: &Kit, s: &TableState) {
    let th = k.theme;
    let low = s.phase == Phase::InHand && s.wall_remaining <= k.prefs.presentation.wall_warn_at;
    p.spawn((
        center(abs(1660.0, 400.0, 260.0, 200.0)),
        BackgroundColor(if low { th.warn } else { th.panel_alt }),
    ))
    .with_children(|q| {
        q.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|c| {
            k.text(
                c,
                "残り",
                22.0,
                if low { Color::WHITE } else { th.text_dim },
                false,
            );
            let txt = if s.phase == Phase::InHand {
                s.wall_remaining.to_string()
            } else {
                "—".into()
            };
            k.text(c, txt, 96.0, th.text, true);
        });
    });
}

fn draw_hand_frame(
    p: &mut Kids,
    k: &Kit,
    s: &TableState,
    seat: SeatId,
    x: f32,
    on_face: bool,
    win: Option<&WinRecord>,
) {
    let th = k.theme;
    let st = &s.seats[seat.index()];
    let border = if win.is_some() {
        th.riichi
    } else if on_face {
        th.highlight
    } else if st.riichi {
        th.riichi.with_alpha(0.6)
    } else {
        th.panel_alt
    };
    p.spawn((
        Node {
            border: UiRect::all(Val::Px(4.0)),
            ..abs(x, 600.0, 480.0, 480.0)
        },
        BackgroundColor(th.panel),
        BorderColor::all(border),
    ))
    .with_children(|q| {
        q.spawn((
            row(abs(0.0, 0.0, 472.0, 44.0), 12.0),
            BackgroundColor(th.band),
        ))
        .with_children(|b| {
            b.spawn(Node {
                width: Val::Px(6.0),
                ..default()
            });
            let (w, dealer) = wind_dealer(s, seat);
            k.text(b, w, 26.0, th.text, true);
            if dealer {
                dealer_badge(b, k, 20.0);
            }
            k.text(b, st.player.name.clone(), 26.0, th.text, true);
            b.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            if st.riichi {
                riichi_badge(b, k, 18.0);
            }
            if win.is_none() {
                k.text(b, "手動", 16.0, th.text_dim, false);
            }
            b.spawn(Node {
                width: Val::Px(6.0),
                ..default()
            });
        });
        // 手牌（アガリ時は倒した手）・副露
        q.spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            bottom: Val::Px(10.0),
            width: Val::Px(456.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::FlexEnd,
            column_gap: Val::Px(0.0),
            ..default()
        })
        .with_children(|r| {
            let (closed, melds): (Vec<Tile>, &[Meld]) = match win {
                Some(w) => {
                    let mut c = w.input.closed.clone();
                    if let Some(pos) = c.iter().position(|t| *t == w.input.win_tile) {
                        c.remove(pos);
                    }
                    c.sort_by_key(|t| t.sort_key());
                    c.push(w.input.win_tile);
                    (c, &w.input.melds)
                }
                None => (vec![], &st.melds),
            };
            let n_tiles = closed.len() + melds.iter().map(|m| m.tiles.len()).sum::<usize>() + 1;
            let w = (440.0 / n_tiles as f32).min(34.0);
            let last = closed.len().saturating_sub(1);
            for (i, t) in closed.iter().enumerate() {
                if win.is_some() && i == last {
                    r.spawn(Node {
                        width: Val::Px(w * 0.4),
                        ..default()
                    });
                }
                k.tile(r, Some(*t), w, false, false);
            }
            if !melds.is_empty() {
                r.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });
            }
            for m in melds {
                draw_meld(r, k, m, w);
                r.spawn(Node {
                    width: Val::Px(w * 0.2),
                    ..default()
                });
            }
        });
    });
}

/// 副露（鳴いた牌を横向きに）。
pub fn draw_meld(p: &mut Kids, k: &Kit, m: &Meld, w: f32) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::FlexEnd,
        ..default()
    })
    .with_children(|r| {
        for (i, t) in m.tiles.iter().enumerate() {
            let face_down = m.kind == MeldKind::Ankan && (i == 0 || i == 3);
            let sideways = m.kind != MeldKind::Ankan && i == 0;
            k.tile(
                r,
                if face_down { None } else { Some(*t) },
                w,
                sideways,
                false,
            );
        }
    });
}

fn draw_win(p: &mut Kids, k: &Kit, s: &TableState, r: &WinRecord) {
    let th = k.theme;
    let sc = &r.score;
    let who = &s.seats[r.input.winner.index()].player.name;
    p.spawn((
        abs(0.0, 0.0, 1920.0, 600.0),
        BackgroundColor(Color::srgb(0.02, 0.03, 0.05)),
    ))
    .with_children(|q| {
        q.spawn(row(abs(60.0, 30.0, 1800.0, 80.0), 28.0))
            .with_children(|t| {
                k.text(t, who.clone(), 60.0, th.text, true);
                let how = match r.input.loser {
                    None => "ツモ".to_string(),
                    Some(l) => format!("ロン（{}）", s.seats[l.index()].player.name),
                };
                k.text(t, how, 40.0, th.riichi, true);
            });
        // 手牌
        q.spawn(row(abs(60.0, 130.0, 1800.0, 110.0), 0.0))
            .with_children(|t| {
                let mut c = r.input.closed.clone();
                if let Some(pos) = c.iter().position(|x| *x == r.input.win_tile) {
                    c.remove(pos);
                }
                c.sort_by_key(|x| x.sort_key());
                for x in &c {
                    k.tile(t, Some(*x), 66.0, false, false);
                }
                t.spawn(Node {
                    width: Val::Px(20.0),
                    ..default()
                });
                k.tile(t, Some(r.input.win_tile), 66.0, false, false);
                t.spawn(Node {
                    width: Val::Px(40.0),
                    ..default()
                });
                for m in &r.input.melds {
                    draw_meld(t, k, m, 60.0);
                    t.spawn(Node {
                        width: Val::Px(12.0),
                        ..default()
                    });
                }
            });
        // 役一覧
        q.spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(60.0),
            top: Val::Px(270.0),
            width: Val::Px(1100.0),
            height: Val::Px(300.0),
            flex_direction: FlexDirection::Column,
            flex_wrap: FlexWrap::Wrap,
            row_gap: Val::Px(4.0),
            column_gap: Val::Px(60.0),
            ..default()
        })
        .with_children(|c| {
            for y in &sc.yaku {
                let v = if y.yakuman > 0 {
                    if y.yakuman > 1 {
                        format!("{}倍役満", y.yakuman)
                    } else {
                        "役満".into()
                    }
                } else {
                    format!("{}翻", y.han)
                };
                c.spawn(row(
                    Node {
                        width: Val::Px(500.0),
                        ..default()
                    },
                    20.0,
                ))
                .with_children(|l| {
                    l.spawn(Node {
                        width: Val::Px(340.0),
                        ..default()
                    })
                    .with_children(|n| k.text(n, y.name.clone(), 38.0, th.text, false));
                    k.text(l, v, 38.0, th.text, true);
                });
            }
            for (name, n) in [("ドラ", sc.dora), ("赤ドラ", sc.aka), ("裏ドラ", sc.ura)] {
                if n > 0 {
                    c.spawn(row(
                        Node {
                            width: Val::Px(500.0),
                            ..default()
                        },
                        20.0,
                    ))
                    .with_children(|l| {
                        l.spawn(Node {
                            width: Val::Px(340.0),
                            ..default()
                        })
                        .with_children(|t| k.text(t, name, 38.0, th.text, false));
                        k.text(l, format!("{n}翻"), 38.0, th.text, true);
                    });
                }
            }
        });
        // 翻符・点数
        q.spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(1200.0),
            top: Val::Px(300.0),
            width: Val::Px(680.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            row_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|c| {
            k.text(c, sc.han_fu_text(), 54.0, th.riichi, true);
            k.text(c, sc.points_text(), 96.0, th.text, true);
            if !r.input.ura_indicators.is_empty() && (r.input.riichi || r.input.double_riichi) {
                c.spawn(row(Node::default(), 6.0)).with_children(|u| {
                    k.text(u, "裏ドラ表示牌", 24.0, th.text_dim, false);
                    for t in &r.input.ura_indicators {
                        k.tile(u, Some(*t), 40.0, false, false);
                    }
                });
            }
        });
    });
}

/// 点数移動のアニメーション（設計書 4.6 演出: 点数移動アニメの有無・速度。設定UIは段階B8）。
#[derive(Resource)]
pub struct ScoreAnim {
    pub from: [i32; 4],
    pub to: [i32; 4],
    pub start: f64,
    /// 秒（0 でアニメーションなし）
    pub duration: f64,
    initialized: bool,
}

impl Default for ScoreAnim {
    fn default() -> Self {
        ScoreAnim {
            from: [0; 4],
            to: [0; 4],
            start: 0.0,
            duration: 1.2,
            initialized: false,
        }
    }
}

const ANIM_STEPS: f64 = 24.0;

impl ScoreAnim {
    fn progress(&self, now: f64) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        ((now - self.start) / self.duration).clamp(0.0, 1.0)
    }
    /// 表示する点数（100点単位で丸める）。
    pub fn shown(&self, now: f64) -> [i32; 4] {
        let p = (self.progress(now) * ANIM_STEPS).floor() / ANIM_STEPS;
        std::array::from_fn(|i| {
            let v = self.from[i] as f64 + (self.to[i] - self.from[i]) as f64 * p;
            ((v / 100.0).round() * 100.0) as i32
        })
    }
    fn step(&self, now: f64) -> u32 {
        (self.progress(now) * ANIM_STEPS) as u32
    }
}

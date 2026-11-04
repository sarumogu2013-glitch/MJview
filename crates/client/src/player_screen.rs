//! 選手用画面（設計書 4.8）。会場の選手向けモニターに全画面表示する。
//!
//! **このモジュールは `PublicView`（公開情報だけの型）しか読まない。** 配信用レイアウト（output.rs）や
//! 牌譜再生の部品は使わない別系統。表示してよい部品だけをここに並べる（ホワイトリスト）。
//! 河・手牌・顔・カメラ映像は出さない。アガリ結果も役・翻符・点数・点数移動だけで、手牌は出さない。

use bevy::prelude::*;
use janscope_core::state::Phase;
use janscope_core::tile::SeatId;
use janscope_core::view::{PublicOutcome, PublicView};

use crate::assets::{Fonts, TileImages};
use crate::net::Table;
use crate::theme::Theme;
use crate::ui_kit::*;

#[derive(Component)]
pub struct PlayerCamera;

#[derive(Component)]
pub struct PlayerRoot;

/// 選手用画面の設定（設計書 4.6「選手用画面」タブ）。
#[derive(Resource, Clone)]
pub struct PlayerScreenConfig {
    /// 画面の下に来る席
    pub bottom: SeatId,
    /// 他家の点数を表示する
    pub show_others: bool,
    /// 点差を表示する
    pub show_diff: bool,
    /// 卓配置（false なら一覧）
    pub table_layout: bool,
}

impl Default for PlayerScreenConfig {
    fn default() -> Self {
        PlayerScreenConfig {
            bottom: SeatId(0),
            show_others: true,
            show_diff: true,
            table_layout: true,
        }
    }
}

#[derive(Resource, Default)]
pub struct PlayerDirty(String);

#[allow(clippy::too_many_arguments)]
pub fn rebuild_player_screen(
    mut commands: Commands,
    table: Res<Table>,
    cfg: Res<PlayerScreenConfig>,
    mut dirty: ResMut<PlayerDirty>,
    roots: Query<Entity, With<PlayerRoot>>,
    cam: Query<Entity, With<PlayerCamera>>,
    fonts: Option<Res<Fonts>>,
    tiles: Option<Res<TileImages>>,
    theme: Res<Theme>,
) {
    let (Some(fonts), Some(tiles), Ok(cam)) = (fonts, tiles, cam.single()) else {
        return;
    };
    let key = format!(
        "{}|{}|{}|{}|{}",
        table.seq, cfg.bottom.0, cfg.show_others, cfg.show_diff, cfg.table_layout
    );
    if key == dirty.0 {
        return;
    }
    dirty.0 = key;
    for e in &roots {
        commands.entity(e).despawn();
    }
    // 配信ロールで兼ねる場合も、全情報からではなく公開情報の型を作って使う
    let view: Option<PublicView> = table.public_view();
    let kit = Kit {
        fonts: &fonts,
        tiles: &tiles,
        theme: &theme,
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
            PlayerRoot,
        ))
        .with_children(|p| match &view {
            None => {
                p.spawn(center(abs(0.0, 0.0, 1920.0, 1080.0)))
                    .with_children(|q| {
                        kit.text(q, "接続を待っています", 56.0, theme.text_dim, false)
                    });
            }
            Some(v) => draw(p, &kit, v, &cfg),
        });
}

fn draw(p: &mut Kids, k: &Kit, v: &PublicView, cfg: &PlayerScreenConfig) {
    let th = k.theme;
    // 局情報（大きく）
    p.spawn((
        row(center(abs(0.0, 0.0, 1920.0, 150.0)), 60.0),
        BackgroundColor(th.panel_alt),
    ))
    .with_children(|q| {
        let label = match v.phase {
            Phase::Setup => "対局前".to_string(),
            Phase::GameOver => "終局".to_string(),
            _ => format!("{}{}局", v.round_wind.name_ja(), v.round_number),
        };
        k.text(q, label, 92.0, th.text, true);
        k.text(q, format!("{}本場", v.honba), 72.0, th.text, false);
        k.text(
            q,
            format!(
                "供託 {}",
                janscope_core::scoring::fmt_pts(v.kyotaku as i32 * 1000)
            ),
            64.0,
            th.text_dim,
            false,
        );
        if v.all_last && v.phase != Phase::GameOver {
            k.text(q, "オーラス", 64.0, th.warn, true);
        }
    });
    // 中央: ドラ・残り・サイコロ
    p.spawn((abs(660.0, 380.0, 600.0, 400.0), BackgroundColor(th.panel)))
        .with_children(|q| {
            q.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceEvenly,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            })
            .with_children(|c| {
                c.spawn(row(Node::default(), 12.0)).with_children(|r| {
                    k.text(r, "ドラ", 48.0, th.text_dim, false);
                    for t in &v.dora_indicators {
                        k.tile(r, Some(*t), 70.0, false, false);
                    }
                });
                c.spawn(row(Node::default(), 20.0)).with_children(|r| {
                    k.text(r, "残り", 48.0, th.text_dim, false);
                    let low = v.phase == Phase::InHand && v.wall_remaining <= 4;
                    let txt = if v.phase == Phase::InHand {
                        v.wall_remaining.to_string()
                    } else {
                        "—".into()
                    };
                    k.text(r, txt, 96.0, if low { th.warn } else { th.text }, true);
                });
                c.spawn(row(Node::default(), 12.0)).with_children(|r| {
                    for &(a, b) in &v.dice {
                        k.die(r, a, 60.0, false);
                        k.die(r, b, 60.0, false);
                    }
                });
            });
        });
    // 4人（卓配置: 下・右・上・左 = 自分・下家・対面・上家）
    let pos = [
        (660.0, 800.0),
        (1300.0, 480.0),
        (660.0, 170.0),
        (20.0, 480.0),
    ];
    let me = cfg.bottom;
    let top = v.seats.iter().map(|s| s.score).max().unwrap_or(0);
    for k_i in 0..4u8 {
        let seat = SeatId((me.0 + k_i) % 4);
        let s = &v.seats[seat.index()];
        let (x, y) = if cfg.table_layout {
            pos[k_i as usize]
        } else {
            (360.0, 170.0 + k_i as f32 * 200.0)
        };
        let w = if cfg.table_layout { 600.0 } else { 1200.0 };
        p.spawn((
            center(abs(x, y, w, 200.0)),
            BackgroundColor(if s.riichi {
                th.riichi.with_alpha(0.25)
            } else {
                th.panel_alt
            }),
        ))
        .with_children(|c| {
            c.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|col| {
                col.spawn(row(Node::default(), 16.0)).with_children(|r| {
                    k.text(r, s.wind.name_ja(), 52.0, th.text, true);
                    if s.is_dealer {
                        k.text(r, "親", 44.0, th.dealer, true);
                    }
                    k.text(r, s.name.clone(), 52.0, th.text, true);
                    if s.riichi {
                        k.text(r, "リーチ", 40.0, th.riichi, true);
                    }
                });
                if cfg.show_others || seat == me {
                    col.spawn(row(Node::default(), 20.0)).with_children(|r| {
                        k.text(
                            r,
                            janscope_core::scoring::fmt_pts(s.score),
                            72.0,
                            th.text,
                            true,
                        );
                        if cfg.show_diff && s.score != top {
                            k.text(r, delta_text(s.score - top), 40.0, th.text_dim, false);
                        }
                        let d = v.last_deltas[seat.index()];
                        if d != 0 {
                            k.text(
                                r,
                                delta_text(d),
                                44.0,
                                if d > 0 { th.plus } else { th.minus },
                                true,
                            );
                        }
                    });
                }
            });
        });
    }
    // アガリ結果（手牌は出さない）
    if let Some(r) = &v.last_result {
        if let PublicOutcome::Win(ws) = &r.outcome {
            if v.phase != Phase::InHand {
                p.spawn((abs(1300.0, 700.0, 600.0, 360.0), BackgroundColor(th.panel)))
                    .with_children(|q| {
                        q.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(16.0)),
                            row_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|c| {
                            for w in ws {
                                k.text(
                                    c,
                                    format!(
                                        "{}  {}",
                                        v.seats[w.winner.index()].name,
                                        w.han_fu_text
                                    ),
                                    36.0,
                                    th.riichi,
                                    true,
                                );
                                k.text(c, w.points_text.clone(), 48.0, th.text, true);
                                let names: Vec<String> =
                                    w.yaku.iter().map(|y| y.name.clone()).collect();
                                k.text(c, names.join("・"), 26.0, th.text_dim, false);
                            }
                        });
                    });
            }
        }
    }
}

//! 出力画面（bevy_ui）の組み立ての小道具。座標は 1920×1080 の固定レイアウトの画素。

use bevy::prelude::*;
use bevy::text::{FontSize, FontSource};
use janscope_core::tile::Tile;

use crate::assets::{Fonts, TileImages};
use crate::theme::Theme;

pub type Kids<'a> = ChildSpawnerCommands<'a>;

pub struct Kit<'a> {
    pub fonts: &'a Fonts,
    pub tiles: &'a TileImages,
    pub theme: &'a Theme,
}

/// 絶対位置の箱。
pub fn abs(x: f32, y: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x),
        top: Val::Px(y),
        width: Val::Px(w),
        height: Val::Px(h),
        ..default()
    }
}

/// 中身を縦横中央にそろえる箱。
pub fn center(mut n: Node) -> Node {
    n.justify_content = JustifyContent::Center;
    n.align_items = AlignItems::Center;
    n
}

pub fn row(mut n: Node, gap: f32) -> Node {
    n.flex_direction = FlexDirection::Row;
    n.align_items = AlignItems::Center;
    n.column_gap = Val::Px(gap);
    n
}

impl Kit<'_> {
    pub fn font(&self, size: f32, bold: bool) -> TextFont {
        TextFont {
            font: FontSource::Handle(if bold {
                self.fonts.bold.clone()
            } else {
                self.fonts.regular.clone()
            }),
            font_size: FontSize::Px(size),
            ..default()
        }
    }

    pub fn text(&self, p: &mut Kids, s: impl Into<String>, size: f32, color: Color, bold: bool) {
        p.spawn((Text::new(s.into()), self.font(size, bold), TextColor(color)));
    }

    /// 牌1枚（幅 w、高さは 4:3）。sideways なら横向き（幅と高さを入れ替えて回す）。dim なら暗く。
    pub fn tile(&self, p: &mut Kids, t: Option<Tile>, w: f32, sideways: bool, dim: bool) {
        let h = w * 4.0 / 3.0;
        let img = match t {
            Some(t) => self.tiles.get(t),
            None => self.tiles.back.clone(),
        };
        let color = if dim {
            Color::srgb(0.55, 0.55, 0.55)
        } else {
            Color::WHITE
        };
        if sideways {
            // 横向き: 外枠は横長、中の画像を90°回す
            p.spawn(Node {
                width: Val::Px(h),
                height: Val::Px(w),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|q| {
                q.spawn((
                    // 回転は自分の中心が軸なので、外枠の中央に置いてから回す
                    Node {
                        width: Val::Px(w),
                        height: Val::Px(h),
                        position_type: PositionType::Absolute,
                        left: Val::Px((h - w) / 2.0),
                        top: Val::Px((w - h) / 2.0),
                        ..default()
                    },
                    ImageNode {
                        image: img,
                        color,
                        ..default()
                    },
                    UiTransform {
                        rotation: Rot2::degrees(90.0),
                        ..default()
                    },
                ));
            });
        } else {
            p.spawn((
                Node {
                    width: Val::Px(w),
                    height: Val::Px(h),
                    ..default()
                },
                ImageNode {
                    image: img,
                    color,
                    ..default()
                },
            ));
        }
    }

    /// サイコロ1個（目を丸で描く）。
    pub fn die(&self, p: &mut Kids, n: u8, size: f32, faded: bool) {
        let fg = if n == 1 || n == 4 {
            Color::srgb(0.85, 0.15, 0.15)
        } else {
            Color::srgb(0.1, 0.1, 0.1)
        };
        let alpha = if faded { 0.35 } else { 1.0 };
        let pips: &[(f32, f32)] = match n {
            1 => &[(0.5, 0.5)],
            2 => &[(0.28, 0.28), (0.72, 0.72)],
            3 => &[(0.25, 0.25), (0.5, 0.5), (0.75, 0.75)],
            4 => &[(0.28, 0.28), (0.72, 0.28), (0.28, 0.72), (0.72, 0.72)],
            5 => &[
                (0.25, 0.25),
                (0.75, 0.25),
                (0.5, 0.5),
                (0.25, 0.75),
                (0.75, 0.75),
            ],
            _ => &[
                (0.28, 0.22),
                (0.72, 0.22),
                (0.28, 0.5),
                (0.72, 0.5),
                (0.28, 0.78),
                (0.72, 0.78),
            ],
        };
        let r = if n == 1 { size * 0.24 } else { size * 0.15 };
        p.spawn((
            Node {
                width: Val::Px(size),
                height: Val::Px(size),
                border_radius: BorderRadius::all(Val::Px(size * 0.15)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.97, 0.97, 0.95, alpha)),
        ))
        .with_children(|q| {
            for &(x, y) in pips {
                q.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x * size - r / 2.0),
                        top: Val::Px(y * size - r / 2.0),
                        width: Val::Px(r),
                        height: Val::Px(r),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        ..default()
                    },
                    BackgroundColor(fg.with_alpha(alpha)),
                ));
            }
        });
    }
}

/// 「+8,000」「−3,900」
pub fn delta_text(d: i32) -> String {
    let s = janscope_core::scoring::fmt_pts(d.abs());
    if d > 0 {
        format!("+{s}")
    } else {
        format!("−{s}")
    }
}

//! 牌譜再生（設計書 4.9）。普通の麻雀ゲームのような卓画面で、牌譜を1手ずつ再生する。
//!
//! 安全: このモジュールは出力ウィンドウ（配信）と操作ウィンドウからだけ使う。選手用画面（player_screen.rs）からは参照しない。

use std::path::PathBuf;

use bevy::prelude::*;
use janscope_core::paifu::{Board, Paifu, PaifuEvent, parse_tenhou6};
use janscope_core::tile::SeatId;
use serde::{Deserialize, Serialize};

use crate::output::draw_meld;
use crate::ui_kit::*;

/// 名場面のしおり。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub label: String,
    pub file: String,
    pub hand: usize,
    pub step: usize,
}

#[derive(Resource)]
pub struct Replay {
    pub paifu: Option<Paifu>,
    pub file: String,
    pub hand: usize,
    pub step: usize,
    pub playing: bool,
    /// 再生速度（0.5〜4倍）
    pub speed: f64,
    next_at: f64,
    /// 下に表示する席
    pub viewpoint: SeatId,
    /// 全員の手牌を表示する（false なら視点の選手だけ）
    pub open_all: bool,
    pub bookmarks: Vec<Bookmark>,
    pub msg: Option<String>,
}

impl Default for Replay {
    fn default() -> Self {
        Replay {
            paifu: None,
            file: String::new(),
            hand: 0,
            step: 0,
            playing: false,
            speed: 1.0,
            next_at: 0.0,
            viewpoint: SeatId(0),
            open_all: true,
            bookmarks: load_bookmarks(),
            msg: None,
        }
    }
}

fn bookmarks_path() -> PathBuf {
    crate::config::config_path().with_file_name("janscope-bookmarks.json")
}

fn load_bookmarks() -> Vec<Bookmark> {
    std::fs::read_to_string(bookmarks_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

impl Replay {
    pub fn open(&mut self, path: &str) {
        match std::fs::read_to_string(path.trim())
            .map_err(|e| e.to_string())
            .and_then(|s| parse_tenhou6(&s).map_err(|e| e.to_string()))
        {
            Ok(p) => {
                self.msg = Some(format!("{}局を読み込みました", p.hands.len()));
                self.paifu = Some(p);
                self.file = path.trim().to_string();
                self.hand = 0;
                self.step = 0;
                self.playing = false;
            }
            Err(e) => self.msg = Some(format!("読み込めません: {e}")),
        }
    }

    pub fn events_len(&self) -> usize {
        self.paifu
            .as_ref()
            .and_then(|p| p.hands.get(self.hand))
            .map(|h| h.events.len())
            .unwrap_or(0)
    }

    pub fn board(&self) -> Option<Board> {
        self.paifu
            .as_ref()
            .and_then(|p| p.hands.get(self.hand))
            .map(|h| h.board_at(self.step))
    }

    pub fn step_by(&mut self, d: i64) {
        let n = self.events_len() as i64;
        self.step = (self.step as i64 + d).clamp(0, n) as usize;
    }

    pub fn set_hand(&mut self, h: usize) {
        let n = self.paifu.as_ref().map(|p| p.hands.len()).unwrap_or(0);
        if n > 0 {
            self.hand = h.min(n - 1);
            self.step = 0;
        }
    }

    pub fn add_bookmark(&mut self) {
        let label = self
            .paifu
            .as_ref()
            .and_then(|p| p.hands.get(self.hand))
            .map(|h| format!("{} {}手目", h.label(), self.step))
            .unwrap_or_default();
        self.bookmarks.push(Bookmark {
            label,
            file: self.file.clone(),
            hand: self.hand,
            step: self.step,
        });
        if let Err(e) = std::fs::write(
            bookmarks_path(),
            serde_json::to_string_pretty(&self.bookmarks).unwrap(),
        ) {
            self.msg = Some(format!("しおりを保存できません: {e}"));
        }
    }

    pub fn jump(&mut self, b: &Bookmark) {
        if self.file != b.file || self.paifu.is_none() {
            self.open(&b.file);
        }
        self.set_hand(b.hand);
        self.step = b.step.min(self.events_len());
    }
}

/// 再生中なら手順を進める。新ドラなど一瞬の出来事は待たずに進む。
pub fn tick_replay(time: Res<Time>, mut r: ResMut<Replay>) {
    if !r.playing {
        return;
    }
    let now = time.elapsed_secs_f64();
    if now < r.next_at {
        return;
    }
    let n = r.events_len();
    if r.step >= n {
        r.playing = false;
        return;
    }
    r.step += 1;
    let quick = r
        .paifu
        .as_ref()
        .and_then(|p| p.hands.get(r.hand))
        .and_then(|h| h.events.get(r.step))
        .is_some_and(|e| matches!(e, PaifuEvent::NewDora { .. }));
    let base = if quick { 0.15 } else { 0.9 };
    r.next_at = now + base / r.speed.clamp(0.25, 8.0);
}

/// 牌譜再生の画面（1920×1080）。視点の選手が下、対面が上、上家が左、下家が右。
pub fn draw_replay(p: &mut Kids, k: &Kit, r: &Replay) {
    let th = k.theme;
    let (Some(pf), Some(b)) = (r.paifu.as_ref(), r.board()) else {
        p.spawn(center(abs(0.0, 0.0, 1920.0, 1080.0)))
            .with_children(|c| k.text(c, "牌譜が読み込まれていません", 48.0, th.text_dim, false));
        return;
    };
    let h = &pf.hands[r.hand];
    p.spawn((
        abs(0.0, 0.0, 1920.0, 1080.0),
        BackgroundColor(Color::srgb(0.07, 0.20, 0.13)),
    ));
    // 中央の情報
    p.spawn((
        center(abs(660.0, 380.0, 600.0, 320.0)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
    ))
    .with_children(|c| {
        c.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|col| {
            k.text(
                col,
                format!("{}  供託{}", h.label(), b.kyotaku),
                36.0,
                th.text,
                true,
            );
            col.spawn(row(Node::default(), 6.0)).with_children(|rw| {
                k.text(rw, "ドラ", 26.0, th.text_dim, false);
                for t in &b.dora_indicators {
                    k.tile(rw, Some(*t), 40.0, false, false);
                }
                k.text(
                    rw,
                    format!("  残り {}", b.wall_remaining.max(0)),
                    28.0,
                    th.text,
                    true,
                );
            });
            for i in 0..4u8 {
                let s = SeatId((r.viewpoint.0 + i) % 4);
                let wind =
                    janscope_core::tile::Wind::from_index(s.distance_from(h.dealer)).name_ja();
                let mark = if b.turn == Some(s) { "▶ " } else { "  " };
                let rich = if b.seats[s.index()].riichi {
                    " リーチ"
                } else {
                    ""
                };
                k.text(
                    col,
                    format!(
                        "{mark}{wind} {}  {}{rich}",
                        pf.players[s.index()],
                        janscope_core::scoring::fmt_pts(b.seats[s.index()].score)
                    ),
                    24.0,
                    if b.turn == Some(s) {
                        th.riichi
                    } else {
                        th.text
                    },
                    false,
                );
            }
        });
    });
    // 4人（下・右・上・左）。1人分を 1000×300 の帯として作り、回して置く
    let places = [
        (460.0, 780.0, 0.0),
        (1310.0, 390.0, -90.0),
        (460.0, 0.0, 180.0),
        (-390.0, 390.0, 90.0),
    ];
    for (i, (x, y, rot)) in places.iter().enumerate() {
        let seat = SeatId((r.viewpoint.0 + i as u8) % 4);
        let open = r.open_all || seat == r.viewpoint || b.result.is_some();
        p.spawn((
            abs(*x, *y, 1000.0, 300.0),
            UiTransform {
                rotation: Rot2::degrees(*rot),
                ..default()
            },
        ))
        .with_children(|strip| {
            draw_seat_strip(strip, k, &b, seat, open);
        });
    }
    // 直前の出来事（リーチ・ポン・ロンなど）を大きく
    if let Some((seat, label)) = b.action.as_ref().filter(|_| b.result.is_none()) {
        let i = seat.distance_from(r.viewpoint) as usize;
        let (x, y) = [
            (860.0, 700.0),
            (1300.0, 470.0),
            (860.0, 300.0),
            (420.0, 470.0),
        ][i];
        p.spawn((
            center(abs(x, y, 200.0, 90.0)),
            BackgroundColor(Color::srgb(0.85, 0.20, 0.15)),
            GlobalZIndex(20),
        ))
        .with_children(|c| k.text(c, label.clone(), 52.0, Color::WHITE, true));
    }
    // 局の結果
    if let Some(res) = &b.result {
        p.spawn((
            abs(560.0, 300.0, 800.0, 480.0),
            BackgroundColor(Color::srgb(0.03, 0.04, 0.06)),
            GlobalZIndex(30),
        ))
        .with_children(|c| {
            c.spawn(Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(24.0)),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|col| {
                k.text(col, res.kind.clone(), 44.0, th.riichi, true);
                for w in &res.wins {
                    let how = match w.from {
                        Some(f) => format!("ロン（{}）", pf.players[f.index()]),
                        None => "ツモ".into(),
                    };
                    k.text(
                        col,
                        format!("{}  {how}", pf.players[w.winner.index()]),
                        34.0,
                        th.text,
                        true,
                    );
                    k.text(
                        col,
                        w.points_text.trim_end_matches('∀').to_string(),
                        32.0,
                        th.text,
                        false,
                    );
                    k.text(col, w.yaku.join("・"), 24.0, th.text_dim, false);
                }
                let d: Vec<String> = (0..4)
                    .filter(|i| res.deltas[*i] != 0)
                    .map(|i| format!("{} {}", pf.players[i], delta_text(res.deltas[i])))
                    .collect();
                k.text(col, d.join("  "), 26.0, th.text, false);
                // 裏ドラはアガった人が立直しているときだけ
                if !h.ura_indicators.is_empty()
                    && res.wins.iter().any(|w| b.seats[w.winner.index()].riichi)
                {
                    col.spawn(row(Node::default(), 6.0)).with_children(|rw| {
                        k.text(rw, "裏ドラ", 24.0, th.text_dim, false);
                        for t in &h.ura_indicators {
                            k.tile(rw, Some(*t), 36.0, false, false);
                        }
                    });
                }
            });
        });
    }
}

fn draw_seat_strip(p: &mut Kids, k: &Kit, b: &Board, seat: SeatId, open: bool) {
    let s = &b.seats[seat.index()];
    // 河（6枚ずつ折り返し、最大4段。5段目以降は4段目に続ける）
    p.spawn(Node {
        position_type: PositionType::Absolute,
        left: Val::Px(380.0),
        top: Val::Px(0.0),
        width: Val::Px(300.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(1.0),
        ..default()
    })
    .with_children(|col| {
        let rows: Vec<&[janscope_core::paifu::RiverTile]> = {
            let mut v: Vec<&[_]> = s.river.chunks(6).take(3).collect();
            if s.river.len() > 18 {
                v.push(&s.river[18..]);
            }
            v
        };
        for rw in rows {
            col.spawn(row(Node::default(), 1.0)).with_children(|r| {
                for t in rw {
                    k.tile(r, Some(t.tile), 38.0, t.riichi, t.called);
                }
            });
        }
    });
    // 手牌（ツモ牌は少し離す）と副露（右端）
    p.spawn(Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        bottom: Val::Px(0.0),
        width: Val::Px(1000.0),
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::FlexEnd,
        ..default()
    })
    .with_children(|r| {
        for t in &s.hand {
            k.tile(r, open.then_some(*t), 50.0, false, false);
        }
        if let Some(d) = s.drawn {
            r.spawn(Node {
                width: Val::Px(14.0),
                ..default()
            });
            k.tile(r, open.then_some(d), 50.0, false, false);
        }
        r.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        for m in &s.melds {
            draw_meld(r, k, m, 40.0);
            r.spawn(Node {
                width: Val::Px(8.0),
                ..default()
            });
        }
    });
}

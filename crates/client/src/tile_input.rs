//! 牌の手入力（手牌・河）。テンキーだけで入力できるようにする。
//!
//! - `/` `*` で牌のパレット（萬子・筒子・索子・字牌）を切り替え、`1`〜`9` で牌を入れる（`0` は赤5）
//! - 全手動の流れ: 親から順に配牌を登録 → 親の打牌 → 鳴きの有無 → 下家のツモ → 打牌か槓 → …
//! - 次に何を入れるかは卓状態（`TableState::turn`）から決めるので、取り消し・別の端末からの操作とも食い違わない
//! - 押せるキーと意味は、操作画面のテンキーの図にいつも出しておく

use bevy::prelude::*;
use bevy_egui::egui;
use janscope_core::hands;
use janscope_core::scoring::{MeldKind, WinInput};
use janscope_core::state::{AbortiveKind, Action, DrawKind, KanKind, Phase, TableState, Turn};
use janscope_core::tile::{SeatId, Suit, Tile};
use serde::{Deserialize, Serialize};

use crate::net::Net;

/// 操作の送り先（テストでは卓状態に直接当てる）。
pub trait Sink {
    fn act(&self, a: Action) -> u64;
    fn undo(&self) -> u64;
}

impl Sink for Net {
    fn act(&self, a: Action) -> u64 {
        Net::act(self, a)
    }
    fn undo(&self) -> u64 {
        Net::undo(self)
    }
}

/// 手牌・河の入れ方（設定に保存）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileMode {
    /// 手牌・河を扱わない（点数・リーチ・残り牌数だけ）
    Off,
    /// 最初から手で入れる（テンキーの流れに沿って）
    #[default]
    Manual,
    /// 自動判別（牌の自動読み取り）の結果を手で直す
    AutoFix,
}

impl TileMode {
    pub fn name_ja(self) -> &'static str {
        match self {
            TileMode::Off => "使わない",
            TileMode::Manual => "最初から手動で入力",
            TileMode::AutoFix => "自動判別 → 手動で修正",
        }
    }
}

/// テンキーのキー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumKey {
    Digit(u8),
    Div,
    Mul,
    Sub,
    Add,
    Enter,
    Dot,
}

impl NumKey {
    pub fn from_keys(keys: &ButtonInput<KeyCode>) -> Vec<NumKey> {
        use KeyCode::*;
        let map = [
            (Numpad0, NumKey::Digit(0)),
            (Numpad1, NumKey::Digit(1)),
            (Numpad2, NumKey::Digit(2)),
            (Numpad3, NumKey::Digit(3)),
            (Numpad4, NumKey::Digit(4)),
            (Numpad5, NumKey::Digit(5)),
            (Numpad6, NumKey::Digit(6)),
            (Numpad7, NumKey::Digit(7)),
            (Numpad8, NumKey::Digit(8)),
            (Numpad9, NumKey::Digit(9)),
            (NumpadDivide, NumKey::Div),
            (NumpadMultiply, NumKey::Mul),
            (NumpadSubtract, NumKey::Sub),
            (NumpadAdd, NumKey::Add),
            (NumpadEnter, NumKey::Enter),
            (NumpadDecimal, NumKey::Dot),
            (NumpadComma, NumKey::Dot),
        ];
        map.iter()
            .filter(|(k, _)| keys.just_pressed(*k))
            .map(|(_, n)| *n)
            .collect()
    }
}

/// 鳴き・ロンの種類（鳴きの選択中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Pon,
    Kan,
    Ron,
}

impl CallKind {
    fn name_ja(self) -> &'static str {
        match self {
            CallKind::Pon => "ポン",
            CallKind::Kan => "カン",
            CallKind::Ron => "ロン",
        }
    }
}

/// 打牌・鳴きの途中の選択。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Sub {
    #[default]
    None,
    /// 打牌の番の「その他」（カン・ツモアガリ・九種九牌）
    Menu,
    /// 暗槓・加槓する牌を選ぶ
    KanTile,
    /// 槓のあとの新しいドラ表示牌
    KanDora {
        seat: SeatId,
        kind: KanKind,
        tiles: Vec<Tile>,
    },
    /// 誰が鳴く・ロンするか
    CallWho(CallKind),
    /// 鳴きの牌の組み合わせを選ぶ
    CallChoose {
        seat: SeatId,
        kind: MeldKind,
        options: Vec<Vec<Tile>>,
    },
    /// アガリを手牌から入れた（プレビュー待ち → Enter で放送に出す）
    WinPending,
}

/// 今の入力の段階（卓状態から決まる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 手牌を扱わない・局の途中でない
    Off,
    /// 配牌の登録（need 枚。親は14枚）
    Haipai {
        seat: SeatId,
        need: usize,
    },
    /// 手牌の入れ直し（修正）
    Fix {
        seat: SeatId,
        need: usize,
    },
    Discard {
        seat: SeatId,
    },
    /// 打牌の直後（鳴き・ロンの有無）
    Call {
        discarder: SeatId,
    },
    Tsumo {
        seat: SeatId,
        rinshan: bool,
    },
    /// 残り牌数が0で鳴きもない（流局。テンパイを選ぶ）
    Exhausted,
}

/// 操作画面に返すこと（アガリ入力の欄を埋める、など）。
pub enum Outcome {
    /// アガリ入力の欄をこの内容で埋めてプレビューする
    Win(WinInput),
    /// プレビュー済みのアガリを放送に出す
    ConfirmWin,
}

#[derive(Default)]
pub struct TileInput {
    /// 0 萬子・1 筒子・2 索子・3 字牌
    pub palette: usize,
    /// 配牌・入れ直しで入れている牌
    pub buf: Vec<Tile>,
    /// 次の打牌をリーチ宣言にする
    pub riichi: bool,
    pub sub: Sub,
    /// 鳴きなしと決めた打牌（打った席・その時の河の枚数）
    pub passed: Option<(SeatId, usize)>,
    /// 手牌を入れ直している席
    pub fix_seat: Option<SeatId>,
    /// 流局のテンパイ
    pub tenpai: [bool; 4],
    pub msg: Option<String>,
    /// 段階が変わったら途中の選択を捨てるため
    last_step: Option<Step>,
}

const PALETTES: [&str; 4] = ["萬子", "筒子", "索子", "字牌"];
const HONORS: [&str; 7] = ["東", "南", "西", "北", "白", "發", "中"];

fn seat_name(s: &TableState, seat: SeatId) -> String {
    let n = &s.seats[seat.index()].player.name;
    let w = s.seat_wind(seat).name_ja();
    if n.is_empty() {
        format!("{w}家 席{}", seat.label())
    } else {
        format!("{w}家 {n}")
    }
}

fn tiles_text(v: &[Tile]) -> String {
    v.iter().map(|t| t.name_ja()).collect::<Vec<_>>().join(" ")
}

impl TileInput {
    pub fn step(&self, s: &TableState, mode: TileMode) -> Step {
        if mode == TileMode::Off || s.phase != Phase::InHand {
            return Step::Off;
        }
        if let Some(seat) = self.fix_seat {
            let st = &s.seats[seat.index()];
            let full = st.drawn.is_some() || st.tile_count() == 14;
            let need = if full { 14 } else { 13 } - 3 * st.melds.len();
            return Step::Fix { seat, need };
        }
        if s.turn == Turn::None {
            if let Some(seat) = hands::next_haipai(s) {
                let need = if seat == s.dealer() { 14 } else { 13 };
                return Step::Haipai { seat, need };
            }
        }
        match s.turn {
            Turn::Discard { seat } => Step::Discard { seat },
            Turn::AfterDiscard { seat } => {
                if self.passed == Some((seat, s.seats[seat.index()].river.len())) {
                    if s.wall_remaining == 0 {
                        Step::Exhausted
                    } else {
                        Step::Tsumo {
                            seat: seat.next(),
                            rinshan: false,
                        }
                    }
                } else {
                    Step::Call { discarder: seat }
                }
            }
            Turn::Tsumo { seat, rinshan } => Step::Tsumo { seat, rinshan },
            Turn::None => Step::Off,
        }
    }

    /// 段階が変わったら、途中の選択を捨てる。
    pub fn sync(&mut self, step: Step) {
        if self.last_step != Some(step) {
            let keep_buf = matches!(
                (self.last_step, step),
                (Some(Step::Fix { seat: a, .. }), Step::Fix { seat: b, .. }) if a == b
            );
            if !keep_buf {
                self.buf.clear();
            }
            if self.sub != Sub::WinPending {
                self.sub = Sub::None;
            }
            if !matches!(step, Step::Discard { .. }) {
                self.riichi = false;
            }
            if step != Step::Exhausted {
                self.tenpai = [false; 4];
            }
            self.last_step = Some(step);
        }
    }

    fn tile_for(&self, d: u8) -> Option<Tile> {
        match (self.palette, d) {
            (3, 1..=7) => Some(Tile::from_suit_number(Suit::Honor, d)),
            (3, _) => None,
            (p, 0) => Some(Tile::red_five([Suit::Man, Suit::Pin, Suit::Sou][p])),
            (p, d) => Some(Tile::from_suit_number(
                [Suit::Man, Suit::Pin, Suit::Sou][p],
                d,
            )),
        }
    }

    /// 段階で牌を入れるか（パレットのキーが効くか）。
    fn wants_tile(&self, step: Step) -> bool {
        match step {
            Step::Haipai { .. } | Step::Fix { .. } | Step::Tsumo { .. } => true,
            Step::Discard { .. } => {
                matches!(self.sub, Sub::None | Sub::KanTile | Sub::KanDora { .. })
            }
            Step::Call { .. } => matches!(self.sub, Sub::KanDora { .. }),
            _ => false,
        }
    }

    /// テンキーが押された。
    pub fn key(
        &mut self,
        k: NumKey,
        s: &TableState,
        mode: TileMode,
        net: &dyn Sink,
    ) -> Option<Outcome> {
        let step = self.step(s, mode);
        self.sync(step);
        if step == Step::Off {
            return None;
        }
        self.msg = None;
        if self.sub == Sub::WinPending {
            match k {
                NumKey::Enter => return Some(Outcome::ConfirmWin),
                NumKey::Sub => self.sub = Sub::None,
                _ => {}
            }
            return None;
        }
        if self.wants_tile(step) {
            match k {
                NumKey::Div => {
                    self.palette = (self.palette + 3) % 4;
                    return None;
                }
                NumKey::Mul => {
                    self.palette = (self.palette + 1) % 4;
                    return None;
                }
                NumKey::Digit(d) => {
                    if let Some(t) = self.tile_for(d) {
                        return self.tile(t, s, mode, net);
                    }
                    return None;
                }
                _ => {}
            }
        }
        match step {
            Step::Haipai { seat, need } | Step::Fix { seat, need } => match k {
                NumKey::Sub => {
                    if self.buf.pop().is_none() && matches!(step, Step::Fix { .. }) {
                        self.fix_seat = None;
                    }
                }
                NumKey::Enter => self.finish_hand(seat, need, s, net),
                _ => {}
            },
            Step::Discard { seat } => match (&self.sub, k) {
                (Sub::None, NumKey::Dot) => match s.seats[seat.index()].drawn {
                    Some(t) => self.discard(seat, t, net),
                    None => {
                        self.msg = Some("ツモ牌がありません（鳴いたあとは手牌から選びます）".into())
                    }
                },
                (Sub::None, NumKey::Add) => self.riichi = !self.riichi,
                (Sub::None, NumKey::Enter) => self.sub = Sub::Menu,
                (Sub::None, NumKey::Sub) => {
                    net.undo();
                }
                (Sub::Menu, NumKey::Digit(1)) => self.sub = Sub::KanTile,
                (Sub::Menu, NumKey::Digit(2)) => {
                    self.sub = Sub::None;
                    return self.win(s, seat, None);
                }
                (Sub::Menu, NumKey::Digit(3)) => {
                    self.sub = Sub::None;
                    net.act(Action::Draw {
                        tenpai: vec![],
                        kind: DrawKind::Abortive(AbortiveKind::KyuushuKyuuhai),
                    });
                }
                (Sub::KanDora { seat, kind, tiles }, NumKey::Enter) => {
                    net.act(Action::Kan {
                        seat: *seat,
                        kind: *kind,
                        new_dora_indicator: None,
                        tiles: tiles.clone(),
                    });
                    self.sub = Sub::None;
                }
                (_, NumKey::Sub) => self.sub = Sub::None,
                _ => {}
            },
            Step::Call { discarder } => {
                let called = s.seats[discarder.index()].river.last().map(|r| r.tile);
                match (self.sub.clone(), k) {
                    (Sub::None, NumKey::Enter) => {
                        self.passed = Some((discarder, s.seats[discarder.index()].river.len()));
                    }
                    (Sub::None, NumKey::Sub) => {
                        net.undo();
                    }
                    (Sub::None, NumKey::Digit(1)) => {
                        let seat = discarder.next();
                        let opts = called
                            .map(|c| hands::chi_options(&s.seats[seat.index()].hand, c))
                            .unwrap_or_default();
                        self.choose_call(seat, MeldKind::Chi, opts, s, net);
                    }
                    (Sub::None, NumKey::Digit(2)) => self.sub = Sub::CallWho(CallKind::Pon),
                    (Sub::None, NumKey::Digit(3)) => self.sub = Sub::CallWho(CallKind::Kan),
                    (Sub::None, NumKey::Digit(4)) => self.sub = Sub::CallWho(CallKind::Ron),
                    (Sub::CallWho(kind), NumKey::Digit(n @ 1..=3)) => {
                        let seat = SeatId((discarder.0 + n) % 4);
                        let hand = &s.seats[seat.index()].hand;
                        let Some(c) = called else { return None };
                        match kind {
                            CallKind::Pon => {
                                let opts = hands::pon_options(hand, c);
                                self.choose_call(seat, MeldKind::Pon, opts, s, net);
                            }
                            CallKind::Kan => match hands::minkan_tiles(hand, c) {
                                Some(tiles) => {
                                    self.sub = Sub::KanDora {
                                        seat,
                                        kind: KanKind::Minkan,
                                        tiles,
                                    }
                                }
                                None => {
                                    self.msg = Some(format!(
                                        "{}の手牌に{}が3枚ありません",
                                        seat_name(s, seat),
                                        c.name_ja()
                                    ));
                                    self.sub = Sub::None;
                                }
                            },
                            CallKind::Ron => {
                                self.sub = Sub::None;
                                return self.win(s, seat, Some(discarder));
                            }
                        }
                    }
                    (
                        Sub::CallChoose {
                            seat,
                            kind,
                            options,
                        },
                        NumKey::Digit(n),
                    ) => {
                        if let Some(o) = options.get((n as usize).wrapping_sub(1)) {
                            net.act(Action::Call {
                                seat,
                                kind,
                                tiles: o.clone(),
                            });
                            self.sub = Sub::None;
                        }
                    }
                    (Sub::KanDora { seat, kind, tiles }, NumKey::Enter) => {
                        net.act(Action::Kan {
                            seat,
                            kind,
                            new_dora_indicator: None,
                            tiles,
                        });
                        self.sub = Sub::None;
                    }
                    (_, NumKey::Sub) => self.sub = Sub::None,
                    _ => {}
                }
            }
            Step::Tsumo { .. } => {
                if k == NumKey::Sub {
                    if self.passed.is_some() && matches!(s.turn, Turn::AfterDiscard { .. }) {
                        self.passed = None;
                    } else {
                        net.undo();
                    }
                }
            }
            Step::Exhausted => match k {
                NumKey::Digit(n @ 1..=4) => {
                    // 1〜4 = 東南西北（自風）
                    let seat = SeatId((s.dealer().0 + n - 1) % 4);
                    self.tenpai[seat.index()] = !self.tenpai[seat.index()];
                }
                NumKey::Enter => {
                    let tenpai = SeatId::ALL
                        .into_iter()
                        .filter(|x| self.tenpai[x.index()])
                        .collect();
                    net.act(Action::Draw {
                        tenpai,
                        kind: DrawKind::Exhaustive,
                    });
                }
                NumKey::Sub => self.passed = None,
                _ => {}
            },
            Step::Off => {}
        }
        None
    }

    /// 牌のパレット（テンキーの数字・画面のクリック）で牌が選ばれた。
    pub fn tile(
        &mut self,
        t: Tile,
        s: &TableState,
        mode: TileMode,
        net: &dyn Sink,
    ) -> Option<Outcome> {
        let step = self.step(s, mode);
        self.sync(step);
        self.msg = None;
        match step {
            Step::Haipai { seat, need } | Step::Fix { seat, need } => {
                if self.buf.len() < need {
                    self.buf.push(t);
                }
                if self.buf.len() == need {
                    self.finish_hand(seat, need, s, net);
                }
            }
            Step::Tsumo { seat, .. } => {
                net.act(Action::Tsumo { seat, tile: t });
            }
            Step::Discard { seat } => match self.sub.clone() {
                Sub::None => {
                    let st = &s.seats[seat.index()];
                    if st.drawn == Some(t) || st.hand.contains(&t) {
                        self.discard(seat, t, net);
                    } else {
                        self.msg = Some(format!("手牌に{}がありません", t.name_ja()));
                    }
                }
                Sub::KanTile => match hands::self_kan(s, seat, t) {
                    Some((kind, tiles)) => self.sub = Sub::KanDora { seat, kind, tiles },
                    None => self.msg = Some(format!("{}では槓できません", t.name_ja())),
                },
                Sub::KanDora { seat, kind, tiles } => {
                    net.act(Action::Kan {
                        seat,
                        kind,
                        new_dora_indicator: Some(t),
                        tiles,
                    });
                    self.sub = Sub::None;
                }
                _ => {}
            },
            Step::Call { .. } => {
                if let Sub::KanDora { seat, kind, tiles } = self.sub.clone() {
                    net.act(Action::Kan {
                        seat,
                        kind,
                        new_dora_indicator: Some(t),
                        tiles,
                    });
                    self.sub = Sub::None;
                }
            }
            _ => {}
        }
        None
    }

    fn discard(&mut self, seat: SeatId, t: Tile, net: &dyn Sink) {
        net.act(Action::Discard {
            seat,
            tile: t,
            riichi: self.riichi,
        });
        self.riichi = false;
    }

    fn finish_hand(&mut self, seat: SeatId, need: usize, s: &TableState, net: &dyn Sink) {
        if self.buf.len() != need {
            self.msg = Some(format!("あと{}枚です", need - self.buf.len()));
            return;
        }
        let mut tiles = std::mem::take(&mut self.buf);
        // 14枚目（親の配牌・ツモのあとの入れ直し）はツモ牌として持つ
        let drawn = if tiles.len() + 3 * s.seats[seat.index()].melds.len() == 14 {
            tiles.pop()
        } else {
            None
        };
        net.act(Action::SetHand { seat, tiles, drawn });
        self.fix_seat = None;
    }

    fn choose_call(
        &mut self,
        seat: SeatId,
        kind: MeldKind,
        options: Vec<Vec<Tile>>,
        s: &TableState,
        net: &dyn Sink,
    ) {
        match options.len() {
            0 => {
                self.msg = Some(format!(
                    "{}は{}できません",
                    seat_name(s, seat),
                    if kind == MeldKind::Chi {
                        "チー"
                    } else {
                        "ポン"
                    }
                ));
                self.sub = Sub::None;
            }
            1 => {
                net.act(Action::Call {
                    seat,
                    kind,
                    tiles: options[0].clone(),
                });
                self.sub = Sub::None;
            }
            _ => {
                self.sub = Sub::CallChoose {
                    seat,
                    kind,
                    options,
                }
            }
        }
    }

    fn win(&mut self, s: &TableState, winner: SeatId, loser: Option<SeatId>) -> Option<Outcome> {
        match hands::win_input(s, winner, loser) {
            Some(w) => {
                self.sub = Sub::WinPending;
                Some(Outcome::Win(w))
            }
            None => {
                self.msg = Some("手牌が入っていないので、アガリ入力の欄で入れてください".into());
                None
            }
        }
    }

    /// 段階の見出し。
    pub fn title(&self, step: Step, s: &TableState) -> String {
        match step {
            Step::Off => String::new(),
            Step::Haipai { seat, need } => format!(
                "配牌の登録: {}（{}/{}枚）",
                seat_name(s, seat),
                self.buf.len(),
                need
            ),
            Step::Fix { seat, need } => format!(
                "手牌の入れ直し: {}（{}/{}枚）",
                seat_name(s, seat),
                self.buf.len(),
                need
            ),
            Step::Discard { seat } => match &self.sub {
                Sub::Menu => format!("{}: その他の操作を選ぶ", seat_name(s, seat)),
                Sub::KanTile => format!("{}: 槓する牌を選ぶ", seat_name(s, seat)),
                Sub::KanDora { .. } => "槓: 新しいドラ表示牌を選ぶ（なければ Enter）".into(),
                Sub::WinPending => "アガリ: 計算結果を確かめて Enter で放送に出す".into(),
                _ => format!(
                    "打牌: {}{}",
                    seat_name(s, seat),
                    if self.riichi {
                        "（リーチ宣言）"
                    } else {
                        ""
                    }
                ),
            },
            Step::Call { discarder } => match &self.sub {
                Sub::CallWho(k) => format!("{}するのは誰？", k.name_ja()),
                Sub::CallChoose { .. } => "どの牌で鳴く？".into(),
                Sub::KanDora { .. } => "槓: 新しいドラ表示牌を選ぶ（なければ Enter）".into(),
                Sub::WinPending => "アガリ: 計算結果を確かめて Enter で放送に出す".into(),
                _ => format!(
                    "鳴き・ロンの有無（{}の打牌 {}）",
                    seat_name(s, discarder),
                    s.seats[discarder.index()]
                        .river
                        .last()
                        .map(|r| r.tile.name_ja())
                        .unwrap_or_default()
                ),
            },
            Step::Tsumo { seat, rinshan } => format!(
                "{}: {}{}",
                if rinshan {
                    "嶺上牌のツモ"
                } else {
                    "ツモ"
                },
                seat_name(s, seat),
                if self.sub == Sub::WinPending {
                    "（アガリの確認中）"
                } else {
                    ""
                }
            ),
            Step::Exhausted => "流局（残り0）: テンパイの人を選んで Enter".into(),
        }
    }

    /// テンキーの図の文字（キー → 意味）。
    pub fn labels(&self, step: Step, s: &TableState) -> Vec<(NumKey, String)> {
        let mut v: Vec<(NumKey, String)> = vec![];
        let tiles = self.wants_tile(step) && self.sub != Sub::WinPending;
        if tiles {
            v.push((NumKey::Div, "◀ パレット".into()));
            v.push((NumKey::Mul, "パレット ▶".into()));
            for d in 0..=9u8 {
                if self.tile_for(d).is_some() {
                    let name = if self.palette == 3 {
                        HONORS[(d - 1) as usize].to_string()
                    } else if d == 0 {
                        "赤5".into()
                    } else {
                        format!(
                            "{d}{}",
                            PALETTES[self.palette].chars().next().unwrap_or(' ')
                        )
                    };
                    v.push((NumKey::Digit(d), name));
                }
            }
        }
        if self.sub == Sub::WinPending {
            v.push((NumKey::Enter, "放送に出す".into()));
            v.push((NumKey::Sub, "やめる".into()));
            return v;
        }
        match step {
            Step::Off => {}
            Step::Haipai { .. } => {
                v.push((NumKey::Sub, "1枚消す".into()));
                v.push((NumKey::Enter, "確定".into()));
            }
            Step::Fix { .. } => {
                v.push((NumKey::Sub, "1枚消す（空ならやめる）".into()));
                v.push((NumKey::Enter, "確定".into()));
            }
            Step::Discard { .. } => match &self.sub {
                Sub::Menu => {
                    v.push((NumKey::Digit(1), "カン".into()));
                    v.push((NumKey::Digit(2), "ツモアガリ".into()));
                    v.push((NumKey::Digit(3), "九種九牌".into()));
                    v.push((NumKey::Sub, "戻る".into()));
                }
                Sub::KanTile => v.push((NumKey::Sub, "戻る".into())),
                Sub::KanDora { .. } => {
                    v.push((NumKey::Enter, "ドラなし".into()));
                    v.push((NumKey::Sub, "戻る".into()));
                }
                _ => {
                    v.push((NumKey::Dot, "ツモ切り".into()));
                    v.push((
                        NumKey::Add,
                        if self.riichi {
                            "リーチ ON".into()
                        } else {
                            "リーチ".into()
                        },
                    ));
                    v.push((NumKey::Enter, "カン・ツモ・九種".into()));
                    v.push((NumKey::Sub, "取り消し".into()));
                }
            },
            Step::Call { discarder } => match &self.sub {
                Sub::CallWho(_) => {
                    for n in 1..=3u8 {
                        let seat = SeatId((discarder.0 + n) % 4);
                        let rel = ["下家", "対面", "上家"][(n - 1) as usize];
                        v.push((
                            NumKey::Digit(n),
                            format!("{rel} {}", s.seats[seat.index()].player.name),
                        ));
                    }
                    v.push((NumKey::Sub, "戻る".into()));
                }
                Sub::CallChoose { options, .. } => {
                    for (i, o) in options.iter().enumerate().take(9) {
                        v.push((NumKey::Digit(i as u8 + 1), tiles_text(o)));
                    }
                    v.push((NumKey::Sub, "戻る".into()));
                }
                Sub::KanDora { .. } => {
                    v.push((NumKey::Enter, "ドラなし".into()));
                    v.push((NumKey::Sub, "戻る".into()));
                }
                _ => {
                    v.push((NumKey::Enter, "鳴きなし".into()));
                    v.push((NumKey::Digit(1), "チー".into()));
                    v.push((NumKey::Digit(2), "ポン".into()));
                    v.push((NumKey::Digit(3), "カン".into()));
                    v.push((NumKey::Digit(4), "ロン".into()));
                    v.push((NumKey::Sub, "打牌の取り消し".into()));
                }
            },
            Step::Tsumo { .. } => v.push((NumKey::Sub, "戻る".into())),
            Step::Exhausted => {
                for n in 1..=4u8 {
                    let seat = SeatId((s.dealer().0 + n - 1) % 4);
                    let mark = if self.tenpai[seat.index()] {
                        "✔ "
                    } else {
                        ""
                    };
                    v.push((
                        NumKey::Digit(n),
                        format!(
                            "{mark}{} {}",
                            s.seat_wind(seat).name_ja(),
                            s.seats[seat.index()].player.name
                        ),
                    ));
                }
                v.push((NumKey::Enter, "流局を確定".into()));
                v.push((NumKey::Sub, "戻る".into()));
            }
        }
        v
    }

    /// 操作画面の「手牌・河」の欄。テンキーの図・入力中の牌・席ごとの手牌と河。
    /// 戻り値: クリックされたテンキー（画面のキーを押した）・パレットの牌
    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        s: &TableState,
        mode: &mut TileMode,
        tile_button: &dyn Fn(&mut egui::Ui, Tile, f32) -> egui::Response,
    ) -> (Option<NumKey>, Option<Tile>, bool) {
        let mut pressed = None;
        let mut picked = None;
        let mut mode_changed = false;
        ui.horizontal(|ui| {
            ui.heading("手牌・河");
            egui::ComboBox::from_id_salt("tile_mode")
                .selected_text(mode.name_ja())
                .show_ui(ui, |ui| {
                    for m in [TileMode::Manual, TileMode::AutoFix, TileMode::Off] {
                        if ui.selectable_value(mode, m, m.name_ja()).changed() {
                            mode_changed = true;
                        }
                    }
                });
        });
        if *mode == TileMode::Off {
            ui.weak("手牌・河は扱いません（テンキーの − ＋ は残り牌数）");
            return (None, None, mode_changed);
        }
        if *mode == TileMode::AutoFix {
            ui.weak(
                "牌の自動読み取り（カメラ）はまだ入っていません。入るまでは、手動と同じ流れで入れて、違うところを直してください",
            );
        }
        let step = self.step(s, *mode);
        self.sync(step);
        ui.horizontal(|ui| {
            // 左: テンキーの図
            let labels = self.labels(step, s);
            if let Some(k) = numpad(ui, &labels) {
                pressed = Some(k);
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(self.title(step, s)).size(22.0).strong());
                if self.wants_tile(step) && self.sub != Sub::WinPending {
                    ui.horizontal(|ui| {
                        ui.label("パレット:");
                        for (i, p) in PALETTES.iter().enumerate() {
                            if ui.selectable_label(self.palette == i, *p).clicked() {
                                self.palette = i;
                            }
                        }
                    });
                    // 今のパレットの牌（クリックでも入れられる）
                    ui.horizontal(|ui| {
                        let base = [0u8, 9, 18, 27][self.palette];
                        let n = if self.palette == 3 { 7 } else { 9 };
                        for k in base..base + n {
                            if tile_button(ui, Tile::new(k), 30.0).clicked() {
                                picked = Some(Tile::new(k));
                            }
                        }
                        if self.palette < 3 {
                            ui.add_space(6.0);
                            let r = Tile::red_five([Suit::Man, Suit::Pin, Suit::Sou][self.palette]);
                            if tile_button(ui, r, 30.0).on_hover_text("赤5（テンキー 0）").clicked()
                            {
                                picked = Some(r);
                            }
                        }
                    });
                }
                if matches!(step, Step::Haipai { .. } | Step::Fix { .. }) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("入力中:");
                        for t in &self.buf {
                            tile_button(ui, *t, 26.0);
                        }
                    });
                }
                if let Some(m) = &self.msg {
                    ui.colored_label(egui::Color32::from_rgb(255, 160, 100), m);
                }
                ui.weak("テンキーで操作します（NumLock の状態は問いません）。画面のキーや牌をクリックしてもよい");
            });
        });
        // 席ごとの手牌・河（「手牌を入れ直す」で修正）
        for seat in SeatId::ALL {
            let st = &s.seats[seat.index()];
            let here = matches!(step,
                Step::Haipai { seat: x, .. }
                | Step::Fix { seat: x, .. }
                | Step::Discard { seat: x }
                | Step::Tsumo { seat: x, .. } if x == seat);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let name = format!("{}{}", if here { "▶ " } else { "" }, seat_name(s, seat));
                ui.add_sized(
                    [150.0, 24.0],
                    egui::Label::new(egui::RichText::new(name).strong()),
                );
                if st.hand_known {
                    if ui
                        .small_button("入れ直す")
                        .on_hover_text("この人の手牌を入れ直す")
                        .clicked()
                    {
                        self.fix_seat = Some(seat);
                    }
                    ui.add_space(6.0);
                } else {
                    ui.weak("（手牌は未登録）");
                }
                for t in &st.hand {
                    tile_button(ui, *t, 20.0);
                }
                if let Some(d) = st.drawn {
                    ui.add_space(6.0);
                    tile_button(ui, d, 20.0).on_hover_text("ツモ牌");
                }
                for m in &st.melds {
                    ui.add_space(6.0);
                    for t in &m.tiles {
                        tile_button(ui, *t, 16.0);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.add_sized(
                    [150.0, 20.0],
                    egui::Label::new(egui::RichText::new("　河").weak()),
                );
                for r in &st.river {
                    let resp = tile_button(ui, r.tile, 16.0);
                    let mut tip = vec![];
                    if r.riichi {
                        tip.push("リーチ宣言牌");
                    }
                    if r.called {
                        tip.push("鳴かれた");
                    }
                    if r.tsumogiri {
                        tip.push("ツモ切り");
                    }
                    if !tip.is_empty() {
                        resp.on_hover_text(tip.join("・"));
                    }
                }
            });
        }
        (pressed, picked, mode_changed)
    }
}

/// テンキーの図。クリックされたキーを返す。
fn numpad(ui: &mut egui::Ui, labels: &[(NumKey, String)]) -> Option<NumKey> {
    let cell = egui::vec2(78.0, 54.0);
    let gap = 4.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(4.0 * cell.x + 3.0 * gap, 5.0 * cell.y + 4.0 * gap),
        egui::Sense::hover(),
    );
    // (キー, 表示, 列, 行, 幅, 高さ)
    let keys: [(Option<NumKey>, &str, f32, f32, f32, f32); 17] = [
        (None, "Num", 0.0, 0.0, 1.0, 1.0),
        (Some(NumKey::Div), "/", 1.0, 0.0, 1.0, 1.0),
        (Some(NumKey::Mul), "*", 2.0, 0.0, 1.0, 1.0),
        (Some(NumKey::Sub), "−", 3.0, 0.0, 1.0, 1.0),
        (Some(NumKey::Digit(7)), "7", 0.0, 1.0, 1.0, 1.0),
        (Some(NumKey::Digit(8)), "8", 1.0, 1.0, 1.0, 1.0),
        (Some(NumKey::Digit(9)), "9", 2.0, 1.0, 1.0, 1.0),
        (Some(NumKey::Add), "+", 3.0, 1.0, 1.0, 2.0),
        (Some(NumKey::Digit(4)), "4", 0.0, 2.0, 1.0, 1.0),
        (Some(NumKey::Digit(5)), "5", 1.0, 2.0, 1.0, 1.0),
        (Some(NumKey::Digit(6)), "6", 2.0, 2.0, 1.0, 1.0),
        (Some(NumKey::Digit(1)), "1", 0.0, 3.0, 1.0, 1.0),
        (Some(NumKey::Digit(2)), "2", 1.0, 3.0, 1.0, 1.0),
        (Some(NumKey::Digit(3)), "3", 2.0, 3.0, 1.0, 1.0),
        (Some(NumKey::Enter), "Enter", 3.0, 3.0, 1.0, 2.0),
        (Some(NumKey::Digit(0)), "0", 0.0, 4.0, 2.0, 1.0),
        (Some(NumKey::Dot), ".", 2.0, 4.0, 1.0, 1.0),
    ];
    let mut clicked = None;
    let visuals = ui.visuals().clone();
    for (i, (k, name, c, r, w, h)) in keys.iter().enumerate() {
        let min = rect.min + egui::vec2(c * (cell.x + gap), r * (cell.y + gap));
        let size = egui::vec2(w * cell.x + (w - 1.0) * gap, h * cell.y + (h - 1.0) * gap);
        let kr = egui::Rect::from_min_size(min, size);
        let label = k.and_then(|k| labels.iter().find(|(x, _)| *x == k).map(|(_, l)| l.clone()));
        let id = ui.id().with(("numpad", i));
        let resp = ui.interact(kr, id, egui::Sense::click());
        let active = label.is_some();
        let fill = if !active {
            visuals.extreme_bg_color
        } else if resp.hovered() {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let p = ui.painter();
        p.rect_filled(kr, 6.0, fill);
        p.rect_stroke(
            kr,
            6.0,
            egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
        p.text(
            kr.min + egui::vec2(5.0, 3.0),
            egui::Align2::LEFT_TOP,
            *name,
            egui::FontId::proportional(11.0),
            visuals.weak_text_color(),
        );
        if let Some(l) = &label {
            let galley = p.layout(
                l.clone(),
                egui::FontId::proportional(14.0),
                visuals.strong_text_color(),
                kr.width() - 6.0,
            );
            let pos = kr.center() - galley.size() / 2.0 + egui::vec2(0.0, 5.0);
            p.galley(pos, galley, visuals.strong_text_color());
        }
        if active && resp.clicked() {
            clicked = *k;
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use janscope_core::engine::apply;
    use janscope_core::rules::GameConfig;
    use janscope_core::state::Player;
    use std::cell::RefCell;

    struct Fake(RefCell<TableState>);
    impl Sink for Fake {
        fn act(&self, a: Action) -> u64 {
            apply(&mut self.0.borrow_mut(), &a).unwrap_or_else(|e| panic!("{a:?}: {e}"));
            0
        }
        fn undo(&self) -> u64 {
            0
        }
    }

    fn press(ti: &mut TileInput, f: &Fake, keys: &[NumKey]) -> Option<Outcome> {
        let mut last = None;
        for k in keys {
            let st = f.0.borrow().clone();
            last = ti.key(*k, &st, TileMode::Manual, f);
        }
        last
    }

    use NumKey::*;

    /// パレットを合わせて数字を押す（萬子=0…字牌=3）
    fn tiles(ti_palette: &mut usize, want: &[(usize, u8)]) -> Vec<NumKey> {
        let mut v = vec![];
        for (p, d) in want {
            while *ti_palette != *p {
                v.push(Mul);
                *ti_palette = (*ti_palette + 1) % 4;
            }
            v.push(Digit(*d));
        }
        v
    }

    #[test]
    fn numpad_only_flow() {
        let mut s = TableState::default();
        apply(
            &mut s,
            &Action::StartGame {
                config: GameConfig::default(),
                players: ["A", "B", "C", "D"].map(Player::new),
                chiicha: SeatId(0),
            },
        )
        .unwrap();
        apply(&mut s, &Action::StartHand { dice: vec![(1, 2)] }).unwrap();
        let f = Fake(RefCell::new(s));
        let mut ti = TileInput::default();
        let mut pal = 0;
        // 親（A）: 123m 456p 789s 東東南南 ＋ 白（14枚目）
        let a = tiles(
            &mut pal,
            &[
                (0, 1),
                (0, 2),
                (0, 3),
                (1, 4),
                (1, 5),
                (1, 6),
                (2, 7),
                (2, 8),
                (2, 9),
                (3, 1),
                (3, 1),
                (3, 2),
                (3, 2),
                (3, 5),
            ],
        );
        press(&mut ti, &f, &a);
        assert!(f.0.borrow().seats[0].hand_known);
        assert_eq!(
            f.0.borrow().seats[0].drawn,
            Some(Tile::from_suit_number(Suit::Honor, 5))
        );
        // 打ち間違いを − で消してから続ける（B: 1m×3 ...）
        let mut b = tiles(&mut pal, &[(0, 9)]);
        b.push(Sub);
        b.extend(tiles(
            &mut pal,
            &[
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 5),
                (0, 6),
                (0, 7),
                (1, 1),
                (1, 2),
                (1, 3),
                (2, 1),
                (2, 2),
                (2, 3),
                (3, 7),
            ],
        ));
        press(&mut ti, &f, &b);
        let c = tiles(
            &mut pal,
            &[
                (0, 4),
                (0, 4),
                (0, 8),
                (0, 8),
                (1, 7),
                (1, 7),
                (1, 8),
                (1, 8),
                (2, 4),
                (2, 4),
                (2, 5),
                (2, 5),
                (3, 6),
            ],
        );
        press(&mut ti, &f, &c);
        let d = tiles(
            &mut pal,
            &[
                (0, 2),
                (0, 3),
                (0, 9),
                (1, 1),
                (1, 9),
                (1, 9),
                (2, 1),
                (2, 6),
                (2, 6),
                (3, 3),
                (3, 3),
                (3, 4),
                (3, 4),
            ],
        );
        press(&mut ti, &f, &d);
        let st = f.0.borrow().clone();
        assert_eq!(st.turn, Turn::Discard { seat: SeatId(0) });
        // 親: ツモ切り（.）→ 鳴きなし（Enter）→ B のツモ 9m → リーチ（+）してツモ切り
        let mut k = vec![Dot, Enter];
        k.extend(tiles(&mut pal, &[(0, 9)]));
        k.extend([Add, Dot]);
        press(&mut ti, &f, &k);
        let st = f.0.borrow().clone();
        assert!(st.seats[1].riichi);
        assert_eq!(st.turn, Turn::AfterDiscard { seat: SeatId(1) });
        // C が 9m をポンしようとする → 持っていないので「できません」、そのまま
        press(&mut ti, &f, &[Digit(2), Digit(1)]);
        assert!(ti.msg.is_some());
        // 鳴きなし → C のツモ 1z → 打牌 1z（手出しではなくツモ切り扱い）
        let mut k = vec![Enter];
        k.extend(tiles(&mut pal, &[(3, 1), (3, 1)]));
        press(&mut ti, &f, &k);
        // A が 東 をポン（C は A の上家なので、C から見て A は下家=1）
        press(&mut ti, &f, &[Digit(2), Digit(2)]);
        let st = f.0.borrow().clone();
        assert_eq!(st.seats[0].melds.len(), 1, "{:?}", ti.msg);
        assert_eq!(st.turn, Turn::Discard { seat: SeatId(0) });
        // A が 南 を打つ → D がロン（4 → A から見て D は上家=3）→ アガリの欄を埋める
        let mut k = tiles(&mut pal, &[(3, 2)]);
        k.extend([Digit(4), Digit(3)]);
        let o = press(&mut ti, &f, &k);
        match o {
            Some(Outcome::Win(w)) => {
                assert_eq!(w.winner, SeatId(3));
                assert_eq!(w.loser, Some(SeatId(0)));
                assert_eq!(w.closed.len(), 14);
            }
            _ => panic!("ロンの入力ができていない: {:?}", ti.msg),
        }
        assert_eq!(ti.sub, super::Sub::WinPending);
        assert!(matches!(
            press(&mut ti, &f, &[Enter]),
            Some(Outcome::ConfirmWin)
        ));
    }
}

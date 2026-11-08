//! 牌の手入力（手牌・河）。テンキーだけで入力できるようにする。
//!
//! - `/` `*` で牌のパレット（萬子・筒子・索子・字牌）を切り替え、`1`〜`9` で牌を入れる（`0` は赤5、字牌のパレットでは不明の牌）
//! - 打牌のときのパレットは、その人の手牌にある牌だけ
//! - 全手動の流れ: 局の開始にドラ表示牌（飛ばせる）→ 親から順に配牌 → 親の打牌 → 鳴きの有無 → 下家のツモ → 打牌か槓 → …
//! - 次に何を入れるかは卓状態（`TableState::turn` など）から決めるので、取り消し・別の端末からの操作とも食い違わない
//! - 押せるキーと意味は、操作画面のテンキーの図にいつも出す。図の下の通知バーに、今やることを出す

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

/// 鳴き・ロンの種類（誰がを選ぶとき）。
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
    /// 暗槓・加槓する牌を選ぶ（候補が2つ以上のとき）
    KanTile,
    /// 暗槓の新しいドラ表示牌（すぐめくる）
    KanDora { seat: SeatId, tiles: Vec<Tile> },
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
    /// 局の開始: ドラ表示牌（Enter で飛ばせる）
    DoraStart,
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
    /// 大明槓・加槓のあとの打牌のあと: 新しいドラ表示牌
    DoraReveal,
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
    /// 履歴の操作の牌を直す
    HistFix {
        index: usize,
    },
}

/// 操作画面に返すこと（アガリ入力の欄を埋める、など）。
pub enum Outcome {
    /// アガリ入力の欄をこの内容で埋めてプレビューする
    Win(WinInput),
    /// プレビュー済みのアガリを放送に出す
    ConfirmWin,
}

/// テンキーの図の1つのキーの中身
#[derive(Clone, Debug, PartialEq)]
pub struct KeyLabel {
    pub text: String,
    /// 牌の絵を出す
    pub tile: Option<Tile>,
}

impl KeyLabel {
    fn text(t: impl Into<String>) -> Self {
        KeyLabel {
            text: t.into(),
            tile: None,
        }
    }
    fn tile(t: Tile) -> Self {
        KeyLabel {
            text: String::new(),
            tile: Some(t),
        }
    }
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
    /// 槓ドラをあとで入れると決めた打牌（同上）
    pub dora_later: Option<(SeatId, usize)>,
    /// 局の開始のドラ表示牌を飛ばした局
    pub dora_skipped: Option<String>,
    /// 手牌を入れ直している席
    pub fix_seat: Option<SeatId>,
    /// 履歴の牌を直している操作（番号と、その時の局の操作の列）
    pub hist_fix: Option<(usize, Vec<Action>)>,
    /// 履歴で選んでいる操作
    pub hist_sel: Option<usize>,
    /// 流局のテンパイ
    pub tenpai: [bool; 4],
    pub msg: Option<String>,
    /// 段階が変わったら途中の選択を捨てるため
    last_step: Option<Step>,
}

const PALETTES: [&str; 4] = ["萬子", "筒子", "索子", "字牌"];

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

/// 局ごとのかぎ（局が変わったら「飛ばした」などを忘れる）
fn hand_key(s: &TableState) -> String {
    format!(
        "{:?}{}-{}-{}",
        s.round_wind,
        s.round_number,
        s.honba,
        s.history.len()
    )
}

/// パレットの番号（不明の牌は字牌のパレット）
fn palette_of(t: Tile) -> usize {
    if t.is_unknown() {
        3
    } else {
        match t.suit() {
            Suit::Man => 0,
            Suit::Pin => 1,
            Suit::Sou => 2,
            Suit::Honor => 3,
        }
    }
}

/// パレットの数字キー → 牌（すべての牌から）
fn full_tile(palette: usize, d: u8) -> Option<Tile> {
    match (palette, d) {
        (3, 1..=7) => Some(Tile::from_suit_number(Suit::Honor, d)),
        (3, 0) => Some(Tile::UNKNOWN),
        (3, _) => None,
        (p, 0) => Some(Tile::red_five([Suit::Man, Suit::Pin, Suit::Sou][p])),
        (p, d) => Some(Tile::from_suit_number(
            [Suit::Man, Suit::Pin, Suit::Sou][p],
            d,
        )),
    }
}

impl TileInput {
    pub fn step(&self, s: &TableState, mode: TileMode) -> Step {
        if mode == TileMode::Off || s.phase != Phase::InHand {
            return Step::Off;
        }
        if let Some((index, _)) = &self.hist_fix {
            return Step::HistFix { index: *index };
        }
        if let Some(seat) = self.fix_seat {
            let st = &s.seats[seat.index()];
            let full = st.drawn.is_some() || st.tile_count() == 14;
            let need = if full { 14 } else { 13 } - 3 * st.melds.len();
            return Step::Fix { seat, need };
        }
        if s.turn == Turn::None {
            if s.dora_indicators.is_empty()
                && s.seats.iter().all(|x| !x.hand_known)
                && self.dora_skipped.as_deref() != Some(hand_key(s).as_str())
            {
                return Step::DoraStart;
            }
            if let Some(seat) = hands::next_haipai(s) {
                let need = if seat == s.dealer() { 14 } else { 13 };
                return Step::Haipai { seat, need };
            }
        }
        match s.turn {
            Turn::Discard { seat } => Step::Discard { seat },
            Turn::AfterDiscard { seat } => {
                let mark = Some((seat, s.seats[seat.index()].river.len()));
                if s.kan_dora_pending > 0 && self.dora_later != mark {
                    Step::DoraReveal
                } else if self.passed == mark {
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
    pub fn sync(&mut self, step: Step, s: &TableState) {
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
            // 打牌の番では、ツモ牌の色のパレットにしておく
            if let Step::Discard { seat } = step {
                let st = &s.seats[seat.index()];
                if let Some(t) = st.drawn.or_else(|| st.hand.first().copied()) {
                    self.palette = palette_of(t);
                }
            }
            self.last_step = Some(step);
        }
    }

    /// 打牌・暗槓で使う、手牌から作るパレットか
    fn hand_palette(&self, step: Step) -> Option<SeatId> {
        match step {
            Step::Discard { seat } if matches!(self.sub, Sub::None | Sub::KanTile) => Some(seat),
            _ => None,
        }
    }

    /// 手牌（ツモ牌込み）
    fn hand_of(s: &TableState, seat: SeatId) -> Vec<Tile> {
        let st = &s.seats[seat.index()];
        let mut v = st.hand.clone();
        v.extend(st.drawn);
        v
    }

    /// 手牌から作るパレットの色（手牌にある色だけ）
    fn hand_palettes(s: &TableState, seat: SeatId) -> Vec<usize> {
        let mut v: Vec<usize> = Self::hand_of(s, seat).into_iter().map(palette_of).collect();
        v.sort();
        v.dedup();
        v
    }

    /// 数字キー → 牌（手牌のパレットなら手牌にある牌だけ）
    fn tile_for(&self, d: u8, step: Step, s: &TableState) -> Option<Tile> {
        let t = full_tile(self.palette, d)?;
        let Some(seat) = self.hand_palette(step) else {
            return Some(t);
        };
        let hand = Self::hand_of(s, seat);
        if hand.contains(&t) {
            return Some(t);
        }
        // 5 は赤しか持っていなければ赤を出す
        if d == 5 && !t.is_honor() {
            let r = t.plain();
            return hand
                .iter()
                .copied()
                .find(|x| x.kind() == r.kind() && x.is_red());
        }
        None
    }

    /// 段階で牌を入れるか（パレットのキーが効くか）。
    fn wants_tile(&self, step: Step) -> bool {
        if self.sub == Sub::WinPending {
            return false;
        }
        match step {
            Step::DoraStart
            | Step::Haipai { .. }
            | Step::Fix { .. }
            | Step::Tsumo { .. }
            | Step::DoraReveal
            | Step::HistFix { .. } => true,
            Step::Discard { .. } => {
                matches!(self.sub, Sub::None | Sub::KanTile | Sub::KanDora { .. })
            }
            _ => false,
        }
    }

    fn move_palette(&mut self, step: Step, s: &TableState, dir: isize) {
        let list: Vec<usize> = match self.hand_palette(step) {
            Some(seat) => Self::hand_palettes(s, seat),
            None => vec![0, 1, 2, 3],
        };
        if list.is_empty() {
            return;
        }
        let i = list.iter().position(|p| *p == self.palette).unwrap_or(0) as isize;
        let n = list.len() as isize;
        self.palette = list[((i + dir).rem_euclid(n)) as usize];
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
        self.sync(step, s);
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
                    self.move_palette(step, s, -1);
                    return None;
                }
                NumKey::Mul => {
                    self.move_palette(step, s, 1);
                    return None;
                }
                NumKey::Digit(d) => {
                    match self.tile_for(d, step, s) {
                        Some(t) => return self.tile(t, s, mode, net),
                        None if self.hand_palette(step).is_some() => {
                            self.msg = Some("その牌は手牌にありません".into())
                        }
                        None => {}
                    }
                    return None;
                }
                _ => {}
            }
        }
        match step {
            Step::DoraStart => {
                if k == NumKey::Enter {
                    self.dora_skipped = Some(hand_key(s));
                }
            }
            Step::Haipai { seat, need } | Step::Fix { seat, need } => match k {
                NumKey::Sub => {
                    if self.buf.pop().is_none() && matches!(step, Step::Fix { .. }) {
                        self.fix_seat = None;
                    }
                }
                NumKey::Enter => self.finish_hand(seat, need, s, net),
                _ => {}
            },
            Step::HistFix { .. } => {
                if k == NumKey::Sub {
                    self.hist_fix = None;
                }
            }
            Step::Discard { seat } => match (&self.sub, k) {
                (Sub::None, NumKey::Enter) => match s.seats[seat.index()].drawn {
                    Some(t) => self.discard(seat, t, net),
                    None => {
                        self.msg = Some("ツモ牌がありません（鳴いたあとは手牌から選びます）".into())
                    }
                },
                (Sub::None, NumKey::Add) => self.riichi = !self.riichi,
                (Sub::None, NumKey::Dot) => self.sub = Sub::Menu,
                (Sub::None, NumKey::Sub) => {
                    net.undo();
                }
                (Sub::Menu, NumKey::Digit(1)) => {
                    // 槓できる牌（1つなら、そのまま槓）
                    let cands: Vec<Tile> = {
                        let mut v: Vec<Tile> = Self::hand_of(s, seat)
                            .into_iter()
                            .filter(|t| hands::self_kan(s, seat, *t).is_some())
                            .map(|t| t.plain())
                            .collect();
                        v.sort_by_key(|t| t.sort_key());
                        v.dedup();
                        v
                    };
                    match cands.len() {
                        0 => {
                            self.msg = Some("槓できる牌がありません".into());
                            self.sub = Sub::None;
                        }
                        1 => {
                            self.sub = Sub::None;
                            self.self_kan(seat, cands[0], s, net);
                        }
                        _ => self.sub = Sub::KanTile,
                    }
                }
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
                (Sub::KanDora { seat, tiles }, NumKey::Enter) => {
                    net.act(Action::Kan {
                        seat: *seat,
                        kind: KanKind::Ankan,
                        new_dora_indicator: None,
                        tiles: tiles.clone(),
                    });
                    self.sub = Sub::None;
                }
                (_, NumKey::Sub) => self.sub = Sub::None,
                _ => {}
            },
            Step::DoraReveal => match k {
                NumKey::Enter => {
                    if let Turn::AfterDiscard { seat } = s.turn {
                        self.dora_later = Some((seat, s.seats[seat.index()].river.len()));
                    }
                }
                NumKey::Sub => {
                    net.undo();
                }
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
                    (Sub::None, NumKey::Digit(n @ 2..=4)) => {
                        let kind = [CallKind::Pon, CallKind::Kan, CallKind::Ron][(n - 2) as usize];
                        // ポン・カンは、できる人が1人ならその人に決める
                        let able: Vec<SeatId> = (1..=3u8)
                            .map(|i| SeatId((discarder.0 + i) % 4))
                            .filter(|x| called.is_some_and(|c| can_call(s, *x, kind, c)))
                            .collect();
                        if kind != CallKind::Ron && able.len() == 1 {
                            return self.call_by(kind, able[0], discarder, called?, s, net);
                        }
                        if kind != CallKind::Ron && able.is_empty() {
                            self.msg = Some(format!("{}できる人がいません", kind.name_ja()));
                        } else {
                            self.sub = Sub::CallWho(kind);
                        }
                    }
                    (Sub::CallWho(kind), NumKey::Digit(n @ 1..=3)) => {
                        let seat = SeatId((discarder.0 + n) % 4);
                        return self.call_by(kind, seat, discarder, called?, s, net);
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

    /// ポン・カン・ロンする人が決まった
    fn call_by(
        &mut self,
        kind: CallKind,
        seat: SeatId,
        discarder: SeatId,
        called: Tile,
        s: &TableState,
        net: &dyn Sink,
    ) -> Option<Outcome> {
        let hand = &s.seats[seat.index()].hand;
        match kind {
            CallKind::Pon => {
                let opts = hands::pon_options(hand, called);
                self.choose_call(seat, MeldKind::Pon, opts, s, net);
            }
            CallKind::Kan => match hands::minkan_tiles(hand, called) {
                // 大明槓の新ドラは、嶺上牌をツモって打牌したあとに入れる
                Some(tiles) => {
                    net.act(Action::Kan {
                        seat,
                        kind: KanKind::Minkan,
                        new_dora_indicator: None,
                        tiles,
                    });
                    self.sub = Sub::None;
                }
                None => {
                    self.msg = Some(format!(
                        "{}の手牌に{}が3枚ありません",
                        seat_name(s, seat),
                        called.name_ja()
                    ));
                    self.sub = Sub::None;
                }
            },
            CallKind::Ron => {
                self.sub = Sub::None;
                return self.win(s, seat, Some(discarder));
            }
        }
        None
    }

    /// 暗槓・加槓（暗槓はすぐ新ドラ、加槓は打牌のあと）
    fn self_kan(&mut self, seat: SeatId, t: Tile, s: &TableState, net: &dyn Sink) {
        match hands::self_kan(s, seat, t) {
            Some((KanKind::Ankan, tiles)) => self.sub = Sub::KanDora { seat, tiles },
            Some((kind, tiles)) => {
                net.act(Action::Kan {
                    seat,
                    kind,
                    new_dora_indicator: None,
                    tiles,
                });
            }
            None => self.msg = Some(format!("{}では槓できません", t.name_ja())),
        }
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
        self.sync(step, s);
        self.msg = None;
        match step {
            Step::DoraStart => {
                net.act(Action::SetDora { tiles: vec![t] });
            }
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
            Step::DoraReveal => {
                let mut tiles = s.dora_indicators.clone();
                tiles.push(t);
                net.act(Action::SetDora { tiles });
            }
            Step::HistFix { index } => {
                if let Some((_, log)) = self.hist_fix.take() {
                    self.rewrite(&log, index, t, net);
                }
            }
            Step::Discard { seat } => match self.sub.clone() {
                Sub::None => {
                    if Self::hand_of(s, seat).contains(&t) {
                        self.discard(seat, t, net);
                    } else {
                        self.msg = Some(format!("手牌に{}がありません", t.name_ja()));
                    }
                }
                Sub::KanTile => {
                    self.sub = Sub::None;
                    self.self_kan(seat, t, s, net);
                }
                Sub::KanDora { seat, tiles } => {
                    net.act(Action::Kan {
                        seat,
                        kind: KanKind::Ankan,
                        new_dora_indicator: Some(t),
                        tiles,
                    });
                    self.sub = Sub::None;
                }
                _ => {}
            },
            _ => {}
        }
        None
    }

    /// 履歴の index 番目の操作の牌を t に直して、そのあとの操作をやり直す
    fn rewrite(&mut self, log: &[Action], index: usize, t: Tile, net: &dyn Sink) {
        let Some(fixed) = replace_tile(&log[index], t) else {
            self.msg = Some("この操作の牌は直せません".into());
            return;
        };
        for _ in index..log.len() {
            net.undo();
        }
        net.act(fixed);
        for a in &log[index + 1..] {
            net.act(a.clone());
        }
        self.hist_sel = None;
        self.msg = Some(format!(
            "{}番目の操作を直して、そのあとの {} 件をやり直しました（合わなくなった操作があれば、そこで止まります）",
            index + 1,
            log.len() - index - 1
        ));
    }

    /// 履歴の index 番目から先を取り消す（そこから入れ直す）
    pub fn rewind(&mut self, log: &[Action], index: usize, net: &dyn Sink) {
        for _ in index..log.len() {
            net.undo();
        }
        self.hist_sel = None;
        self.passed = None;
        self.dora_later = None;
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

    /// 通知バーに出す「今やること」
    pub fn prompt(&self, step: Step, s: &TableState) -> String {
        if self.sub == Sub::WinPending {
            return "アガリの計算結果を確かめて、Enter で放送に出してください（− でやめる）".into();
        }
        match step {
            Step::Off => String::new(),
            Step::DoraStart => "ドラ表示牌を選んでください（あとで入れるなら Enter）".into(),
            Step::Haipai { seat, need } => format!(
                "{}の配牌を選んでください（{}/{}枚）",
                seat_name(s, seat),
                self.buf.len(),
                need
            ),
            Step::Fix { seat, need } => format!(
                "{}の手牌を入れ直してください（{}/{}枚。− で1枚消す）",
                seat_name(s, seat),
                self.buf.len(),
                need
            ),
            Step::HistFix { index } => format!(
                "{}番目の操作の、正しい牌を選んでください（− でやめる）",
                index + 1
            ),
            Step::Discard { seat } => match &self.sub {
                Sub::Menu => format!(
                    "{}: 1 カン・2 ツモアガリ・3 九種九牌 から選んでください",
                    seat_name(s, seat)
                ),
                Sub::KanTile => format!("{}: 槓する牌を選んでください", seat_name(s, seat)),
                Sub::KanDora { .. } => {
                    "暗槓: 新しいドラ表示牌を選んでください（あとで入れるなら Enter）".into()
                }
                _ => format!(
                    "{}の捨牌を選んでください（Enter: ツモ切り）{}",
                    seat_name(s, seat),
                    if self.riichi {
                        "　【リーチ宣言】"
                    } else {
                        ""
                    }
                ),
            },
            Step::DoraReveal => {
                "槓の新しいドラ表示牌を選んでください（あとで入れるなら Enter）".into()
            }
            Step::Call { discarder } => match &self.sub {
                Sub::CallWho(k) => format!("{}するのは誰か選んでください", k.name_ja()),
                Sub::CallChoose { .. } => "鳴く牌の組み合わせを選んでください".into(),
                _ => format!(
                    "{}の打牌 {} を鳴く・ロンする人はいますか？（なければ Enter）",
                    seat_name(s, discarder),
                    s.seats[discarder.index()]
                        .river
                        .last()
                        .map(|r| r.tile.name_ja())
                        .unwrap_or_default()
                ),
            },
            Step::Tsumo { seat, rinshan } => format!(
                "{}の{}を選んでください",
                seat_name(s, seat),
                if rinshan { "嶺上牌" } else { "ツモ牌" }
            ),
            Step::Exhausted => "流局です。テンパイの人を 1〜4（東南西北）で選んで Enter".into(),
        }
    }

    /// テンキーの図の中身（キー → 意味）。
    pub fn labels(&self, step: Step, s: &TableState) -> Vec<(NumKey, KeyLabel)> {
        let mut v: Vec<(NumKey, KeyLabel)> = vec![];
        if self.wants_tile(step) {
            v.push((NumKey::Div, KeyLabel::text("◀ パレット")));
            v.push((NumKey::Mul, KeyLabel::text("パレット ▶")));
            for d in 0..=9u8 {
                if let Some(t) = self.tile_for(d, step, s) {
                    v.push((NumKey::Digit(d), KeyLabel::tile(t)));
                }
            }
        }
        if self.sub == Sub::WinPending {
            v.push((NumKey::Enter, KeyLabel::text("放送に出す")));
            v.push((NumKey::Sub, KeyLabel::text("やめる")));
            return v;
        }
        let t = |k: NumKey, s: &str| (k, KeyLabel::text(s));
        match step {
            Step::Off => {}
            Step::DoraStart => v.push(t(NumKey::Enter, "あとで入れる")),
            Step::Haipai { .. } => {
                v.push(t(NumKey::Sub, "1枚消す"));
                v.push(t(NumKey::Enter, "確定"));
            }
            Step::Fix { .. } => {
                v.push(t(NumKey::Sub, "1枚消す（空ならやめる）"));
                v.push(t(NumKey::Enter, "確定"));
            }
            Step::HistFix { .. } => v.push(t(NumKey::Sub, "やめる")),
            Step::Discard { .. } => match &self.sub {
                Sub::Menu => {
                    v.push(t(NumKey::Digit(1), "カン"));
                    v.push(t(NumKey::Digit(2), "ツモアガリ"));
                    v.push(t(NumKey::Digit(3), "九種九牌"));
                    v.push(t(NumKey::Sub, "戻る"));
                }
                Sub::KanTile => v.push(t(NumKey::Sub, "戻る")),
                Sub::KanDora { .. } => {
                    v.push(t(NumKey::Enter, "あとで入れる"));
                    v.push(t(NumKey::Sub, "戻る"));
                }
                _ => {
                    v.push(t(NumKey::Enter, "ツモ切り"));
                    v.push(t(
                        NumKey::Add,
                        if self.riichi {
                            "リーチ ON"
                        } else {
                            "リーチ"
                        },
                    ));
                    v.push(t(NumKey::Dot, "カン・ツモ・九種"));
                    v.push(t(NumKey::Sub, "取り消し"));
                }
            },
            Step::DoraReveal => {
                v.push(t(NumKey::Enter, "あとで入れる"));
                v.push(t(NumKey::Sub, "打牌の取り消し"));
            }
            Step::Call { discarder } => match &self.sub {
                Sub::CallWho(_) => {
                    for n in 1..=3u8 {
                        let seat = SeatId((discarder.0 + n) % 4);
                        let rel = ["下家", "対面", "上家"][(n - 1) as usize];
                        v.push((
                            NumKey::Digit(n),
                            KeyLabel::text(format!("{rel} {}", s.seats[seat.index()].player.name)),
                        ));
                    }
                    v.push(t(NumKey::Sub, "戻る"));
                }
                Sub::CallChoose { options, .. } => {
                    for (i, o) in options.iter().enumerate().take(9) {
                        v.push((NumKey::Digit(i as u8 + 1), KeyLabel::text(tiles_text(o))));
                    }
                    v.push(t(NumKey::Sub, "戻る"));
                }
                _ => {
                    v.push(t(NumKey::Enter, "鳴きなし"));
                    v.push(t(NumKey::Digit(1), "チー"));
                    v.push(t(NumKey::Digit(2), "ポン"));
                    v.push(t(NumKey::Digit(3), "カン"));
                    v.push(t(NumKey::Digit(4), "ロン"));
                    v.push(t(NumKey::Sub, "打牌の取り消し"));
                }
            },
            Step::Tsumo { .. } => v.push(t(NumKey::Sub, "戻る")),
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
                        KeyLabel::text(format!(
                            "{mark}{} {}",
                            s.seat_wind(seat).name_ja(),
                            s.seats[seat.index()].player.name
                        )),
                    ));
                }
                v.push(t(NumKey::Enter, "流局を確定"));
                v.push(t(NumKey::Sub, "戻る"));
            }
        }
        v
    }

    /// 操作画面の「手牌・河」の欄。テンキーの図・パレット・通知バー・席ごとの手牌と河。
    /// 戻り値: クリックされたテンキー（画面のキーを押した）・パレットの牌・入れ方が変わったか
    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        s: &TableState,
        mode: &mut TileMode,
        tile_button: &dyn Fn(&mut egui::Ui, Tile, f32) -> egui::Response,
        tex: &dyn Fn(Tile) -> Option<egui::TextureId>,
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
            if *mode != TileMode::Off {
                ui.label(
                    egui::RichText::new(format!("残り {}（自動）", s.wall_remaining))
                        .size(20.0)
                        .strong(),
                );
            }
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
        self.sync(step, s);
        ui.horizontal(|ui| {
            // 左: テンキーの図
            let labels = self.labels(step, s);
            if let Some(k) = numpad(ui, &labels, tex) {
                pressed = Some(k);
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                if self.wants_tile(step) {
                    let hand = self.hand_palette(step);
                    let list: Vec<usize> = match hand {
                        Some(seat) => Self::hand_palettes(s, seat),
                        None => vec![0, 1, 2, 3],
                    };
                    ui.horizontal(|ui| {
                        ui.label(if hand.is_some() {
                            "パレット（手牌から）:"
                        } else {
                            "パレット:"
                        });
                        for i in list {
                            if ui.selectable_label(self.palette == i, PALETTES[i]).clicked() {
                                self.palette = i;
                            }
                        }
                    });
                    // 今のパレットの牌（クリックでも入れられる）
                    ui.horizontal(|ui| {
                        for d in [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 0] {
                            if let Some(t) = self.tile_for(d, step, s) {
                                let tip = if t.is_unknown() {
                                    "不明の牌（テンキー 0）".to_string()
                                } else {
                                    format!("{}（テンキー {d}）", t.name_ja())
                                };
                                if tile_button(ui, t, 32.0).on_hover_text(tip).clicked() {
                                    picked = Some(t);
                                }
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
                // 通知バー: 今やること
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(30, 70, 120))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.set_min_width(520.0);
                        ui.label(
                            egui::RichText::new(self.prompt(step, s))
                                .size(20.0)
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                    });
                if let Some(m) = &self.msg {
                    ui.colored_label(egui::Color32::from_rgb(255, 160, 100), m);
                }
                ui.weak("テンキーで操作します（NumLock の状態は問いません）。画面のキーや牌をクリックしてもよい");
            });
        });
        // 席ごとの手牌・河（「入れ直す」で修正）
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

    /// 進行の履歴（今の局の操作）。選んで、そこから入れ直す・牌を直す
    pub fn history(&mut self, ui: &mut egui::Ui, s: &TableState, log: &[Action], net: &dyn Sink) {
        ui.horizontal(|ui| {
            ui.heading("進行の履歴");
            ui.weak("（この局の操作。選んで、そこから入れ直す・牌を直す）");
        });
        if let Some(i) = self.hist_sel {
            if i >= log.len() {
                self.hist_sel = None;
            }
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.weak("（まだ操作がありません）");
                    }
                    for (i, a) in log.iter().enumerate() {
                        let sel = self.hist_sel == Some(i);
                        if ui
                            .selectable_label(sel, format!("{:>3}. {}", i + 1, describe(a, s)))
                            .clicked()
                        {
                            self.hist_sel = if sel { None } else { Some(i) };
                        }
                    }
                });
        });
        if let Some(i) = self.hist_sel {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}番目を選んでいます:", i + 1));
                if ui
                    .button("ここから入れ直す")
                    .on_hover_text("この操作とそのあとを取り消します（やり直しボタンで戻せます）")
                    .clicked()
                {
                    self.rewind(log, i, net);
                }
                if replace_tile(&log[i], Tile::UNKNOWN).is_some()
                    && ui
                        .button("この牌を直す")
                        .on_hover_text("正しい牌をパレットで選ぶと、この操作を直して、そのあとの操作をやり直します")
                        .clicked()
                {
                    self.hist_fix = Some((i, log.to_vec()));
                }
                if ui.button("選ぶのをやめる").clicked() {
                    self.hist_sel = None;
                }
            });
        }
    }
}

/// ポン・カンができるか（ロンは分からないので、いつでも選べる）
fn can_call(s: &TableState, seat: SeatId, kind: CallKind, called: Tile) -> bool {
    let hand = &s.seats[seat.index()].hand;
    match kind {
        CallKind::Pon => !hands::pon_options(hand, called).is_empty(),
        CallKind::Kan => hands::minkan_tiles(hand, called).is_some(),
        CallKind::Ron => true,
    }
}

/// 操作の牌を差し替える（牌のない操作は None）
fn replace_tile(a: &Action, t: Tile) -> Option<Action> {
    Some(match a {
        Action::Tsumo { seat, .. } => Action::Tsumo {
            seat: *seat,
            tile: t,
        },
        Action::Discard { seat, riichi, .. } => Action::Discard {
            seat: *seat,
            tile: t,
            riichi: *riichi,
        },
        Action::SetDora { tiles } if !tiles.is_empty() => {
            let mut v = tiles.clone();
            *v.last_mut().unwrap() = t;
            Action::SetDora { tiles: v }
        }
        Action::Kan {
            seat,
            kind,
            new_dora_indicator: Some(_),
            tiles,
        } => Action::Kan {
            seat: *seat,
            kind: *kind,
            new_dora_indicator: Some(t),
            tiles: tiles.clone(),
        },
        _ => return None,
    })
}

/// 履歴に出す説明
fn describe(a: &Action, s: &TableState) -> String {
    let who = |x: &SeatId| seat_name(s, *x);
    match a {
        Action::SetHand { seat, tiles, drawn } => format!(
            "{}: 手牌 {}{}",
            who(seat),
            tiles_text(tiles),
            drawn
                .map(|d| format!(" ＋ {}", d.name_ja()))
                .unwrap_or_default()
        ),
        Action::Tsumo { seat, tile } => format!("{}: ツモ {}", who(seat), tile.name_ja()),
        Action::Discard { seat, tile, riichi } => format!(
            "{}: 打牌 {}{}",
            who(seat),
            tile.name_ja(),
            if *riichi { "（リーチ）" } else { "" }
        ),
        Action::Call { seat, kind, tiles } => format!(
            "{}: {} {}",
            who(seat),
            if *kind == MeldKind::Chi {
                "チー"
            } else {
                "ポン"
            },
            tiles_text(tiles)
        ),
        Action::Kan {
            seat,
            kind,
            new_dora_indicator,
            ..
        } => format!(
            "{}: {}{}",
            who(seat),
            match kind {
                KanKind::Ankan => "暗槓",
                KanKind::Minkan => "大明槓",
                KanKind::Kakan => "加槓",
            },
            new_dora_indicator
                .map(|d| format!("（新ドラ表示牌 {}）", d.name_ja()))
                .unwrap_or_default()
        ),
        Action::SetDora { tiles } => format!("ドラ表示牌 {}", tiles_text(tiles)),
        Action::Riichi { seat } => format!("{}: リーチ", who(seat)),
        Action::CancelRiichi { seat } => format!("{}: リーチの取り消し", who(seat)),
        Action::SetRiver { seat, .. } => format!("{}: 河の修正", who(seat)),
        Action::WallDecrement => "残り −1".into(),
        Action::WallIncrement => "残り +1".into(),
        Action::SetWallRemaining { value } => format!("残り {value}"),
        Action::SetDice { .. } => "サイコロの修正".into(),
        Action::Win { .. } => "アガリ".into(),
        Action::Draw { .. } => "流局".into(),
        Action::ManualAdjust { reason, .. } => format!("点数の調整（{reason}）"),
        other => format!("{other:?}").chars().take(40).collect(),
    }
}

/// テンキーの図。クリックされたキーを返す。
fn numpad(
    ui: &mut egui::Ui,
    labels: &[(NumKey, KeyLabel)],
    tex: &dyn Fn(Tile) -> Option<egui::TextureId>,
) -> Option<NumKey> {
    let cell = egui::vec2(78.0, 58.0);
    let gap = 4.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(4.0 * cell.x + 3.0 * gap, 5.0 * cell.y + 4.0 * gap),
        egui::Sense::hover(),
    );
    // (キー, 表示, 列, 行, 幅, 高さ)
    type Key = (Option<NumKey>, &'static str, f32, f32, f32, f32);
    let keys: [Key; 17] = [
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
            match l.tile.and_then(|t| tex(t).map(|id| (t, id))) {
                Some((t, id)) => {
                    // 牌の絵（不明の牌は裏に「?」）
                    let th = (kr.height() - 12.0).min(46.0);
                    let tw = th * 0.75;
                    let tr = egui::Rect::from_center_size(
                        kr.center() + egui::vec2(0.0, 4.0),
                        egui::vec2(tw, th),
                    );
                    p.image(
                        id,
                        tr,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    if t.is_unknown() {
                        p.text(
                            tr.center(),
                            egui::Align2::CENTER_CENTER,
                            "?",
                            egui::FontId::proportional(22.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
                None => {
                    let text = match l.tile {
                        Some(t) => t.name_ja(),
                        None => l.text.clone(),
                    };
                    let galley = p.layout(
                        text,
                        egui::FontId::proportional(14.0),
                        visuals.strong_text_color(),
                        kr.width() - 6.0,
                    );
                    let pos = kr.center() - galley.size() / 2.0 + egui::vec2(0.0, 5.0);
                    p.galley(pos, galley, visuals.strong_text_color());
                }
            }
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

    /// パレットを合わせて数字を押す（すべての牌のパレット: 萬子=0…字牌=3）
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
        // 局の開始: ドラ表示牌は飛ばす
        press(&mut ti, &f, &[Enter]);
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
        assert!(f.0.borrow().seats[0].hand_known, "{:?}", ti.msg);
        // B は不明の牌（字牌のパレットの 0）を1枚入れる。打ち間違いは − で消す
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
                (3, 0),
            ],
        ));
        press(&mut ti, &f, &b);
        assert!(f.0.borrow().seats[1].hand.contains(&Tile::UNKNOWN));
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
        // 打牌のパレットは手牌から: A の手牌に 9萬 はない
        ti.palette = 0;
        press(&mut ti, &f, &[Digit(9)]);
        assert!(ti.msg.is_some());
        // 親: ツモ切り（Enter）→ 鳴きなし（Enter）→ B のツモ 9m → リーチ（+）してツモ切り（Enter）
        let mut k = vec![Enter, Enter];
        pal = 0;
        ti.palette = 0;
        k.extend(tiles(&mut pal, &[(0, 9)]));
        k.extend([Add, Enter]);
        press(&mut ti, &f, &k);
        let st = f.0.borrow().clone();
        assert!(st.seats[1].riichi && st.seats[1].double_riichi && st.seats[1].ippatsu);
        assert_eq!(st.turn, Turn::AfterDiscard { seat: SeatId(1) });
        // ポンできる人がいない → 「できる人がいません」
        press(&mut ti, &f, &[Digit(2)]);
        assert!(ti.msg.is_some());
        // 鳴きなし → C のツモ 東 → 打牌 東（手牌のパレットで字牌）
        let mut k = vec![Enter];
        pal = ti.palette;
        k.extend(tiles(&mut pal, &[(3, 1)]));
        press(&mut ti, &f, &k);
        // 打牌の番になると、パレットはツモ牌（東）の色
        assert_eq!(ti.palette, 3);
        press(&mut ti, &f, &[Digit(1)]);
        // A だけが 東 をポンできるので、2 だけで決まる
        press(&mut ti, &f, &[Digit(2)]);
        let st = f.0.borrow().clone();
        assert_eq!(st.seats[0].melds.len(), 1, "{:?}", ti.msg);
        assert!(!st.seats[1].ippatsu, "鳴きで一発は消える");
        assert_eq!(st.turn, Turn::Discard { seat: SeatId(0) });
        // A が 南 を打つ → D がロン（4 → A から見て D は上家=3）→ アガリの欄を埋める
        ti.palette = 3;
        let o = press(&mut ti, &f, &[Digit(2), Digit(4), Digit(3)]);
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

    #[test]
    fn minkan_dora_after_discard() {
        use janscope_core::tile::parse_tiles;
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
        let hands = [
            ("123m456p789s1122z", Some("7z")),
            ("777z11m234p567s99s", None),
            ("456m11p23s666z333z", None),
            ("789m999p111s4444z", None),
        ];
        for (i, (h, d)) in hands.iter().enumerate() {
            apply(
                &mut s,
                &Action::SetHand {
                    seat: SeatId(i as u8),
                    tiles: parse_tiles(h).unwrap(),
                    drawn: d.map(|x| x.parse().unwrap()),
                },
            )
            .unwrap();
        }
        let f = Fake(RefCell::new(s));
        let mut ti = TileInput::default();
        // A が 中 を打つ → B だけが大明槓できるので 3 だけで決まる
        ti.palette = 3;
        press(&mut ti, &f, &[Digit(7)]);
        press(&mut ti, &f, &[Digit(3)]);
        let st = f.0.borrow().clone();
        assert_eq!(
            st.turn,
            Turn::Tsumo {
                seat: SeatId(1),
                rinshan: true
            },
            "{:?}",
            ti.msg
        );
        assert_eq!(st.kan_dora_pending, 1);
        // 嶺上牌 1索 → 打牌 1索（Enter でツモ切り）→ 新ドラ表示牌を入れる段階
        ti.palette = 2;
        press(&mut ti, &f, &[Digit(1), Enter]);
        let st = f.0.borrow().clone();
        assert_eq!(ti.step(&st, TileMode::Manual), Step::DoraReveal);
        ti.palette = 1;
        press(&mut ti, &f, &[Digit(5)]);
        let st = f.0.borrow().clone();
        assert_eq!(st.kan_dora_pending, 0);
        assert_eq!(st.dora_indicators.len(), 1);
        assert_eq!(
            ti.step(&st, TileMode::Manual),
            Step::Call {
                discarder: SeatId(1)
            }
        );
    }
}

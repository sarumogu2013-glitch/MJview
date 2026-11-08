//! ロールごとの状態の見せ方（設計書 3.2・4.8）。
//!
//! 選手用画面ロールには `PublicView` だけを送る。`PublicView` は手牌（アガリの入力・面子分解）や
//! 認識データを持たない別の型なので、型の上で漏れようがない。項目を足すときは「卓上で公開されている情報か」を確認する。

use serde::{Deserialize, Serialize};

use crate::scoring::{Limit, YakuItem};
use crate::state::{DrawKind, FinalResult, HandOutcome, HandRecord, Phase, TableState};
use crate::tile::{SeatId, Tile, Wind};

/// 配信・入力担当ロール向け（全情報）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FullView {
    pub state: TableState,
    pub can_undo: bool,
    pub can_redo: bool,
    /// 大会通算（配信・入力担当ロールだけに送る）
    #[serde(default)]
    pub tournament: crate::tournament::Tournament,
    /// 今の局の操作（局の開始のあとから。入力担当が遡って直すのに使う）
    #[serde(default)]
    pub hand_log: Vec<crate::state::Action>,
    /// サーバーがデモモード（ダミー履歴）で動いている
    #[serde(default)]
    pub demo: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicSeat {
    pub name: String,
    pub affiliation: Option<String>,
    pub wind: Wind,
    pub score: i32,
    pub rank: u8,
    pub riichi: bool,
    pub is_dealer: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicWin {
    pub winner: SeatId,
    pub loser: Option<SeatId>,
    pub yaku: Vec<YakuItem>,
    pub dora: u32,
    pub aka: u32,
    pub ura: u32,
    pub han: u32,
    pub fu: u32,
    pub limit: Option<Limit>,
    pub han_fu_text: String,
    pub points_text: String,
    /// アガリ後に公開される裏ドラ表示牌
    pub ura_indicators: Vec<Tile>,
    pub deltas: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PublicOutcome {
    Win(Vec<PublicWin>),
    Draw { tenpai: Vec<SeatId>, kind: DrawKind },
    Adjust { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicHandResult {
    pub wind: Wind,
    pub number: u8,
    pub honba: u32,
    pub outcome: PublicOutcome,
    pub deltas: [i32; 4],
}

/// 選手用画面ロール向け（公開情報だけ）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicView {
    pub phase: Phase,
    pub round_wind: Wind,
    pub round_number: u8,
    pub honba: u32,
    pub kyotaku: u32,
    pub all_last: bool,
    pub seats: [PublicSeat; 4],
    pub dora_indicators: Vec<Tile>,
    pub wall_remaining: u32,
    pub dice: Vec<(u8, u8)>,
    pub last_deltas: [i32; 4],
    pub last_result: Option<PublicHandResult>,
    pub final_results: Option<Vec<FinalResult>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TableStateView {
    Full(Box<FullView>),
    Public(Box<PublicView>),
}

pub fn public_result(h: &HandRecord) -> PublicHandResult {
    let outcome = match &h.outcome {
        HandOutcome::Win(ws) => PublicOutcome::Win(
            ws.iter()
                .map(|w| PublicWin {
                    winner: w.input.winner,
                    loser: w.input.loser,
                    yaku: w.score.yaku.clone(),
                    dora: w.score.dora,
                    aka: w.score.aka,
                    ura: w.score.ura,
                    han: w.score.han,
                    fu: w.score.fu,
                    limit: w.score.limit,
                    han_fu_text: w.score.han_fu_text(),
                    points_text: w.score.points_text(),
                    ura_indicators: if w.input.riichi || w.input.double_riichi {
                        w.input.ura_indicators.clone()
                    } else {
                        vec![]
                    },
                    deltas: w.deltas,
                })
                .collect(),
        ),
        HandOutcome::Draw { tenpai, kind } => PublicOutcome::Draw {
            tenpai: tenpai.clone(),
            kind: kind.clone(),
        },
        HandOutcome::Adjust { reason } => PublicOutcome::Adjust {
            reason: reason.clone(),
        },
    };
    PublicHandResult {
        wind: h.wind,
        number: h.number,
        honba: h.honba,
        outcome,
        deltas: h.deltas,
    }
}

impl PublicView {
    pub fn from_state(s: &TableState) -> Self {
        let ranks = s.ranks();
        let dealer = s.dealer();
        let seat = |i: usize| {
            let st = &s.seats[i];
            PublicSeat {
                name: st.player.name.clone(),
                affiliation: st.player.affiliation.clone(),
                wind: s.seat_wind(SeatId(i as u8)),
                score: st.score,
                rank: ranks[i],
                riichi: st.riichi,
                is_dealer: dealer.index() == i,
            }
        };
        PublicView {
            phase: s.phase,
            round_wind: s.round_wind,
            round_number: s.round_number,
            honba: s.honba,
            kyotaku: s.kyotaku,
            all_last: s.is_all_last(),
            seats: [seat(0), seat(1), seat(2), seat(3)],
            dora_indicators: s.dora_indicators.clone(),
            wall_remaining: s.wall_remaining,
            dice: s.dice.clone(),
            last_deltas: s.last_deltas,
            last_result: s.history.last().map(public_result),
            final_results: s.final_results.clone(),
        }
    }
}

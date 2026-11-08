//! 卓状態（設計書 6章 TableState）と操作（Action）。

use serde::{Deserialize, Serialize};

use crate::rules::GameConfig;
use crate::scoring::{Meld, WinInput, WinScore};
use crate::tile::{SeatId, Tile, Wind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    #[serde(default)]
    pub affiliation: Option<String>,
}

impl Player {
    pub fn new(name: impl Into<String>) -> Self {
        Player {
            name: name.into(),
            affiliation: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KanKind {
    Ankan,
    Minkan,
    Kakan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AbortiveKind {
    /// 九種九牌
    KyuushuKyuuhai,
    /// 四風連打
    SuufonRenda,
    /// 四開槓
    SuukanSanra,
    /// 四家立直
    SuuchaRiichi,
    /// 三家和
    SanchaHou,
}

impl AbortiveKind {
    pub fn name_ja(self) -> &'static str {
        match self {
            AbortiveKind::KyuushuKyuuhai => "九種九牌",
            AbortiveKind::SuufonRenda => "四風連打",
            AbortiveKind::SuukanSanra => "四開槓",
            AbortiveKind::SuuchaRiichi => "四家立直",
            AbortiveKind::SanchaHou => "三家和",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawKind {
    /// 荒牌流局（テンパイ者は Action::Draw の tenpai）
    Exhaustive,
    /// 流し満貫（達成者）
    NagashiMangan(Vec<SeatId>),
    /// 途中流局
    Abortive(AbortiveKind),
}

/// 対局の操作。すべてログに残り、Undo の対象になる（設計書 5.5）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Action {
    /// 半荘開始。players は席A〜Dに座る選手、chiicha は起家の席
    StartGame {
        config: GameConfig,
        players: [Player; 4],
        chiicha: SeatId,
    },
    /// 局開始（サイコロの出目。残り牌数を初期値に、ドラ・リーチ状態をリセット）
    StartHand {
        dice: Vec<(u8, u8)>,
    },
    WallDecrement,
    WallIncrement,
    SetWallRemaining {
        value: u32,
    },
    Riichi {
        seat: SeatId,
    },
    /// 立直の取り消し（宣言牌が通らずロンされた場合など、供託を戻す）
    CancelRiichi {
        seat: SeatId,
    },
    /// 槓（残り牌数 −1）。tiles は牌の手入力のとき、手牌から出す牌（暗槓4枚・大明槓3枚・加槓1枚）。
    /// 空なら残り牌数とドラだけを扱う（手牌を追っていないとき）
    Kan {
        seat: SeatId,
        kind: KanKind,
        new_dora_indicator: Option<Tile>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tiles: Vec<Tile>,
    },
    SetDora {
        tiles: Vec<Tile>,
    },
    SetDice {
        dice: Vec<(u8, u8)>,
    },
    /// アガリの確定（ダブロンは複数）。計算は engine が行う
    Win {
        wins: Vec<WinInput>,
    },
    /// 流局
    Draw {
        tenpai: Vec<SeatId>,
        kind: DrawKind,
    },
    /// 点数の手動調整（チョンボなど。理由をログに残す）
    ManualAdjust {
        deltas: [i32; 4],
        reason: String,
    },
    /// 局・本場・供託の手動修正
    SetRound {
        wind: Wind,
        number: u8,
        honba: u32,
        kyotaku: u32,
    },
    /// 選手の名前などの修正（対局中の誤字修正用）
    SetPlayer {
        seat: SeatId,
        player: Player,
    },
    /// 半荘を途中で打ち切って終了（時間切れなど）
    EndGame,
    /// 手牌の登録・修正（牌の手入力。配牌の登録にも使う）。drawn はツモ牌（親の配牌の14枚目など）
    SetHand {
        seat: SeatId,
        tiles: Vec<Tile>,
        #[serde(default)]
        drawn: Option<Tile>,
    },
    /// ツモ（牌の手入力）。残り牌数を1減らす（槓のあとの嶺上牌は減らさない）
    Tsumo {
        seat: SeatId,
        tile: Tile,
    },
    /// 打牌（牌の手入力）。riichi なら立直の宣言牌（立直と同じく供託が入る）
    Discard {
        seat: SeatId,
        tile: Tile,
        #[serde(default)]
        riichi: bool,
    },
    /// チー・ポン（牌の手入力）。直前の打牌を鳴く。tiles は手牌から出す2枚
    Call {
        seat: SeatId,
        kind: crate::scoring::MeldKind,
        tiles: Vec<Tile>,
    },
    /// 河の修正（牌の手入力）
    SetRiver {
        seat: SeatId,
        river: Vec<RiverTile>,
    },
    /// 対局中にルールを差し替える（設定UIのルールタブ。点数は変えず、以後の計算と進行に使う）
    SetConfig {
        config: GameConfig,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeatState {
    pub player: Player,
    pub score: i32,
    pub riichi: bool,
    /// 副露（手入力・自動読み取りの結果）
    #[serde(default)]
    pub melds: Vec<Meld>,
    /// 手牌（門前部分。ツモ牌は含めない）。牌の手入力・自動読み取りの結果
    #[serde(default)]
    pub hand: Vec<Tile>,
    /// ツモ牌（打牌するまで手牌とは分けて持つ）
    #[serde(default)]
    pub drawn: Option<Tile>,
    #[serde(default)]
    pub river: Vec<RiverTile>,
    /// この局の手牌が登録済み（配牌の登録が終わった）
    #[serde(default)]
    pub hand_known: bool,
    /// 一発の権利がある（立直の宣言牌を打ってから、自分の次の打牌・誰かの鳴きまで）
    #[serde(default)]
    pub ippatsu: bool,
    /// ダブル立直（鳴きのない1巡目に立直した）
    #[serde(default)]
    pub double_riichi: bool,
}

impl SeatState {
    /// 手牌＋ツモ牌＋副露（槓も3枚と数える）の枚数。打牌の前は14、後は13
    pub fn tile_count(&self) -> usize {
        self.hand.len() + self.drawn.is_some() as usize + 3 * self.melds.len()
    }
}

/// 河の牌1枚。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiverTile {
    pub tile: Tile,
    /// 立直の宣言牌（横向き）
    #[serde(default)]
    pub riichi: bool,
    /// 鳴かれた（暗く表示）
    #[serde(default)]
    pub called: bool,
    /// ツモ切り
    #[serde(default)]
    pub tsumogiri: bool,
}

/// 牌の手入力で、次に入れるもの（engine が操作のたびに決める）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Turn {
    /// 牌を追っていない、または配牌の登録中
    #[default]
    None,
    /// seat の打牌（ツモのあと・鳴いたあと）
    Discard { seat: SeatId },
    /// seat が打牌した直後（鳴き・ロン、なければ下家のツモ）
    AfterDiscard { seat: SeatId },
    /// seat のツモ（rinshan: 槓のあとの嶺上牌）
    Tsumo { seat: SeatId, rinshan: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// 半荘が始まっていない
    Setup,
    /// 局の開始待ち（サイコロ入力待ち）
    BetweenHands,
    InHand,
    GameOver,
}

/// 1件のアガリの結果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WinRecord {
    pub input: WinInput,
    pub score: WinScore,
    /// この人のアガリで動いた点（本場・供託込み）
    pub deltas: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HandOutcome {
    Win(Vec<WinRecord>),
    Draw { tenpai: Vec<SeatId>, kind: DrawKind },
    Adjust { reason: String },
}

/// 終わった局の記録。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandRecord {
    pub wind: Wind,
    pub number: u8,
    pub honba: u32,
    pub kyotaku_before: u32,
    pub outcome: HandOutcome,
    /// 合計の点数移動
    pub deltas: [i32; 4],
    pub scores_after: [i32; 4],
    pub dora_indicators: Vec<Tile>,
}

/// 最終結果（1人分）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FinalResult {
    pub seat: SeatId,
    pub rank: u8,
    pub score: i32,
    /// ウマ・オカ込みのポイント（小数1桁）
    pub points: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableState {
    pub config: GameConfig,
    pub phase: Phase,
    pub round_wind: Wind,
    /// 1〜4
    pub round_number: u8,
    /// 起家
    pub chiicha: SeatId,
    pub honba: u32,
    /// 供託リーチ棒の本数
    pub kyotaku: u32,
    /// 物理的な席A〜D
    pub seats: [SeatState; 4],
    pub dice: Vec<(u8, u8)>,
    pub dora_indicators: Vec<Tile>,
    pub wall_remaining: u32,
    pub kans_this_hand: u8,
    /// 終わった局の一覧
    pub history: Vec<HandRecord>,
    /// 直前の局の点数移動（±表示用）
    pub last_deltas: [i32; 4],
    pub final_results: Option<Vec<FinalResult>>,
    /// 西入・延長中
    pub extension: bool,
    /// 牌の手入力の手番
    #[serde(default)]
    pub turn: Turn,
    /// この局で鳴き・槓があった（1巡目の役・一発の判定に使う）
    #[serde(default)]
    pub calls_this_hand: bool,
    /// 今のツモ牌は嶺上牌（嶺上開花の判定）
    #[serde(default)]
    pub rinshan_drawn: bool,
    /// 打牌のあとにめくる槓ドラの枚数（大明槓・加槓。暗槓はすぐめくる）
    #[serde(default)]
    pub kan_dora_pending: u8,
}

impl Default for TableState {
    fn default() -> Self {
        let p = || SeatState {
            player: Player::new(""),
            score: 0,
            riichi: false,
            melds: vec![],
            hand: vec![],
            drawn: None,
            river: vec![],
            hand_known: false,
            ippatsu: false,
            double_riichi: false,
        };
        TableState {
            config: GameConfig::default(),
            phase: Phase::Setup,
            round_wind: Wind::East,
            round_number: 1,
            chiicha: SeatId(0),
            honba: 0,
            kyotaku: 0,
            seats: [p(), p(), p(), p()],
            dice: vec![],
            dora_indicators: vec![],
            wall_remaining: 0,
            kans_this_hand: 0,
            history: vec![],
            last_deltas: [0; 4],
            final_results: None,
            extension: false,
            turn: Turn::None,
            calls_this_hand: false,
            rinshan_drawn: false,
            kan_dora_pending: 0,
        }
    }
}

impl TableState {
    /// 現在の親の席。
    pub fn dealer(&self) -> SeatId {
        SeatId((self.chiicha.0 + self.round_number - 1) % 4)
    }
    /// 席の自風。
    pub fn seat_wind(&self, seat: SeatId) -> Wind {
        Wind::from_index(seat.distance_from(self.dealer()))
    }
    pub fn scores(&self) -> [i32; 4] {
        [
            self.seats[0].score,
            self.seats[1].score,
            self.seats[2].score,
            self.seats[3].score,
        ]
    }
    /// 「東2局」
    pub fn round_label(&self) -> String {
        format!("{}{}局", self.round_wind.name_ja(), self.round_number)
    }
    /// 現在の順位（1〜4）。同点は起家に近い方が上（表示用。最終順位はルールの同点処理に従う）
    pub fn ranks(&self) -> [u8; 4] {
        let order = self.rank_order();
        let mut r = [0u8; 4];
        for (i, s) in order.iter().enumerate() {
            r[s.index()] = i as u8 + 1;
        }
        r
    }
    /// 点数の高い順の席（同点は起家に近い方が先）。
    pub fn rank_order(&self) -> Vec<SeatId> {
        let mut v: Vec<SeatId> = SeatId::ALL.to_vec();
        v.sort_by_key(|s| (-self.seats[s.index()].score, s.distance_from(self.chiicha)));
        v
    }
    /// オーラス（規定の最終局）か。
    pub fn is_all_last(&self) -> bool {
        !self.extension
            && self.round_wind == self.config.last_regular_wind()
            && self.round_number == 4
    }
}

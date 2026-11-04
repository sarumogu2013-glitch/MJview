//! アガリの点数計算（設計書 4.5・5.2）。
//!
//! 手順:
//! 1. 門前部分（アガリ牌を含む）を面子分解する（`decompose`）
//! 2. 分解ごと・アガリ牌を含む組ごとに mahc の `Hand` を作り、役と符を求める
//! 3. ドラ・赤・裏ドラ・大会ルール（喰いタン、連風牌の符、切り上げ満貫、数え役満、ダブル役満…）を本ソフト側で反映して点数を出す
//! 4. いちばん高い解釈を採用する（高点法）
//!
//! 本場・供託・ダブロンの精算は `engine` 側で行う。

pub mod decompose;
pub mod yaku_ja;

use mahc::fu::Fu;
use mahc::hand::Hand;
use mahc::yaku::Yaku;
use serde::{Deserialize, Serialize};

use crate::rules::{DoubleWindPairFu, GameConfig};
use crate::tile::{SeatId, Tile, Wind};
use decompose::{Group, GroupKind, Shape, decompose};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeldKind {
    Chi,
    Pon,
    /// 大明槓
    Minkan,
    Ankan,
    Kakan,
}

impl MeldKind {
    pub fn is_kan(self) -> bool {
        matches!(self, MeldKind::Minkan | MeldKind::Ankan | MeldKind::Kakan)
    }
    pub fn is_open(self) -> bool {
        !matches!(self, MeldKind::Ankan)
    }
}

/// 副露（面子単位）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meld {
    pub kind: MeldKind,
    pub tiles: Vec<Tile>,
    /// 鳴いた相手（暗槓は None）。表示で横向きにする牌の位置に使う
    #[serde(default)]
    pub from: Option<SeatId>,
}

/// アガリの入力（設計書 4.5）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinInput {
    pub winner: SeatId,
    /// 放銃者。None ならツモ
    pub loser: Option<SeatId>,
    /// 門前部分の牌（アガリ牌を含む）
    pub closed: Vec<Tile>,
    #[serde(default)]
    pub melds: Vec<Meld>,
    pub win_tile: Tile,
    #[serde(default)]
    pub riichi: bool,
    #[serde(default)]
    pub double_riichi: bool,
    #[serde(default)]
    pub ippatsu: bool,
    /// 海底摸月（ツモ）／河底撈魚（ロン）。残り牌数が0なら engine が自動で立てる
    #[serde(default)]
    pub haitei: bool,
    #[serde(default)]
    pub rinshan: bool,
    #[serde(default)]
    pub chankan: bool,
    /// 天和（親）／地和（子）
    #[serde(default)]
    pub tenhou: bool,
    /// 裏ドラ表示牌（立直していなければ無視）
    #[serde(default)]
    pub ura_indicators: Vec<Tile>,
    /// 包（パオ）の責任者
    #[serde(default)]
    pub pao: Option<SeatId>,
}

impl WinInput {
    pub fn is_tsumo(&self) -> bool {
        self.loser.is_none()
    }
    pub fn all_tiles(&self) -> Vec<Tile> {
        let mut v = self.closed.clone();
        for m in &self.melds {
            v.extend(&m.tiles);
        }
        v
    }
}

/// 計算に必要な場の情報。
#[derive(Clone, Debug)]
pub struct ScoreContext<'a> {
    pub config: &'a GameConfig,
    pub round_wind: Wind,
    pub seat_wind: Wind,
    pub dora_indicators: &'a [Tile],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YakuItem {
    pub name: String,
    /// 翻数（役満なら0）
    pub han: u32,
    /// 役満の倍数（通常役は0）
    pub yakuman: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuItem {
    pub name: String,
    pub fu: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Limit {
    Mangan,
    Haneman,
    Baiman,
    Sanbaiman,
    /// 役満（倍数。数え役満も1）
    Yakuman(u32),
}

impl Limit {
    pub fn name_ja(self) -> String {
        match self {
            Limit::Mangan => "満貫".into(),
            Limit::Haneman => "跳満".into(),
            Limit::Baiman => "倍満".into(),
            Limit::Sanbaiman => "三倍満".into(),
            Limit::Yakuman(1) => "役満".into(),
            Limit::Yakuman(2) => "ダブル役満".into(),
            Limit::Yakuman(3) => "トリプル役満".into(),
            Limit::Yakuman(n) => format!("{n}倍役満"),
        }
    }
}

/// アガリの計算結果（本場・供託を含まない）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinScore {
    pub yaku: Vec<YakuItem>,
    pub dora: u32,
    pub aka: u32,
    pub ura: u32,
    /// 合計翻数（役満なら0）
    pub han: u32,
    pub fu: u32,
    pub fu_items: Vec<FuItem>,
    pub limit: Option<Limit>,
    pub base_points: i64,
    pub is_dealer: bool,
    pub tsumo: bool,
    /// ロン: 放銃者が払う点（本場なし）
    pub ron_points: i32,
    /// ツモ: 親が払う点（子のツモ）／各自が払う点（親のツモ）
    pub tsumo_dealer_pays: i32,
    /// ツモ: 子が払う点（子のツモ）
    pub tsumo_child_pays: i32,
    /// 採用した面子分解（表示用。各組の牌、副露は含まない）
    pub groups: Vec<Vec<Tile>>,
}

impl WinScore {
    /// 本場なしの、アガった人の収入。
    pub fn total(&self) -> i32 {
        if !self.tsumo {
            self.ron_points
        } else if self.is_dealer {
            self.tsumo_dealer_pays * 3
        } else {
            self.tsumo_dealer_pays + self.tsumo_child_pays * 2
        }
    }

    /// 「8,000」「2,000-4,000」「4,000オール」のような表示。
    pub fn points_text(&self) -> String {
        if !self.tsumo {
            fmt_pts(self.ron_points)
        } else if self.is_dealer {
            format!("{}オール", fmt_pts(self.tsumo_dealer_pays))
        } else {
            format!(
                "{}-{}",
                fmt_pts(self.tsumo_child_pays),
                fmt_pts(self.tsumo_dealer_pays)
            )
        }
    }

    /// 「3翻40符」「満貫」「役満」のような表示。
    pub fn han_fu_text(&self) -> String {
        match self.limit {
            Some(Limit::Yakuman(_)) if self.han == 0 => self.limit.unwrap().name_ja(),
            Some(l) => format!("{}翻 {}", self.han, l.name_ja()),
            None => format!("{}翻{}符", self.han, self.fu),
        }
    }
}

pub fn fmt_pts(p: i32) -> String {
    let s = p.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if p < 0 { format!("-{out}") } else { out }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ScoreError {
    #[error("牌の枚数が合いません（門前 {closed} 枚・副露 {melds} 組。門前は 14 − 3×副露 枚）")]
    TileCount { closed: usize, melds: usize },
    #[error("{0} が5枚以上あります")]
    TooMany(String),
    #[error("アガリ牌 {0} が手牌に含まれていません")]
    WinTileMissing(String),
    #[error("副露の形が不正です: {0}")]
    InvalidMeld(String),
    #[error("アガリの形になっていません")]
    NotWinning,
    #[error("役がありません")]
    NoYaku,
    #[error("状況役の組み合わせが不正です: {0}")]
    Conditions(String),
}

/// アガリを計算する（高点法）。
pub fn score_win(input: &WinInput, ctx: &ScoreContext) -> Result<WinScore, ScoreError> {
    validate(input)?;
    let n_melds = input.melds.len();
    let shapes = decompose(&input.closed, n_melds);
    if shapes.is_empty() {
        return Err(ScoreError::NotWinning);
    }
    let win_kind = input.win_tile.kind();
    let mut best: Option<WinScore> = None;
    let mut last_err = ScoreError::NoYaku;
    for shape in &shapes {
        let groups = shape.groups();
        for (wi, g) in groups.iter().enumerate() {
            if !g.contains(win_kind) {
                continue;
            }
            // 同じ形の組を何度も試さない
            if groups[..wi].iter().any(|h| h == g) {
                continue;
            }
            match score_one(input, ctx, shape, wi) {
                Ok(s) => {
                    let better = match &best {
                        None => true,
                        Some(b) => (s.total(), s.han, s.fu) > (b.total(), b.han, b.fu),
                    };
                    if better {
                        best = Some(s);
                    }
                }
                Err(e) => last_err = e,
            }
        }
    }
    best.ok_or(last_err)
}

fn validate(input: &WinInput) -> Result<(), ScoreError> {
    let n = input.melds.len();
    if n > 4 || input.closed.len() + 3 * n != 14 {
        return Err(ScoreError::TileCount {
            closed: input.closed.len(),
            melds: n,
        });
    }
    let mut c = [0u8; 34];
    for t in input.all_tiles() {
        c[t.kind() as usize] += 1;
    }
    if let Some(k) = (0..34).find(|&k| c[k] > 4) {
        return Err(ScoreError::TooMany(Tile::new(k as u8).name_ja()));
    }
    if !input
        .closed
        .iter()
        .any(|t| t.kind() == input.win_tile.kind())
    {
        return Err(ScoreError::WinTileMissing(input.win_tile.name_ja()));
    }
    for m in &input.melds {
        let ks: Vec<u8> = m.tiles.iter().map(|t| t.kind()).collect();
        let ok = match m.kind {
            MeldKind::Chi => {
                let mut s = ks.clone();
                s.sort();
                s.len() == 3 && s[0] < 27 && s[0] % 9 <= 6 && s[1] == s[0] + 1 && s[2] == s[0] + 2
            }
            MeldKind::Pon => ks.len() == 3 && ks.iter().all(|&k| k == ks[0]),
            _ => ks.len() == 4 && ks.iter().all(|&k| k == ks[0]),
        };
        if !ok {
            return Err(ScoreError::InvalidMeld(format!(
                "{:?} {}",
                m.kind,
                m.tiles.iter().map(|t| t.to_string()).collect::<String>()
            )));
        }
    }
    if input.riichi && input.double_riichi {
        return Err(ScoreError::Conditions(
            "立直とダブル立直は同時に指定しません".into(),
        ));
    }
    if (input.riichi || input.double_riichi) && input.melds.iter().any(|m| m.kind.is_open()) {
        return Err(ScoreError::Conditions("鳴いた手で立直はできません".into()));
    }
    Ok(())
}

fn tile_str(kinds: &[u8]) -> String {
    let suit = Tile::new(kinds[0]).suit().char();
    let mut s: String = kinds
        .iter()
        .map(|&k| char::from(b'0' + Tile::new(k).number()))
        .collect();
    s.push(suit);
    s
}

fn score_one(
    input: &WinInput,
    ctx: &ScoreContext,
    shape: &Shape,
    win_group: usize,
) -> Result<WinScore, ScoreError> {
    let cfg = ctx.config;
    let groups = shape.groups();
    // mahc の Hand（副露 → 門前の組 → アガリ牌を含む組を最後に）
    let mut strs: Vec<String> = Vec::new();
    for m in &input.melds {
        let mut ks: Vec<u8> = m.tiles.iter().map(|t| t.kind()).collect();
        ks.sort();
        let mut s = tile_str(&ks);
        if m.kind.is_open() {
            s.push('o');
        }
        strs.push(s);
    }
    let mut order: Vec<&Group> = groups
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != win_group)
        .map(|(_, g)| g)
        .collect();
    order.push(&groups[win_group]);
    for g in &order {
        strs.push(tile_str(&g.kinds()));
    }
    let wt = |t: Tile| t.plain().to_string();
    let hand = Hand::new_from_strings(
        strs,
        wt(input.win_tile),
        wt(ctx.round_wind.tile()),
        wt(ctx.seat_wind.tile()),
    )
    .map_err(|_| ScoreError::NotWinning)?;

    let tsumo = input.is_tsumo();
    let riichi = input.riichi;
    let dbl = input.double_riichi;
    if let Some(e) = mahc::calc::validate_scoring_conditions(
        &hand,
        tsumo,
        riichi,
        dbl,
        input.ippatsu,
        input.haitei,
        input.rinshan,
        input.chankan,
    ) {
        return Err(ScoreError::Conditions(format!("{e:?}")));
    }
    let mut yaku = mahc::calc::get_yaku_and_yakuman(
        &hand,
        tsumo,
        riichi,
        dbl,
        input.ippatsu,
        input.haitei,
        input.rinshan,
        input.chankan,
        input.tenhou,
    );
    let open = hand.is_open();
    if !cfg.kuitan && open {
        yaku.retain(|y| *y != Yaku::Tanyao);
    }
    // 大七星はローカル役（字一色の七対子は字一色として数える）
    if yaku.contains(&Yaku::Daichiishin) {
        yaku.retain(|y| *y != Yaku::Daichiishin);
        if !yaku.contains(&Yaku::Tsuuiisou) {
            yaku.push(Yaku::Tsuuiisou);
        }
    }
    // 上位の役満があれば通常版を外す
    for (upper, lower) in [
        (Yaku::SuuankouTankiWait, Yaku::Suuankou),
        (Yaku::KokushiMusou13SidedWait, Yaku::KokushiMusou),
        (Yaku::ChuurenPoutou9SidedWait, Yaku::ChuurenPoutou),
    ] {
        if yaku.contains(&upper) {
            yaku.retain(|y| *y != lower);
        }
    }
    let is_dealer = ctx.seat_wind == Wind::East;
    let yakuman: Vec<&Yaku> = yaku.iter().filter(|y| y.is_yakuman()).collect();

    let mut items = Vec::new();
    let (mut han, mut fu, mut fu_items, limit, base, dora, aka, ura);
    if !yakuman.is_empty() {
        let mult = |y: &Yaku| -> u32 {
            let double = matches!(
                y,
                Yaku::SuuankouTankiWait
                    | Yaku::KokushiMusou13SidedWait
                    | Yaku::ChuurenPoutou9SidedWait
                    | Yaku::Daisuushii
            );
            if double && cfg.double_yakuman { 2 } else { 1 }
        };
        for y in &yakuman {
            items.push(YakuItem {
                name: yaku_ja::yaku_ja(y).into(),
                han: 0,
                yakuman: mult(y),
            });
        }
        let total = if cfg.multiple_yakuman {
            yakuman.iter().map(|y| mult(y)).sum()
        } else {
            yakuman.iter().map(|y| mult(y)).max().unwrap_or(1)
        };
        han = 0;
        fu = 0;
        fu_items = Vec::new();
        limit = Some(Limit::Yakuman(total));
        base = 8_000 * total as i64;
        dora = 0;
        aka = 0;
        ura = 0;
    } else {
        // 役牌は名前を付けて並べる
        let yakuhai = yakuhai_names(input, ctx, groups);
        let mut yakuhai_iter = yakuhai.into_iter();
        for y in &yaku {
            let name = match y {
                Yaku::Yakuhai => yakuhai_iter.next().unwrap_or_else(|| "役牌".into()),
                Yaku::Haitei if !tsumo => "河底撈魚".into(),
                _ => yaku_ja::yaku_ja(y).into(),
            };
            items.push(YakuItem {
                name,
                han: yaku_han(y, open),
                yakuman: 0,
            });
        }
        if items.is_empty() {
            return Err(ScoreError::NoYaku);
        }
        han = items.iter().map(|i| i.han).sum::<u32>();
        let all = input.all_tiles();
        dora = count_dora(&all, ctx.dora_indicators);
        aka = all.iter().filter(|t| t.is_red()).count() as u32;
        ura = if riichi || dbl {
            count_dora(&all, &input.ura_indicators)
        } else {
            0
        };
        han += dora + aka + ura;

        let fu_types = hand.calculate_fu(tsumo);
        fu_items = fu_types
            .iter()
            .map(|f| FuItem {
                name: yaku_ja::fu_ja(f).into(),
                fu: f.value() as u32,
            })
            .collect();
        // 連風牌の雀頭（mahc は2符）
        if cfg.double_wind_pair_fu == DoubleWindPairFu::Four
            && ctx.round_wind == ctx.seat_wind
            && fu_types.contains(&Fu::Toitsu)
            && groups
                .iter()
                .any(|g| g.kind == GroupKind::Pair && g.first == ctx.seat_wind.tile().kind())
        {
            fu_items.push(FuItem {
                name: "連風牌の雀頭".into(),
                fu: 2,
            });
        }
        let raw: u32 = fu_items.iter().map(|f| f.fu).sum();
        fu = if matches!(shape, Shape::Chiitoitsu(_)) {
            25
        } else {
            raw.div_ceil(10) * 10
        };
        if open && fu < 30 {
            // 鳴いた平和形のロンは30符
            fu_items.push(FuItem {
                name: "副露の平和形（30符に切り上げ）".into(),
                fu: 30 - fu,
            });
            fu = 30;
        }
        let (b, l) = base_points(han, fu, cfg);
        base = b;
        limit = l;
    }

    let ceil100 = |v: i64| ((v + 99) / 100 * 100) as i32;
    let (ron_points, tsumo_dealer_pays, tsumo_child_pays) = if is_dealer {
        (ceil100(base * 6), ceil100(base * 2), 0)
    } else {
        (ceil100(base * 4), ceil100(base * 2), ceil100(base))
    };
    let mut disp: Vec<Vec<Tile>> = order
        .iter()
        .map(|g| g.kinds().into_iter().map(Tile::new).collect())
        .collect();
    restore_reds(&mut disp, &input.closed);
    Ok(WinScore {
        yaku: items,
        dora,
        aka,
        ura,
        han,
        fu,
        fu_items,
        limit,
        base_points: base,
        is_dealer,
        tsumo,
        ron_points: if tsumo { 0 } else { ron_points },
        tsumo_dealer_pays: if tsumo { tsumo_dealer_pays } else { 0 },
        tsumo_child_pays: if tsumo && !is_dealer {
            tsumo_child_pays
        } else {
            0
        },
        groups: disp,
    })
}

/// 役の翻数。mahc 2.2.0 は混全帯幺九の喰い下がりを数えない（副露でも2翻）ので、ここで1翻に直す
/// （Python の mahjong ライブラリとの突き合わせで判明）。
fn yaku_han(y: &Yaku, open: bool) -> u32 {
    match y {
        Yaku::Chantaiyao if open => 1,
        _ => y.get_han(open),
    }
}

/// 翻・符から基本点と満貫以上の区分を出す。
pub fn base_points(han: u32, fu: u32, cfg: &GameConfig) -> (i64, Option<Limit>) {
    match han {
        13.. if cfg.kazoe_yakuman => (8_000, Some(Limit::Yakuman(1))),
        11.. => (6_000, Some(Limit::Sanbaiman)),
        8..=10 => (4_000, Some(Limit::Baiman)),
        6..=7 => (3_000, Some(Limit::Haneman)),
        5 => (2_000, Some(Limit::Mangan)),
        _ => {
            let b = fu as i64 * (1i64 << (han + 2));
            if b >= 2_000
                || (cfg.kiriage_mangan && ((han == 4 && fu == 30) || (han == 3 && fu == 60)))
            {
                (2_000, Some(Limit::Mangan))
            } else {
                (b, None)
            }
        }
    }
}

fn count_dora(tiles: &[Tile], indicators: &[Tile]) -> u32 {
    indicators
        .iter()
        .map(|ind| {
            let d = ind.dora_from_indicator().kind();
            tiles.iter().filter(|t| t.kind() == d).count() as u32
        })
        .sum()
}

fn yakuhai_names(input: &WinInput, ctx: &ScoreContext, groups: &[Group]) -> Vec<String> {
    let mut kinds: Vec<u8> = groups
        .iter()
        .filter(|g| g.kind == GroupKind::Triplet)
        .map(|g| g.first)
        .collect();
    kinds.extend(
        input
            .melds
            .iter()
            .filter(|m| m.kind != MeldKind::Chi)
            .map(|m| m.tiles[0].kind()),
    );
    kinds.sort();
    let mut out = Vec::new();
    for k in kinds {
        let t = Tile::new(k);
        if k == ctx.seat_wind.tile().kind() {
            out.push(format!("自風 {}", t.name_ja()));
        }
        if k == ctx.round_wind.tile().kind() {
            out.push(format!("場風 {}", t.name_ja()));
        }
        if k >= 31 {
            out.push(format!("役牌 {}", t.name_ja()));
        }
    }
    out
}

/// 表示用の分解に、入力の赤5を戻す。
fn restore_reds(groups: &mut [Vec<Tile>], closed: &[Tile]) {
    for red in closed.iter().filter(|t| t.is_red()) {
        'outer: for g in groups.iter_mut() {
            for t in g.iter_mut() {
                if t.kind() == red.kind() && !t.is_red() {
                    *t = *red;
                    break 'outer;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

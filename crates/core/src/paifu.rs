//! 牌譜（設計書 4.9・6章 Paifu）: 天鳳形式の JSON（各種 AI 解析ツールで共通の形式）を読み込み、1手ずつ再生する。
//!
//! 天鳳形式の要点:
//! - 牌の番号: 11〜19 萬子 / 21〜29 筒子 / 31〜39 索子 / 41〜47 東南西北白發中 / 51・52・53 赤五萬・筒・索。60 はツモ切り
//! - `log` の1要素が1局: `[[局, 本場, 供託], [持ち点×4], [ドラ表示牌], [裏ドラ表示牌], 配牌0, 取得0, 捨て0, 配牌1, …, 捨て3, [結果]]`
//!   席の番号は起家が0。局の親は「局 % 4」
//! - 取得: 数字はツモ、文字列は鳴き（"c" チー / "p" ポン / "m" 大明槓）。文字の位置が鳴いた相手（0: 上家、1: 対面、2 or 3: 下家）
//! - 捨て: 数字は打牌（60 はツモ切り）、"r" は立直宣言牌、"a" は暗槓、"k" は加槓。大明槓の後の捨ては 0（嶺上牌のツモに続く）
//! - 結果: `["和了", [点数移動×4], [アガリ, 放銃, 包, "30符1飜1000点", "役(n飜)", …], …]` や `["流局", [点数移動×4]]` など

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scoring::{Meld, MeldKind};
use crate::tile::{SeatId, Suit, Tile, Wind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaifuEvent {
    Draw {
        seat: SeatId,
        tile: Tile,
    },
    Discard {
        seat: SeatId,
        tile: Tile,
        tsumogiri: bool,
        riichi: bool,
    },
    /// チー・ポン・大明槓（鳴いた牌は直前の捨て牌）
    Call {
        seat: SeatId,
        meld: Meld,
    },
    /// 暗槓・加槓
    ConcealedKan {
        seat: SeatId,
        meld: Meld,
    },
    NewDora {
        indicator: Tile,
    },
    /// 局の終わり（アガリ・流局）。summary は表示用の文字列
    End {
        result: PaifuResult,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaifuWin {
    pub winner: SeatId,
    pub from: Option<SeatId>,
    /// 例: "30符1飜1000点"、"満貫8000点"
    pub points_text: String,
    pub yaku: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaifuResult {
    /// "和了"・"流局"・"九種九牌" など
    pub kind: String,
    pub deltas: [i32; 4],
    pub wins: Vec<PaifuWin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaifuHand {
    pub wind: Wind,
    /// 1〜4
    pub number: u8,
    pub honba: u32,
    pub kyotaku: u32,
    pub dealer: SeatId,
    pub scores_start: [i32; 4],
    pub dora_indicators: Vec<Tile>,
    pub ura_indicators: Vec<Tile>,
    pub haipai: [Vec<Tile>; 4],
    pub events: Vec<PaifuEvent>,
}

impl PaifuHand {
    pub fn label(&self) -> String {
        format!(
            "{}{}局 {}本場",
            self.wind.name_ja(),
            self.number,
            self.honba
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paifu {
    pub title: String,
    pub players: [String; 4],
    pub hands: Vec<PaifuHand>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("牌譜を読めません: {0}")]
pub struct PaifuError(pub String);

fn err<T>(m: impl Into<String>) -> Result<T, PaifuError> {
    Err(PaifuError(m.into()))
}

/// 天鳳の牌の番号 → 牌。
pub fn tile_from_code(n: i64) -> Option<Tile> {
    let suit = |d: i64| match d {
        1 => Some(Suit::Man),
        2 => Some(Suit::Pin),
        3 => Some(Suit::Sou),
        _ => None,
    };
    match n {
        11..=19 | 21..=29 | 31..=39 => Some(Tile::from_suit_number(suit(n / 10)?, (n % 10) as u8)),
        41..=47 => Some(Tile::from_suit_number(Suit::Honor, (n - 40) as u8)),
        51..=53 => Some(Tile::red_five(suit(n - 50)?)),
        _ => None,
    }
}

fn tiles(v: &Value, what: &str) -> Result<Vec<Tile>, PaifuError> {
    let Some(a) = v.as_array() else {
        return err(format!("{what} が配列ではありません"));
    };
    a.iter()
        .map(|x| {
            x.as_i64()
                .and_then(tile_from_code)
                .ok_or_else(|| PaifuError(format!("{what} の牌 {x} が不正です")))
        })
        .collect()
}

/// 鳴き・槓の文字列を、(文字, 文字の位置, 牌の列) に分ける。例: "45p4545" → ('p', 1, [45,45,45])
fn parse_call(s: &str) -> Result<(char, usize, Vec<Tile>), PaifuError> {
    let mut letter = None;
    let mut pos = 0;
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() {
            if letter.is_some() {
                return err(format!("鳴きの表記 {s} が不正です"));
            }
            letter = Some(c);
            pos = out.len();
            continue;
        }
        let d2 = chars
            .next()
            .ok_or_else(|| PaifuError(format!("鳴きの表記 {s} が不正です")))?;
        let n: i64 = format!("{c}{d2}")
            .parse()
            .map_err(|_| PaifuError(format!("鳴きの表記 {s} が不正です")))?;
        out.push(
            tile_from_code(n)
                .ok_or_else(|| PaifuError(format!("鳴きの表記 {s} の牌が不正です")))?,
        );
    }
    let l = letter.ok_or_else(|| PaifuError(format!("鳴きの表記 {s} に種類の文字がありません")))?;
    Ok((l, pos, out))
}

/// 文字の位置 → 鳴いた相手（0: 上家, 1: 対面, それ以外: 下家）
fn from_pos(seat: SeatId, pos: usize) -> SeatId {
    let back = match pos {
        0 => 3,
        1 => 2,
        _ => 1,
    };
    SeatId((seat.0 + back) % 4)
}

/// 天鳳形式の JSON を読む。
pub fn parse_tenhou6(json: &str) -> Result<Paifu, PaifuError> {
    let v: Value = serde_json::from_str(json)
        .map_err(|e| PaifuError(format!("JSON として読めません: {e}")))?;
    let names: [String; 4] = match v.get("name").and_then(|n| n.as_array()) {
        Some(a) if a.len() >= 4 => std::array::from_fn(|i| a[i].as_str().unwrap_or("").to_string()),
        _ => std::array::from_fn(|i| format!("選手{}", i + 1)),
    };
    let title = v
        .get("title")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let Some(log) = v.get("log").and_then(|l| l.as_array()) else {
        return err("log がありません");
    };
    let mut hands = Vec::new();
    for (hi, h) in log.iter().enumerate() {
        hands.push(parse_hand(h).map_err(|e| PaifuError(format!("{}局目: {}", hi + 1, e.0)))?);
    }
    Ok(Paifu {
        title,
        players: names,
        hands,
    })
}

fn parse_hand(h: &Value) -> Result<PaifuHand, PaifuError> {
    let Some(a) = h.as_array() else {
        return err("局が配列ではありません");
    };
    if a.len() < 17 {
        return err("局の要素が足りません");
    }
    let head = a[0]
        .as_array()
        .ok_or_else(|| PaifuError("局の情報が不正です".into()))?;
    let k = head.first().and_then(|x| x.as_i64()).unwrap_or(0);
    let honba = head.get(1).and_then(|x| x.as_i64()).unwrap_or(0) as u32;
    let kyotaku = head.get(2).and_then(|x| x.as_i64()).unwrap_or(0) as u32;
    let sc = a[1]
        .as_array()
        .ok_or_else(|| PaifuError("持ち点が不正です".into()))?;
    let scores_start: [i32; 4] =
        std::array::from_fn(|i| sc.get(i).and_then(|x| x.as_i64()).unwrap_or(0) as i32);
    let all_dora = tiles(&a[2], "ドラ表示牌")?;
    let ura = tiles(&a[3], "裏ドラ表示牌")?;
    let mut haipai: [Vec<Tile>; 4] = Default::default();
    let mut takes: [Vec<Value>; 4] = Default::default();
    let mut discards: [Vec<Value>; 4] = Default::default();
    for p in 0..4 {
        haipai[p] = tiles(&a[4 + p * 3], "配牌")?;
        takes[p] = a[5 + p * 3].as_array().cloned().unwrap_or_default();
        discards[p] = a[6 + p * 3].as_array().cloned().unwrap_or_default();
    }
    let result = parse_result(&a[16]);
    let dealer = SeatId((k % 4) as u8);
    let mut ev = Vec::new();
    let (mut ti, mut di) = ([0usize; 4], [0usize; 4]);
    let mut dora_shown = 1usize;
    let mut cur = dealer;
    // 手番の人の「取得」→「捨て」を交互に読む。捨て牌は他家の次の取得（鳴き）と突き合わせる
    let mut guard = 0;
    'outer: loop {
        guard += 1;
        if guard > 1000 {
            return err("手順が終わりません（牌譜が壊れている可能性があります）");
        }
        let p = cur.index();
        // 取得（ツモ。鳴いた直後は既に読んでいる）
        let Some(t) = takes[p].get(ti[p]).cloned() else {
            break;
        };
        ti[p] += 1;
        let mut rinshan_needed = false;
        match &t {
            Value::Number(n) => {
                let tile = n
                    .as_i64()
                    .and_then(tile_from_code)
                    .ok_or_else(|| PaifuError(format!("ツモ {n} が不正です")))?;
                ev.push(PaifuEvent::Draw { seat: cur, tile });
            }
            Value::String(s) => {
                // 前の捨て牌を鳴いた（取得の順は正しいが、手番の決定は前の打牌側で行う）
                let (l, pos, ts) = parse_call(s)?;
                let kind = match l {
                    'c' => MeldKind::Chi,
                    'p' => MeldKind::Pon,
                    'm' => MeldKind::Minkan,
                    _ => return err(format!("取得の鳴き {s} の種類が不正です")),
                };
                ev.push(PaifuEvent::Call {
                    seat: cur,
                    meld: Meld {
                        kind,
                        tiles: ts,
                        from: Some(from_pos(cur, pos)),
                    },
                });
                if kind == MeldKind::Minkan {
                    rinshan_needed = true;
                }
            }
            _ => return err("取得の要素が不正です"),
        }
        // 捨て（槓が続くときは嶺上牌のツモを挟む）
        loop {
            let Some(d) = discards[p].get(di[p]).cloned() else {
                break 'outer;
            };
            di[p] += 1;
            match &d {
                Value::Number(n) if n.as_i64() == Some(0) => {
                    // 大明槓の後の空き。嶺上牌をツモって次の捨てへ
                    if rinshan_needed {
                        if dora_shown < all_dora.len() {
                            ev.push(PaifuEvent::NewDora {
                                indicator: all_dora[dora_shown],
                            });
                            dora_shown += 1;
                        }
                        let Some(Value::Number(r)) = takes[p].get(ti[p]).cloned() else {
                            break 'outer;
                        };
                        ti[p] += 1;
                        let tile = r
                            .as_i64()
                            .and_then(tile_from_code)
                            .ok_or_else(|| PaifuError("嶺上牌が不正です".into()))?;
                        ev.push(PaifuEvent::Draw { seat: cur, tile });
                        rinshan_needed = false;
                    }
                    continue;
                }
                Value::Number(n) => {
                    let code = n.as_i64().unwrap_or(0);
                    let (tile, tsumogiri) = discard_tile(code, &ev, cur)?;
                    ev.push(PaifuEvent::Discard {
                        seat: cur,
                        tile,
                        tsumogiri,
                        riichi: false,
                    });
                }
                Value::String(s) if s.starts_with('r') => {
                    let code: i64 = s[1..]
                        .parse()
                        .map_err(|_| PaifuError(format!("立直の表記 {s} が不正です")))?;
                    let (tile, tsumogiri) = discard_tile(code, &ev, cur)?;
                    ev.push(PaifuEvent::Discard {
                        seat: cur,
                        tile,
                        tsumogiri,
                        riichi: true,
                    });
                }
                Value::String(s) => {
                    let (l, pos, ts) = parse_call(s)?;
                    let kind = match l {
                        'a' => MeldKind::Ankan,
                        'k' => MeldKind::Kakan,
                        _ => return err(format!("捨ての表記 {s} が不正です")),
                    };
                    let from = (kind == MeldKind::Kakan).then(|| from_pos(cur, pos));
                    ev.push(PaifuEvent::ConcealedKan {
                        seat: cur,
                        meld: Meld {
                            kind,
                            tiles: ts,
                            from,
                        },
                    });
                    if dora_shown < all_dora.len() {
                        ev.push(PaifuEvent::NewDora {
                            indicator: all_dora[dora_shown],
                        });
                        dora_shown += 1;
                    }
                    // 嶺上牌
                    let Some(Value::Number(r)) = takes[p].get(ti[p]).cloned() else {
                        break 'outer;
                    };
                    ti[p] += 1;
                    let tile = r
                        .as_i64()
                        .and_then(tile_from_code)
                        .ok_or_else(|| PaifuError("嶺上牌が不正です".into()))?;
                    ev.push(PaifuEvent::Draw { seat: cur, tile });
                    continue;
                }
                _ => return err("捨ての要素が不正です"),
            }
            break;
        }
        // 次の手番: 誰かの次の取得が今の捨て牌の鳴きならその人（ポン・カンはチーより優先）
        let discarded = match ev.last() {
            Some(PaifuEvent::Discard { tile, .. }) => *tile,
            _ => break,
        };
        let mut next = cur.next();
        let mut best: Option<(u8, SeatId)> = None;
        for k in 1..4u8 {
            let o = SeatId((cur.0 + k) % 4);
            if let Some(Value::String(s)) = takes[o.index()].get(ti[o.index()]) {
                if let Ok((l, _, ts)) = parse_call(s) {
                    if ts.iter().any(|t| t.kind() == discarded.kind()) {
                        let prio = match l {
                            'p' | 'm' => 2,
                            'c' if k == 1 => 1,
                            _ => 0,
                        };
                        if prio > 0 && best.is_none_or(|(b, _)| prio > b) {
                            best = Some((prio, o));
                        }
                    }
                }
            }
        }
        if let Some((_, o)) = best {
            next = o;
        }
        cur = next;
    }
    ev.push(PaifuEvent::End { result });
    let wind = Wind::from_index((k / 4) as u8);
    Ok(PaifuHand {
        wind,
        number: (k % 4) as u8 + 1,
        honba,
        kyotaku,
        dealer,
        scores_start,
        dora_indicators: all_dora.first().copied().into_iter().collect(),
        ura_indicators: ura,
        haipai,
        events: ev,
    })
}

/// 打牌の番号を牌にする。60 はツモ切り（直前のツモ牌）。
fn discard_tile(code: i64, ev: &[PaifuEvent], seat: SeatId) -> Result<(Tile, bool), PaifuError> {
    if code == 60 {
        let t = ev.iter().rev().find_map(|e| match e {
            PaifuEvent::Draw { seat: s, tile } if *s == seat => Some(*tile),
            _ => None,
        });
        return t
            .map(|t| (t, true))
            .ok_or_else(|| PaifuError("ツモ切りの前にツモがありません".into()));
    }
    tile_from_code(code)
        .map(|t| (t, false))
        .ok_or_else(|| PaifuError(format!("打牌 {code} が不正です")))
}

fn parse_result(v: &Value) -> PaifuResult {
    let a = v.as_array().cloned().unwrap_or_default();
    let kind = a.first().and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mut deltas = [0i32; 4];
    let mut wins = Vec::new();
    let mut i = 1;
    while i < a.len() {
        if let Some(d) = a[i]
            .as_array()
            .filter(|d| d.len() == 4 && d.iter().all(|x| x.is_i64()))
        {
            for k in 0..4 {
                deltas[k] += d[k].as_i64().unwrap_or(0) as i32;
            }
            if let Some(info) = a.get(i + 1).and_then(|x| x.as_array()) {
                if info.len() >= 4 && info[0].is_i64() {
                    let w = info[0].as_i64().unwrap_or(0) as u8;
                    let f = info[1].as_i64().unwrap_or(0) as u8;
                    wins.push(PaifuWin {
                        winner: SeatId(w % 4),
                        from: (f != w).then_some(SeatId(f % 4)),
                        points_text: info[3].as_str().unwrap_or("").to_string(),
                        yaku: info[4..]
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect(),
                    });
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    PaifuResult { kind, deltas, wins }
}

/// 河の牌。
#[derive(Clone, Debug, PartialEq)]
pub struct RiverTile {
    pub tile: Tile,
    pub tsumogiri: bool,
    /// 立直宣言牌（横向き）
    pub riichi: bool,
    /// 鳴かれた（暗く表示）
    pub called: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardSeat {
    pub hand: Vec<Tile>,
    /// ツモってまだ捨てていない牌（右に離して表示）
    pub drawn: Option<Tile>,
    pub river: Vec<RiverTile>,
    pub melds: Vec<Meld>,
    pub riichi: bool,
    pub score: i32,
}

/// ある手順の時点の卓。
#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub seats: [BoardSeat; 4],
    pub dora_indicators: Vec<Tile>,
    pub wall_remaining: i32,
    pub turn: Option<SeatId>,
    /// 直前の出来事の見出し（「リーチ」「ポン」「ツモ」など。演出用）
    pub action: Option<(SeatId, String)>,
    pub result: Option<PaifuResult>,
    pub kyotaku: u32,
}

fn remove_tile(hand: &mut Vec<Tile>, t: Tile) {
    if let Some(i) = hand
        .iter()
        .position(|x| *x == t)
        .or_else(|| hand.iter().position(|x| x.kind() == t.kind()))
    {
        hand.remove(i);
    }
}

impl PaifuHand {
    /// `step` 個目の出来事まで進めた卓（0 なら配牌直後）。
    pub fn board_at(&self, step: usize) -> Board {
        let mut b = Board {
            seats: std::array::from_fn(|i| BoardSeat {
                hand: self.haipai[i].clone(),
                score: self.scores_start[i],
                ..Default::default()
            }),
            dora_indicators: self.dora_indicators.clone(),
            wall_remaining: 70,
            turn: Some(self.dealer),
            action: None,
            result: None,
            kyotaku: self.kyotaku,
        };
        for s in &mut b.seats {
            s.hand.sort_by_key(|t| t.sort_key());
        }
        let mut last_discard: Option<SeatId> = None;
        for e in self.events.iter().take(step) {
            b.action = None;
            match e {
                PaifuEvent::Draw { seat, tile } => {
                    let s = &mut b.seats[seat.index()];
                    if let Some(d) = s.drawn.take() {
                        s.hand.push(d);
                    }
                    s.drawn = Some(*tile);
                    b.wall_remaining -= 1;
                    b.turn = Some(*seat);
                }
                PaifuEvent::Discard {
                    seat,
                    tile,
                    tsumogiri,
                    riichi,
                } => {
                    let s = &mut b.seats[seat.index()];
                    if let Some(d) = s.drawn.take() {
                        s.hand.push(d);
                    }
                    remove_tile(&mut s.hand, *tile);
                    s.hand.sort_by_key(|t| t.sort_key());
                    s.river.push(RiverTile {
                        tile: *tile,
                        tsumogiri: *tsumogiri,
                        riichi: *riichi,
                        called: false,
                    });
                    if *riichi {
                        s.riichi = true;
                        s.score -= 1000;
                        b.kyotaku += 1;
                        b.action = Some((*seat, "リーチ".into()));
                    }
                    last_discard = Some(*seat);
                    b.turn = Some(seat.next());
                }
                PaifuEvent::Call { seat, meld } => {
                    if let Some(from) = last_discard {
                        if let Some(rt) = b.seats[from.index()].river.last_mut() {
                            rt.called = true;
                        }
                    }
                    let called =
                        last_discard.and_then(|f| b.seats[f.index()].river.last().map(|r| r.tile));
                    let s = &mut b.seats[seat.index()];
                    let mut used = false;
                    for t in &meld.tiles {
                        if !used && Some(*t) == called {
                            used = true;
                            continue;
                        }
                        remove_tile(&mut s.hand, *t);
                    }
                    s.melds.push(meld.clone());
                    b.turn = Some(*seat);
                    let name = match meld.kind {
                        MeldKind::Chi => "チー",
                        MeldKind::Pon => "ポン",
                        _ => "カン",
                    };
                    b.action = Some((*seat, name.into()));
                }
                PaifuEvent::ConcealedKan { seat, meld } => {
                    let s = &mut b.seats[seat.index()];
                    if let Some(d) = s.drawn.take() {
                        s.hand.push(d);
                    }
                    if meld.kind == MeldKind::Kakan {
                        // ポンを槓に置き換え、足した1枚を手牌から抜く
                        let k = meld.tiles[0].kind();
                        if let Some(m) = s
                            .melds
                            .iter_mut()
                            .find(|m| m.kind == MeldKind::Pon && m.tiles[0].kind() == k)
                        {
                            *m = meld.clone();
                        }
                        remove_tile(&mut s.hand, meld.tiles[0]);
                    } else {
                        for t in &meld.tiles {
                            remove_tile(&mut s.hand, *t);
                        }
                        s.melds.push(meld.clone());
                    }
                    s.hand.sort_by_key(|t| t.sort_key());
                    b.action = Some((*seat, "カン".into()));
                }
                PaifuEvent::NewDora { indicator } => b.dora_indicators.push(*indicator),
                PaifuEvent::End { result } => {
                    for (i, d) in result.deltas.iter().enumerate() {
                        b.seats[i].score += d;
                    }
                    if let Some(w) = result.wins.first() {
                        b.action = Some((
                            w.winner,
                            if w.from.is_some() {
                                "ロン".into()
                            } else {
                                "ツモ".into()
                            },
                        ));
                    }
                    b.result = Some(result.clone());
                    b.turn = None;
                }
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手で作った短い牌譜:
    /// 東1局。親(0)が 1m をツモって 9p を切る → 対面(2)が… ではなく下家(1)がツモ…
    /// 0: ツモ11 打39(9s)  1: ツモ45 打r45(立直・白)  2: 45 をポン(上家=1 から, "p454545")、打11
    /// 3: ツモ12 打12(ツモ切り=60)  0: ツモ 47 でツモアガリ
    fn sample() -> String {
        let hp0 = "[11,12,13,21,22,23,31,32,33,34,35,47,39]";
        let hp1 = "[14,15,16,24,25,26,36,37,38,41,41,42,45]";
        let hp2 = "[17,18,19,27,28,29,43,43,44,44,45,45,46]";
        let hp3 = "[11,12,13,14,15,16,17,18,19,21,21,22,22]";
        format!(
            r#"{{"title":["テスト",""],"name":["東家","南家","西家","北家"],"rule":{{"aka":1}},
            "log":[[[0,0,0],[25000,25000,25000,25000],[41],[42],
              {hp0},[11,47],[39],
              {hp1},[45],["r45"],
              {hp2},["p454545"],[17],
              {hp3},[12],[60],
              ["和了",[12000,-4000,-4000,-4000],[0,0,0,"満貫4000点∀","門前清自摸和(1飜)","役牌 中(1飜)"]]]]}}"#
        )
    }

    #[test]
    fn parse_and_replay() {
        let p = parse_tenhou6(&sample()).unwrap();
        assert_eq!(p.players[0], "東家");
        assert_eq!(p.hands.len(), 1);
        let h = &p.hands[0];
        assert_eq!(h.label(), "東1局 0本場");
        // 0ツモ, 0打, 1ツモ, 1立直, 2ポン, 2打, 3ツモ, 3打, 0ツモ, 終わり
        let kinds: Vec<&str> = h
            .events
            .iter()
            .map(|e| match e {
                PaifuEvent::Draw { .. } => "D",
                PaifuEvent::Discard { .. } => "X",
                PaifuEvent::Call { .. } => "C",
                PaifuEvent::ConcealedKan { .. } => "K",
                PaifuEvent::NewDora { .. } => "N",
                PaifuEvent::End { .. } => "E",
            })
            .collect();
        assert_eq!(kinds.join(""), "DXDXCXDXDE");
        // ポンは上家(1)から
        let PaifuEvent::Call { seat, meld } = &h.events[4] else {
            panic!()
        };
        assert_eq!(*seat, SeatId(2));
        assert_eq!(meld.from, Some(SeatId(1)));
        // 立直後の卓
        let b = h.board_at(4);
        assert!(b.seats[1].riichi);
        assert_eq!(b.seats[1].score, 24_000);
        assert_eq!(b.action.as_ref().map(|a| a.1.as_str()), Some("リーチ"));
        // ポンの後: 鳴かれた牌は暗く、西家の手牌は 13 − 2 = 11 枚＋副露
        let b = h.board_at(5);
        assert!(b.seats[1].river[0].called);
        assert_eq!(b.seats[2].hand.len(), 11);
        assert_eq!(b.seats[2].melds.len(), 1);
        // ツモ切り
        let PaifuEvent::Discard {
            tsumogiri, tile, ..
        } = &h.events[7]
        else {
            panic!()
        };
        assert!(*tsumogiri);
        assert_eq!(tile.to_string(), "2m");
        // 終局
        let b = h.board_at(h.events.len());
        assert_eq!(b.seats[0].score, 25_000 + 12_000);
        assert_eq!(b.result.as_ref().unwrap().wins[0].yaku.len(), 2);
        assert_eq!(b.action.as_ref().map(|a| a.1.as_str()), Some("ツモ"));
        // 手牌の枚数はずっと 13（ツモ牌を除く）か 副露ぶん少ない
        for step in 0..=h.events.len() {
            let b = h.board_at(step);
            for (i, s) in b.seats.iter().enumerate() {
                let n = s.hand.len() + 3 * s.melds.len();
                // 鳴いた直後（打牌前）だけ 14 枚
                let just_called = step > 0
                    && matches!(&h.events[step - 1], PaifuEvent::Call { seat, .. } if seat.index() == i);
                assert_eq!(n, if just_called { 14 } else { 13 }, "手順{step} 席{i}");
            }
        }
    }

    #[test]
    fn ankan_rinshan_and_new_dora() {
        let j = r#"{"name":["a","b","c","d"],"log":[[[0,2,1],[30000,20000,25000,25000],[41,13],[],
          [41,41,41,41,11,12,13,21,22,23,31,32,33],[19,29],["414141a41",29],
          [14,15,16,24,25,26,36,37,38,42,42,43,45],[],[],
          [17,18,19,27,28,29,43,44,44,45,45,46,46],[],[],
          [11,12,13,14,15,16,17,18,19,21,21,22,22],[],[],
          ["流局",[0,0,0,0]]]]}"#;
        let p = parse_tenhou6(j).unwrap();
        let h = &p.hands[0];
        assert_eq!(h.label(), "東1局 2本場");
        let kinds: String = h
            .events
            .iter()
            .map(|e| match e {
                PaifuEvent::Draw { .. } => 'D',
                PaifuEvent::Discard { .. } => 'X',
                PaifuEvent::Call { .. } => 'C',
                PaifuEvent::ConcealedKan { .. } => 'K',
                PaifuEvent::NewDora { .. } => 'N',
                PaifuEvent::End { .. } => 'E',
            })
            .collect();
        // ツモ → 暗槓 → 新ドラ → 嶺上牌 → 打牌 → 終わり
        assert_eq!(kinds, "DKNDXE");
        let b = h.board_at(h.events.len());
        assert_eq!(b.dora_indicators.len(), 2);
        assert_eq!(b.seats[0].melds[0].kind, MeldKind::Ankan);
        assert_eq!(b.seats[0].hand.len() + 3 * b.seats[0].melds.len(), 13);
        assert_eq!(b.result.as_ref().unwrap().kind, "流局");
    }

    #[test]
    fn tile_codes() {
        assert_eq!(tile_from_code(11).unwrap().to_string(), "1m");
        assert_eq!(tile_from_code(29).unwrap().to_string(), "9p");
        assert_eq!(tile_from_code(47).unwrap().to_string(), "7z");
        assert_eq!(tile_from_code(52).unwrap().to_string(), "0p");
        assert!(tile_from_code(60).is_none());
        assert_eq!(
            parse_call("4545p45").unwrap(),
            ('p', 2, vec![tile_from_code(45).unwrap(); 3])
        );
        assert_eq!(from_pos(SeatId(0), 0), SeatId(3));
        assert_eq!(from_pos(SeatId(0), 1), SeatId(2));
        assert_eq!(from_pos(SeatId(0), 2), SeatId(1));
    }

    #[test]
    fn broken_json_is_reported() {
        assert!(parse_tenhou6("{").is_err());
        assert!(parse_tenhou6(r#"{"log":[[1]]}"#).is_err());
    }
}

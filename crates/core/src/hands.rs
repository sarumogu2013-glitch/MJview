//! 牌の手入力（手牌・ツモ・河・鳴き）。engine から呼ぶ。
//!
//! 入力の流れ（全手動）: 親から順に配牌を登録 → 親の打牌 → 鳴き（なければ下家のツモ）→ 打牌か槓 → …
//! 次に入れるものは `TableState::turn` に入る。自動読み取りが入っても、同じ操作で手修正できる。

use crate::engine::EngineError;
use crate::scoring::{Meld, MeldKind, WinInput};
use crate::state::{KanKind, RiverTile, TableState, Turn};
use crate::tile::{SeatId, Tile};

fn invalid<T>(msg: impl Into<String>) -> Result<T, EngineError> {
    Err(EngineError::Invalid(msg.into()))
}

fn name(s: &TableState, seat: SeatId) -> String {
    let n = &s.seats[seat.index()].player.name;
    if n.is_empty() {
        format!("席{}", seat.label())
    } else {
        n.clone()
    }
}

/// 牌1枚を取り除く（同じ牌がなければ false）。
fn remove(v: &mut Vec<Tile>, t: Tile) -> bool {
    match v.iter().position(|x| *x == t) {
        Some(i) => {
            v.remove(i);
            true
        }
        None => false,
    }
}

fn tiles_text(v: &[Tile]) -> String {
    v.iter().map(|t| t.name_ja()).collect::<Vec<_>>().join("・")
}

/// 同じ牌が5枚以上・赤5が2枚以上になっていないか（打ち間違いの検出）。
pub fn check_counts(s: &TableState) -> Result<(), EngineError> {
    let mut n = [0u8; 34];
    let mut red = [0u8; 3];
    let mut add = |t: Tile| {
        if t.is_unknown() {
            return;
        }
        n[t.kind() as usize] += 1;
        if t.is_red() {
            red[(t.kind() / 9) as usize] += 1;
        }
    };
    for st in &s.seats {
        st.hand.iter().for_each(|t| add(*t));
        st.drawn.iter().for_each(|t| add(*t));
        // 鳴かれた牌は副露の側で数える
        st.river
            .iter()
            .filter(|r| !r.called)
            .for_each(|r| add(r.tile));
        st.melds.iter().flat_map(|m| &m.tiles).for_each(|t| add(*t));
    }
    s.dora_indicators.iter().for_each(|t| add(*t));
    if let Some(k) = n.iter().position(|c| *c > 4) {
        return invalid(format!(
            "{}が5枚以上になります（入力を確かめてください）",
            Tile::new(k as u8).name_ja()
        ));
    }
    if red.iter().any(|c| *c > 1) {
        return invalid("赤5が同じ色で2枚以上になります");
    }
    Ok(())
}

/// 配牌の登録が終わったら、親の打牌（14枚目がなければ親のツモ）へ。
fn after_haipai(s: &mut TableState) {
    if s.turn == Turn::None && s.seats.iter().all(|st| st.hand_known) {
        let d = s.dealer();
        s.turn = if s.seats[d.index()].drawn.is_some() {
            Turn::Discard { seat: d }
        } else {
            Turn::Tsumo {
                seat: d,
                rinshan: false,
            }
        };
    }
}

/// 配牌の登録の順番（親から反時計回り）で、まだ登録していない最初の席。
pub fn next_haipai(s: &TableState) -> Option<SeatId> {
    let d = s.dealer();
    (0..4)
        .map(|i| SeatId((d.0 + i) % 4))
        .find(|x| !s.seats[x.index()].hand_known)
}

pub fn set_hand(
    s: &mut TableState,
    seat: SeatId,
    tiles: &[Tile],
    drawn: Option<Tile>,
) -> Result<(), EngineError> {
    let melds = s.seats[seat.index()].melds.len();
    let total = tiles.len() + drawn.is_some() as usize + 3 * melds;
    if total != 13 && total != 14 {
        return invalid(format!(
            "{}の手牌は副露と合わせて13枚か14枚です（今は{}枚）",
            name(s, seat),
            total
        ));
    }
    let first = !s.seats[seat.index()].hand_known;
    let st = &mut s.seats[seat.index()];
    st.hand = tiles.to_vec();
    st.hand.sort_by_key(|t| t.sort_key());
    st.drawn = drawn;
    st.hand_known = true;
    // 配牌の登録中に親の14枚目を入れたら、それは最初のツモ
    if first && s.turn == Turn::None && drawn.is_some() {
        s.wall_remaining = s.wall_remaining.saturating_sub(1);
    }
    check_counts(s)?;
    after_haipai(s);
    Ok(())
}

pub fn tsumo(s: &mut TableState, seat: SeatId, tile: Tile) -> Result<(), EngineError> {
    let who = name(s, seat);
    let st = &s.seats[seat.index()];
    if !st.hand_known {
        return invalid(format!("{who}の手牌がまだ登録されていません"));
    }
    if st.drawn.is_some() || st.tile_count() != 13 {
        return invalid(format!("{who}はツモの前に打牌が必要です"));
    }
    let rinshan = s.turn
        == (Turn::Tsumo {
            seat,
            rinshan: true,
        });
    if !rinshan {
        if s.wall_remaining == 0 {
            return invalid("残り牌数が0なのでツモれません（流局です）");
        }
        s.wall_remaining -= 1;
    }
    s.seats[seat.index()].drawn = Some(tile);
    s.rinshan_drawn = rinshan;
    check_counts(s)?;
    s.turn = Turn::Discard { seat };
    Ok(())
}

pub fn discard(
    s: &mut TableState,
    seat: SeatId,
    tile: Tile,
    riichi: bool,
) -> Result<(), EngineError> {
    let who = name(s, seat);
    let st = &mut s.seats[seat.index()];
    if !st.hand_known {
        return invalid(format!("{who}の手牌がまだ登録されていません"));
    }
    if st.tile_count() != 14 {
        return invalid(format!("{who}は打牌の前にツモか鳴きが必要です"));
    }
    // 一発は、立直のあと自分の次の打牌まで
    st.ippatsu = riichi;
    let tsumogiri = st.drawn == Some(tile);
    if tsumogiri {
        st.drawn = None;
    } else {
        if !remove(&mut st.hand, tile) {
            return invalid(format!("{who}の手牌に{}がありません", tile.name_ja()));
        }
        if let Some(d) = st.drawn.take() {
            st.hand.push(d);
        }
        st.hand.sort_by_key(|t| t.sort_key());
    }
    st.river.push(RiverTile {
        tile,
        riichi,
        called: false,
        tsumogiri,
    });
    s.turn = Turn::AfterDiscard { seat };
    s.rinshan_drawn = false;
    Ok(())
}

/// 鳴き・槓のあと（一発・1巡目の役が消える）
fn broke_turn(s: &mut TableState) {
    s.calls_this_hand = true;
    for st in &mut s.seats {
        st.ippatsu = false;
    }
}

/// 直前の打牌（鳴ける牌）。
pub fn last_discard(s: &TableState) -> Option<(SeatId, Tile)> {
    match s.turn {
        Turn::AfterDiscard { seat } => {
            let r = s.seats[seat.index()].river.last()?;
            (!r.called).then_some((seat, r.tile))
        }
        _ => None,
    }
}

fn take_called(s: &mut TableState) -> Result<(SeatId, Tile), EngineError> {
    let Some((from, t)) = last_discard(s) else {
        return invalid("鳴ける打牌がありません（打牌の直後だけ鳴けます）");
    };
    if let Some(r) = s.seats[from.index()].river.last_mut() {
        r.called = true;
    }
    Ok((from, t))
}

pub fn call(
    s: &mut TableState,
    seat: SeatId,
    kind: MeldKind,
    tiles: &[Tile],
) -> Result<(), EngineError> {
    if !matches!(kind, MeldKind::Chi | MeldKind::Pon) {
        return invalid("鳴きはチー・ポンです（大明槓は槓で入力します）");
    }
    let who = name(s, seat);
    let Some((from, called)) = last_discard(s) else {
        return invalid("鳴ける打牌がありません（打牌の直後だけ鳴けます）");
    };
    if from == seat {
        return invalid("自分の打牌は鳴けません");
    }
    if tiles.len() != 2 {
        return invalid("鳴きでは手牌から2枚出します");
    }
    let mut all = [called, tiles[0], tiles[1]];
    all.sort_by_key(|t| t.kind());
    let ok = match kind {
        MeldKind::Chi => {
            if from.next() != seat {
                return invalid("チーは上家の打牌だけです");
            }
            !all[0].is_honor()
                && all[0].suit() == all[2].suit()
                && all[1].kind() == all[0].kind() + 1
                && all[2].kind() == all[0].kind() + 2
        }
        _ => all.iter().all(|t| t.kind() == called.kind()),
    };
    if !ok {
        return invalid(format!(
            "{}と{}では{}できません",
            called.name_ja(),
            tiles_text(tiles),
            if kind == MeldKind::Chi {
                "チー"
            } else {
                "ポン"
            }
        ));
    }
    let st = &mut s.seats[seat.index()];
    if st.drawn.is_some() || st.tile_count() != 13 {
        return invalid(format!("{who}は今は鳴けません（打牌の前です）"));
    }
    let mut hand = st.hand.clone();
    for t in tiles {
        if !remove(&mut hand, *t) {
            return invalid(format!("{who}の手牌に{}がありません", t.name_ja()));
        }
    }
    st.hand = hand;
    let mut mt = vec![called];
    mt.extend_from_slice(tiles);
    st.melds.push(Meld {
        kind,
        tiles: mt,
        from: Some(from),
    });
    take_called(s)?;
    broke_turn(s);
    s.turn = Turn::Discard { seat };
    Ok(())
}

/// 槓の手牌の動き（残り牌数・ドラは engine 側）。終わったら嶺上牌のツモへ。
pub fn kan(
    s: &mut TableState,
    seat: SeatId,
    kind: KanKind,
    tiles: &[Tile],
) -> Result<(), EngineError> {
    let who = name(s, seat);
    match kind {
        KanKind::Minkan => {
            let Some((from, called)) = last_discard(s) else {
                return invalid("大明槓できる打牌がありません（打牌の直後だけ）");
            };
            if from == seat {
                return invalid("自分の打牌は鳴けません");
            }
            if tiles.len() != 3 || tiles.iter().any(|t| t.kind() != called.kind()) {
                return invalid(format!("大明槓は手牌の{}3枚で行います", called.name_ja()));
            }
            let st = &mut s.seats[seat.index()];
            if st.drawn.is_some() || st.tile_count() != 13 {
                return invalid(format!("{who}は今は鳴けません"));
            }
            let mut hand = st.hand.clone();
            for t in tiles {
                if !remove(&mut hand, *t) {
                    return invalid(format!("{who}の手牌に{}がありません", t.name_ja()));
                }
            }
            st.hand = hand;
            let mut mt = vec![called];
            mt.extend_from_slice(tiles);
            st.melds.push(Meld {
                kind: MeldKind::Minkan,
                tiles: mt,
                from: Some(from),
            });
            take_called(s)?;
        }
        KanKind::Ankan | KanKind::Kakan => {
            let st = &mut s.seats[seat.index()];
            if st.tile_count() != 14 {
                return invalid(format!("{who}は今は槓できません（ツモのあとだけ）"));
            }
            let mut pool = st.hand.clone();
            if let Some(d) = st.drawn {
                pool.push(d);
            }
            if kind == KanKind::Ankan {
                if tiles.len() != 4 || tiles.iter().any(|t| t.kind() != tiles[0].kind()) {
                    return invalid("暗槓は同じ牌4枚です");
                }
                for t in tiles {
                    if !remove(&mut pool, *t) {
                        return invalid(format!("{who}の手牌に{}が4枚ありません", t.name_ja()));
                    }
                }
                st.melds.push(Meld {
                    kind: MeldKind::Ankan,
                    tiles: tiles.to_vec(),
                    from: None,
                });
            } else {
                let [t] = tiles else {
                    return invalid("加槓は1枚です");
                };
                let Some(m) = st
                    .melds
                    .iter_mut()
                    .find(|m| m.kind == MeldKind::Pon && m.tiles[0].kind() == t.kind())
                else {
                    return invalid(format!("{}のポンがないので加槓できません", t.name_ja()));
                };
                if !remove(&mut pool, *t) {
                    return invalid(format!("{who}の手牌に{}がありません", t.name_ja()));
                }
                m.kind = MeldKind::Kakan;
                m.tiles.push(*t);
            }
            pool.sort_by_key(|t| t.sort_key());
            st.hand = pool;
            st.drawn = None;
        }
    }
    check_counts(s)?;
    broke_turn(s);
    s.turn = Turn::Tsumo {
        seat,
        rinshan: true,
    };
    Ok(())
}

/// 手牌から、`want` と同じ種類の牌を n 枚選ぶ組み合わせ（赤の有無で分かれる）。
fn same_kind_options(hand: &[Tile], want: Tile, n: usize) -> Vec<Vec<Tile>> {
    let same: Vec<Tile> = hand
        .iter()
        .copied()
        .filter(|t| t.kind() == want.kind())
        .collect();
    if same.len() < n {
        return vec![];
    }
    let reds = same.iter().filter(|t| t.is_red()).count();
    let plain = same.len() - reds;
    let mut out = vec![];
    // 赤なし、赤あり（それぞれ取れるなら）
    if plain >= n {
        out.push(vec![want.plain(); n]);
    }
    if reds >= 1 && n >= 1 && plain >= n - 1 {
        let mut v = vec![same.iter().copied().find(|t| t.is_red()).unwrap()];
        v.extend(vec![want.plain(); n - 1]);
        out.push(v);
    }
    out
}

/// チーの候補（手牌から出す2枚）。
pub fn chi_options(hand: &[Tile], called: Tile) -> Vec<Vec<Tile>> {
    if called.is_honor() {
        return vec![];
    }
    let base = called.kind() - called.kind() % 9;
    let k = called.kind();
    let mut out: Vec<Vec<Tile>> = vec![];
    for (a, b) in [(-2i8, -1i8), (-1, 1), (1, 2)] {
        let ka = k as i8 + a;
        let kb = k as i8 + b;
        if ka < base as i8 || kb > base as i8 + 8 {
            continue;
        }
        let oa = same_kind_options(hand, Tile::new(ka as u8), 1);
        let ob = same_kind_options(hand, Tile::new(kb as u8), 1);
        for x in &oa {
            for y in &ob {
                out.push(vec![x[0], y[0]]);
            }
        }
    }
    out
}

/// ポンの候補（手牌から出す2枚）。
pub fn pon_options(hand: &[Tile], called: Tile) -> Vec<Vec<Tile>> {
    same_kind_options(hand, called, 2)
}

/// 大明槓の手牌3枚（なければ None）。
pub fn minkan_tiles(hand: &[Tile], called: Tile) -> Option<Vec<Tile>> {
    let v: Vec<Tile> = hand
        .iter()
        .copied()
        .filter(|t| t.kind() == called.kind())
        .collect();
    (v.len() >= 3).then(|| v[..3].to_vec())
}

/// 暗槓・加槓の候補（`t` の種類で）。暗槓なら4枚、加槓なら1枚。
pub fn self_kan(s: &TableState, seat: SeatId, t: Tile) -> Option<(KanKind, Vec<Tile>)> {
    let st = &s.seats[seat.index()];
    let mut pool = st.hand.clone();
    pool.extend(st.drawn);
    let same: Vec<Tile> = pool
        .iter()
        .copied()
        .filter(|x| x.kind() == t.kind())
        .collect();
    if same.len() == 4 {
        return Some((KanKind::Ankan, same));
    }
    let pon = st
        .melds
        .iter()
        .any(|m| m.kind == MeldKind::Pon && m.tiles[0].kind() == t.kind());
    match same.first() {
        Some(x) if pon => Some((KanKind::Kakan, vec![*x])),
        _ => None,
    }
}

/// 手牌からアガリの入力を作る（ロンなら直前の打牌、ツモならツモ牌をアガリ牌に）。
pub fn win_input(s: &TableState, winner: SeatId, loser: Option<SeatId>) -> Option<WinInput> {
    let st = &s.seats[winner.index()];
    if !st.hand_known {
        return None;
    }
    let mut closed = st.hand.clone();
    let win_tile = match loser {
        Some(l) => {
            let r = s.seats[l.index()].river.last()?;
            r.tile
        }
        None => st.drawn?,
    };
    closed.push(win_tile);
    Some(WinInput {
        winner,
        loser,
        closed,
        melds: st.melds.clone(),
        win_tile,
        riichi: st.riichi && !st.double_riichi,
        double_riichi: st.double_riichi,
        ippatsu: st.ippatsu,
        haitei: false,
        rinshan: loser.is_none() && s.rinshan_drawn,
        chankan: false,
        // 天和・地和: 鳴きのない1巡目で、まだ打牌していないツモアガリ
        tenhou: loser.is_none() && st.river.is_empty() && !s.calls_this_hand,
        ura_indicators: vec![],
        pao: None,
    })
}

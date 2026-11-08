//! 機能の確認（レビュー）用のダミー対局データ。サーバーの `--dummy` と、クライアントの `--dummy-server` が使う。
//!
//! 普通の操作（[`Action`]）を並べたものなので、取り消し・やり直しや、ここからの続きの操作もそのまま試せる。
//! 中身: 東1局で南家がロン → 東2局でツモ → 東3局の途中（ドラ表示・山の残り・北家のリーチ）

use serde::{Deserialize, Serialize};

use crate::rules::GameConfig;
use crate::scoring::WinInput;
use crate::state::{Action, Player};
use crate::tile::{SeatId, Tile, parse_tiles};

fn player(name: &str, aff: Option<&str>) -> Player {
    Player {
        name: name.into(),
        affiliation: aff.map(Into::into),
    }
}

fn win(winner: u8, loser: Option<u8>, closed: &str, wt: &str, riichi: bool) -> WinInput {
    WinInput {
        winner: SeatId(winner),
        loser: loser.map(SeatId),
        closed: parse_tiles(closed).expect("dummy tiles"),
        melds: vec![],
        win_tile: wt.parse().expect("dummy tile"),
        riichi,
        double_riichi: false,
        ippatsu: false,
        haitei: false,
        rinshan: false,
        chankan: false,
        tenhou: false,
        ura_indicators: vec![],
        pao: None,
    }
}

/// 取り込んだ対局（雀魂の牌譜などから作ったダミーデータ）のファイル
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyFile {
    pub format: String,
    pub source: String,
    pub uuid: String,
    pub title: String,
    pub actions: Vec<Action>,
    /// 変換で合わせたところ（点数の手動調整など）
    #[serde(default)]
    pub notes: Vec<String>,
}

pub const FORMAT: &str = "janscope-dummy-v1";

/// ダミーデータのファイルを読む
pub fn load(text: &str) -> Result<DummyFile, String> {
    let d: DummyFile = serde_json::from_str(text)
        .map_err(|e| format!("ダミーデータのファイルを読めません: {e}"))?;
    if d.format != FORMAT {
        return Err(format!("ダミーデータの形式が違います（{}）", d.format));
    }
    if d.actions.is_empty() {
        return Err("ダミーデータに操作がありません".into());
    }
    Ok(d)
}

/// ダミー対局の操作の列（最初から順に当てる）
pub fn actions() -> Vec<Action> {
    let mut v = vec![
        Action::StartGame {
            config: GameConfig::preset_standard(),
            players: [
                player("山田 太郎", Some("東京支部")),
                player("佐藤 花子", Some("大阪支部")),
                player("鈴木 一郎", None),
                player("高橋 美咲", Some("名古屋支部")),
            ],
            chiicha: SeatId(0),
        },
        // 東1局: 南家（席1）が東家（席0）からロン
        Action::StartHand { dice: vec![(3, 5)] },
        Action::SetDora {
            tiles: parse_tiles("1s").unwrap(),
        },
        Action::Win {
            wins: vec![win(1, Some(0), "234567m345p678s99p", "2m", false)],
        },
        // 東2局: 西家（席2）のリーチ・ツモ
        Action::StartHand { dice: vec![(4, 4)] },
        Action::SetDora {
            tiles: parse_tiles("6m").unwrap(),
        },
        Action::Riichi { seat: SeatId(2) },
        Action::Win {
            wins: vec![win(2, None, "123m456p789s11555z", "5z", true)],
        },
        // 東3局の途中
        Action::StartHand { dice: vec![(2, 6)] },
        Action::SetDora {
            tiles: parse_tiles("3p").unwrap(),
        },
    ];
    // 配牌・ツモ・打牌（牌の手入力の流れ）。最後は北家（席3）のリーチ宣言牌
    v.extend(east3_tiles(parse_tiles("3p").unwrap()[0]));
    v
}

/// 東3局（親は席2）の配牌から、決まった順の山で29巡ぶんツモ・打牌する。
fn east3_tiles(dora_indicator: Tile) -> Vec<Action> {
    // 136枚（赤5を1枚ずつ）を決まった並びに混ぜる
    let mut deck: Vec<Tile> = vec![];
    for k in 0..34u8 {
        for i in 0..4 {
            let t = Tile::new(k);
            let red = i == 0 && !t.is_honor() && t.number() == 5;
            deck.push(if red { Tile::red_five(t.suit()) } else { t });
        }
    }
    if let Some(i) = deck.iter().position(|t| *t == dora_indicator) {
        deck.remove(i);
    }
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    for i in (1..deck.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        deck.swap(i, (x % (i as u64 + 1)) as usize);
    }
    let mut wall = deck.into_iter();
    let order = [SeatId(2), SeatId(3), SeatId(0), SeatId(1)];
    let mut hands: [Vec<Tile>; 4] = Default::default();
    let mut out = vec![];
    for (n, seat) in order.iter().enumerate() {
        let mut h: Vec<Tile> = wall.by_ref().take(13).collect();
        h.sort_by_key(|t| t.sort_key());
        let drawn = (n == 0).then(|| wall.next().unwrap());
        out.push(Action::SetHand {
            seat: *seat,
            tiles: h.clone(),
            drawn,
        });
        h.extend(drawn);
        hands[seat.index()] = h;
    }
    // いらない牌（孤立した字牌 → 么九牌 → ツモ切り）
    let pick = |h: &[Tile], drawn: Tile| -> Tile {
        let count = |t: Tile| h.iter().filter(|x| x.kind() == t.kind()).count();
        h.iter()
            .copied()
            .filter(|t| t.is_honor() && count(*t) == 1)
            .chain(
                h.iter()
                    .copied()
                    .filter(|t| t.is_terminal_or_honor() && count(*t) == 1),
            )
            .next()
            .unwrap_or(drawn)
    };
    let discard =
        |out: &mut Vec<Action>, h: &mut Vec<Tile>, seat: SeatId, drawn: Tile, riichi: bool| {
            let t = pick(h, drawn);
            let i = h.iter().position(|x| *x == t).unwrap();
            h.remove(i);
            out.push(Action::Discard {
                seat,
                tile: t,
                riichi,
            });
        };
    let first = *hands[2].last().unwrap();
    discard(&mut out, &mut hands[2], SeatId(2), first, false);
    for turn in 0..29 {
        let seat = order[(turn + 1) % 4];
        let t = wall.next().unwrap();
        out.push(Action::Tsumo { seat, tile: t });
        hands[seat.index()].push(t);
        discard(&mut out, &mut hands[seat.index()], seat, t, turn == 28);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TableState;

    #[test]
    fn dummy_game_applies_cleanly() {
        let mut s = TableState::default();
        for a in actions() {
            crate::engine::apply(&mut s, &a).unwrap_or_else(|e| panic!("{a:?}: {e}"));
        }
        assert_eq!(s.seats[3].player.name, "高橋 美咲");
        assert!(s.seats[3].riichi);
        assert!(s.seats.iter().all(|x| x.hand_known && x.tile_count() == 13));
        assert_eq!(s.wall_remaining, 70 - 1 - 29);
    }
}

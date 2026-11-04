//! Python の `mahjong` ライブラリとの点数計算の突き合わせ（設計書 8章・段階B4）。
//! ケースは tools/gen_scoring_cases.py で作る。

use janscope_core::rules::GameConfig;
use janscope_core::scoring::{
    Limit, Meld, MeldKind, ScoreContext, ScoreError, WinInput, score_win,
};
use janscope_core::tile::{SeatId, Tile, Wind};
use serde::Deserialize;

#[derive(Deserialize)]
struct CaseMeld {
    kind: String,
    tiles: Vec<Tile>,
}

#[derive(Deserialize)]
struct Expected {
    error: Option<String>,
    han: Option<u32>,
    fu: Option<u32>,
    yakuman: Option<bool>,
    main: Option<i32>,
    additional: Option<i32>,
    yaku: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Case {
    closed: Vec<Tile>,
    melds: Vec<CaseMeld>,
    win: Tile,
    tsumo: bool,
    riichi: bool,
    seat_wind: u8,
    round_wind: u8,
    dora_indicators: Vec<Tile>,
    expected: Expected,
}

#[test]
fn matches_python_mahjong() {
    let data = include_str!("data/mahjong_cases.json");
    let cases: Vec<Case> = serde_json::from_str(data).unwrap();
    assert!(cases.len() >= 300, "ケースが少なすぎます");
    let cfg = GameConfig::preset_standard();
    let mut fails = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let input = WinInput {
            winner: SeatId(0),
            loser: if c.tsumo { None } else { Some(SeatId(1)) },
            closed: c.closed.clone(),
            melds: c
                .melds
                .iter()
                .map(|m| Meld {
                    kind: if m.kind == "Chi" {
                        MeldKind::Chi
                    } else {
                        MeldKind::Pon
                    },
                    tiles: m.tiles.clone(),
                    from: Some(SeatId(1)),
                })
                .collect(),
            win_tile: c.win,
            riichi: c.riichi,
            double_riichi: false,
            ippatsu: false,
            haitei: false,
            rinshan: false,
            chankan: false,
            tenhou: false,
            ura_indicators: vec![],
            pao: None,
        };
        let ctx = ScoreContext {
            config: &cfg,
            round_wind: Wind::from_index(c.round_wind),
            seat_wind: Wind::from_index(c.seat_wind),
            dora_indicators: &c.dora_indicators,
        };
        let got = score_win(&input, &ctx);
        let e = &c.expected;
        let desc = || {
            format!(
                "#{i} 門前 {} 副露 {:?} アガリ {} {} 自風{} 場風{} ドラ表示 {:?}",
                c.closed
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(""),
                c.melds
                    .iter()
                    .map(|m| format!("{}:{:?}", m.kind, m.tiles))
                    .collect::<Vec<_>>(),
                c.win,
                if c.tsumo { "ツモ" } else { "ロン" },
                c.seat_wind,
                c.round_wind,
                c.dora_indicators
            )
        };
        match (&e.error, got) {
            (Some(_), Err(ScoreError::NoYaku)) => {}
            (Some(_), Ok(s)) => fails.push(format!(
                "{}: 期待=役なし 結果={:?} {}",
                desc(),
                s.yaku,
                s.han_fu_text()
            )),
            (None, Err(err)) => {
                fails.push(format!("{}: 期待={:?} 結果=エラー {err}", desc(), e.yaku))
            }
            (Some(_), Err(err)) => fails.push(format!("{}: 期待=役なし 結果=エラー {err}", desc())),
            (None, Ok(s)) => {
                let yakuman = e.yakuman.unwrap();
                let (main, add) = if s.tsumo {
                    if s.is_dealer {
                        (s.tsumo_dealer_pays, s.tsumo_dealer_pays)
                    } else {
                        (s.tsumo_dealer_pays, s.tsumo_child_pays)
                    }
                } else {
                    (s.ron_points, 0)
                };
                let han_ok = yakuman || s.han == e.han.unwrap();
                let fu_ok = yakuman || s.fu == e.fu.unwrap();
                let lim_ok =
                    yakuman == matches!(s.limit, Some(Limit::Yakuman(_))) || e.han.unwrap() >= 13;
                if !(han_ok
                    && fu_ok
                    && lim_ok
                    && main == e.main.unwrap()
                    && add == e.additional.unwrap())
                {
                    fails.push(format!(
                        "{}: 期待 {}翻{}符 {}/{} {:?} ／ 結果 {} {}/{} {:?} 符内訳 {:?}",
                        desc(),
                        e.han.unwrap(),
                        e.fu.unwrap(),
                        e.main.unwrap(),
                        e.additional.unwrap(),
                        e.yaku.as_ref().unwrap(),
                        s.han_fu_text(),
                        main,
                        add,
                        s.yaku.iter().map(|y| &y.name).collect::<Vec<_>>(),
                        s.fu_items
                            .iter()
                            .map(|f| (&f.name, f.fu))
                            .collect::<Vec<_>>()
                    ));
                }
            }
        }
    }
    if !fails.is_empty() {
        let n = fails.len();
        panic!(
            "{n}/{} 件が不一致:\n{}",
            cases.len(),
            fails.into_iter().take(15).collect::<Vec<_>>().join("\n")
        );
    }
}

use super::*;
use crate::tile::parse_tiles;

fn t(s: &str) -> Tile {
    s.parse().unwrap()
}

struct Case {
    closed: &'static str,
    melds: Vec<(MeldKind, &'static str)>,
    win: &'static str,
    tsumo: bool,
    riichi: bool,
    seat: Wind,
    round: Wind,
    dora: &'static str,
}

impl Case {
    fn new(closed: &'static str, win: &'static str) -> Self {
        Case {
            closed,
            melds: vec![],
            win,
            tsumo: false,
            riichi: false,
            seat: Wind::South,
            round: Wind::East,
            dora: "",
        }
    }
    fn input(&self) -> WinInput {
        WinInput {
            winner: SeatId(1),
            loser: if self.tsumo { None } else { Some(SeatId(0)) },
            closed: parse_tiles(self.closed).unwrap(),
            melds: self
                .melds
                .iter()
                .map(|(k, s)| Meld {
                    kind: *k,
                    tiles: parse_tiles(s).unwrap(),
                    from: Some(SeatId(0)),
                })
                .collect(),
            win_tile: t(self.win),
            riichi: self.riichi,
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
    fn score_with(&self, cfg: &GameConfig) -> Result<WinScore, ScoreError> {
        let dora = parse_tiles(self.dora).unwrap();
        let ctx = ScoreContext {
            config: cfg,
            round_wind: self.round,
            seat_wind: self.seat,
            dora_indicators: &dora,
        };
        score_win(&self.input(), &ctx)
    }
    fn score(&self) -> WinScore {
        self.score_with(&GameConfig::preset_standard()).unwrap()
    }
}

fn names(s: &WinScore) -> Vec<&str> {
    s.yaku.iter().map(|y| y.name.as_str()).collect()
}

#[test]
fn pinfu_ron_30fu() {
    // 234m 567m 345p 678s 99p, 待ち 2m-5m のリャンメン → 平和のみ 1翻30符 子ロン 1000
    let s = Case {
        riichi: false,
        ..Case::new("234567m345p678s99p", "2m")
    }
    .score();
    assert_eq!(names(&s), vec!["平和"]);
    assert_eq!((s.han, s.fu), (1, 30));
    assert_eq!(s.ron_points, 1_000);
}

#[test]
fn pinfu_tsumo_20fu_riichi() {
    let s = Case {
        tsumo: true,
        riichi: true,
        ..Case::new("234567m345p678s99p", "2m")
    }
    .score();
    assert!(
        names(&s).contains(&"立直")
            && names(&s).contains(&"平和")
            && names(&s).contains(&"門前清自摸和")
    );
    assert_eq!((s.han, s.fu), (3, 20));
    // 子 3翻20符ツモ: 700-1300
    assert_eq!((s.tsumo_child_pays, s.tsumo_dealer_pays), (700, 1_300));
    assert_eq!(s.points_text(), "700-1,300");
}

#[test]
fn chiitoitsu_25fu() {
    let s = Case {
        riichi: true,
        ..Case::new("1133m5577p22s3344z", "4z")
    }
    .score();
    assert!(names(&s).contains(&"七対子"));
    assert_eq!((s.han, s.fu), (3, 25));
    assert_eq!(s.ron_points, 3_200);
}

#[test]
fn dealer_mangan_tsumo() {
    // 親 リーチツモ 清一色ではなく 混一色＋一通… 簡単に: 123456789m 東東東 11m? → 1m は5枚になるので 22m
    let s = Case {
        tsumo: true,
        seat: Wind::East,
        round: Wind::East,
        ..Case::new("123456789m11122z", "2z")
    }
    .score();
    // 一気通貫2 + 混一色3 + 場風東1 + 自風東1 + 門前ツモ1 = 8翻 倍満
    assert_eq!(s.limit, Some(Limit::Baiman));
    assert_eq!(s.tsumo_dealer_pays, 8_000);
    assert_eq!(s.points_text(), "8,000オール");
    assert!(names(&s).contains(&"自風 東") && names(&s).contains(&"場風 東"));
}

#[test]
fn open_pinfu_shape_is_30fu() {
    let s = Case {
        melds: vec![(MeldKind::Chi, "234m")],
        ..Case::new("567m345p678s22p", "5m")
    }
    .score();
    assert_eq!(names(&s), vec!["断幺九"]);
    assert_eq!((s.han, s.fu), (1, 30));
}

#[test]
fn kuitan_off() {
    let c = Case {
        melds: vec![(MeldKind::Chi, "234m")],
        ..Case::new("567m345p678s22p", "5m")
    };
    let cfg = GameConfig {
        kuitan: false,
        ..GameConfig::preset_standard()
    };
    assert_eq!(c.score_with(&cfg), Err(ScoreError::NoYaku));
}

#[test]
fn high_point_rule_picks_best_wait() {
    // 111222333m 456p 77z, 3m ツモ: 三暗刻(1m2m3m暗刻) か 一盃口系の解釈 → 高い方
    let s = Case {
        tsumo: true,
        ..Case::new("111222333m456p77z", "3m")
    }
    .score();
    assert!(names(&s).contains(&"三暗刻"), "{:?}", names(&s));
}

#[test]
fn dora_aka_ura() {
    let mut c = Case {
        riichi: true,
        dora: "1m",
        ..Case::new("234567m340p678s99p", "2m")
    };
    c.closed = "234567m340p678s99p";
    let mut inp = c.input();
    inp.ura_indicators = parse_tiles("8p").unwrap();
    let cfg = GameConfig::preset_standard();
    let dora = parse_tiles(c.dora).unwrap();
    let ctx = ScoreContext {
        config: &cfg,
        round_wind: Wind::East,
        seat_wind: Wind::South,
        dora_indicators: &dora,
    };
    let s = score_win(&inp, &ctx).unwrap();
    assert_eq!((s.dora, s.aka, s.ura), (1, 1, 2));
    // 立直1 平和1 + ドラ1 赤1 裏2 = 6翻 跳満
    assert_eq!(s.han, 6);
    assert_eq!(s.limit, Some(Limit::Haneman));
    assert_eq!(s.ron_points, 12_000);
    assert!(s.groups.iter().flatten().any(|x| x.is_red()));
}

#[test]
fn kiriage() {
    // 子 4翻30符: 7700 / 切り上げで 8000
    let c = Case {
        riichi: true,
        dora: "4m",
        ..Case::new("234567m345p678s99p", "2m")
    };
    // 立直 平和 ドラ1 = 3翻… 4翻にするため 一発を付ける
    let mut inp = c.input();
    inp.ippatsu = true;
    let dora = parse_tiles("4m").unwrap();
    let cfg = GameConfig::preset_standard();
    let ctx = ScoreContext {
        config: &cfg,
        round_wind: Wind::East,
        seat_wind: Wind::South,
        dora_indicators: &dora,
    };
    let s = score_win(&inp, &ctx).unwrap();
    assert_eq!((s.han, s.fu, s.ron_points), (4, 30, 7_700));
    let cfg2 = GameConfig {
        kiriage_mangan: true,
        ..GameConfig::preset_standard()
    };
    let ctx2 = ScoreContext {
        config: &cfg2,
        ..ctx
    };
    assert_eq!(score_win(&inp, &ctx2).unwrap().ron_points, 8_000);
}

#[test]
fn yakuman_double() {
    let c = Case {
        tsumo: true,
        ..Case::new("19m19p19s12345677z", "7z")
    };
    let s = c.score();
    assert_eq!(s.limit, Some(Limit::Yakuman(2)));
    assert_eq!(names(&s), vec!["国士無双十三面待ち"]);
    let single = c
        .score_with(&GameConfig {
            double_yakuman: false,
            ..GameConfig::preset_standard()
        })
        .unwrap();
    assert_eq!(single.limit, Some(Limit::Yakuman(1)));
    // 子の役満ツモ: 8000-16000
    assert_eq!(
        (single.tsumo_child_pays, single.tsumo_dealer_pays),
        (8_000, 16_000)
    );
}

#[test]
fn double_wind_pair() {
    // 東場の東家、東の雀頭: 4符（ルール既定） → 40符 / 2符ルールなら 30符… 役は立直のみ
    let c = Case {
        riichi: true,
        seat: Wind::East,
        round: Wind::East,
        ..Case::new("234567m345p678s11z", "2m")
    };
    let four = c.score();
    let two = c
        .score_with(&GameConfig {
            double_wind_pair_fu: DoubleWindPairFu::Two,
            ..GameConfig::preset_standard()
        })
        .unwrap();
    // 20 + 門前ロン10 + 雀頭4 = 34 → 40 / 20+10+2 = 32 → 40（どちらも40符）
    assert_eq!(four.fu, 40);
    assert_eq!(two.fu, 40);
    assert!(four.fu_items.iter().map(|f| f.fu).sum::<u32>() == 34);
}

#[test]
fn errors() {
    assert!(matches!(
        Case::new("234567m345p678s9p", "2m").score_with(&GameConfig::default()),
        Err(ScoreError::TileCount { .. })
    ));
    assert!(matches!(
        Case::new("234567m345p678s19p", "2m").score_with(&GameConfig::default()),
        Err(ScoreError::NotWinning)
    ));
    assert!(matches!(
        Case::new("234567m345p678s99p", "1m").score_with(&GameConfig::default()),
        Err(ScoreError::WinTileMissing(_))
    ));
    assert!(matches!(
        Case::new("11111m2345p678s99p", "1m").score_with(&GameConfig::default()),
        Err(ScoreError::TooMany(_))
    ));
}

#[test]
fn houtei_name() {
    let mut inp = Case::new("234567m345p678s99p", "2m").input();
    inp.haitei = true;
    let cfg = GameConfig::default();
    let ctx = ScoreContext {
        config: &cfg,
        round_wind: Wind::East,
        seat_wind: Wind::South,
        dora_indicators: &[],
    };
    let s = score_win(&inp, &ctx).unwrap();
    assert!(names(&s).contains(&"河底撈魚"));
}

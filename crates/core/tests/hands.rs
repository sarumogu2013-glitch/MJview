//! 牌の手入力（配牌 → 打牌 → 鳴き → ツモ → 槓）の流れ。

use janscope_core::engine::apply;
use janscope_core::hands;
use janscope_core::rules::GameConfig;
use janscope_core::scoring::MeldKind;
use janscope_core::state::*;
use janscope_core::tile::{SeatId, Tile, parse_tiles};

const A: SeatId = SeatId(0);
const B: SeatId = SeatId(1);
const C: SeatId = SeatId(2);
const D: SeatId = SeatId(3);

fn t(s: &str) -> Tile {
    s.parse().unwrap()
}

fn ts(s: &str) -> Vec<Tile> {
    parse_tiles(s).unwrap()
}

fn act(s: &mut TableState, a: Action) {
    if let Err(e) = apply(s, &a) {
        panic!("{a:?}: {e}");
    }
}

fn started() -> TableState {
    let mut s = TableState::default();
    act(
        &mut s,
        Action::StartGame {
            config: GameConfig::default(),
            players: ["A", "B", "C", "D"].map(Player::new),
            chiicha: A,
        },
    );
    act(&mut s, Action::StartHand { dice: vec![(3, 4)] });
    // 親 A は14枚（最後の1枚がツモ）、子は13枚
    act(
        &mut s,
        Action::SetHand {
            seat: A,
            tiles: ts("123m456p789s1122z"),
            drawn: Some(t("5z")),
        },
    );
    assert_eq!(s.turn, Turn::None);
    assert_eq!(hands::next_haipai(&s), Some(B));
    act(
        &mut s,
        Action::SetHand {
            seat: B,
            tiles: ts("11m234p567s33z555z"),
            drawn: None,
        },
    );
    act(
        &mut s,
        Action::SetHand {
            seat: C,
            tiles: ts("456m11p23s666z777z"),
            drawn: None,
        },
    );
    act(
        &mut s,
        Action::SetHand {
            seat: D,
            tiles: ts("789m999p111s4444z"),
            drawn: None,
        },
    );
    s
}

#[test]
fn haipai_then_discard_call_tsumo() {
    let mut s = started();
    assert_eq!(s.turn, Turn::Discard { seat: A });
    // 親の14枚目は最初のツモなので残りは 69
    assert_eq!(s.wall_remaining, 69);
    // 親が 2z を打つ（手出し）→ ツモ牌 5z は手牌に入る
    act(
        &mut s,
        Action::Discard {
            seat: A,
            tile: t("2z"),
            riichi: false,
        },
    );
    assert_eq!(s.turn, Turn::AfterDiscard { seat: A });
    assert!(s.seats[0].hand.contains(&t("5z")));
    assert!(!s.seats[0].river[0].tsumogiri);
    // B のツモ → ツモ切り
    act(
        &mut s,
        Action::Tsumo {
            seat: B,
            tile: t("9m"),
        },
    );
    assert_eq!(s.wall_remaining, 68);
    act(
        &mut s,
        Action::Discard {
            seat: B,
            tile: t("9m"),
            riichi: true,
        },
    );
    assert!(s.seats[1].riichi && s.seats[1].river[0].riichi && s.seats[1].river[0].tsumogiri);
    assert_eq!(s.kyotaku, 1);
    // D がポン（9m は D の手牌に1枚しかないので不可）、C もない → 次のツモ
    assert!(hands::pon_options(&s.seats[3].hand, t("9m")).is_empty());
    act(
        &mut s,
        Action::Tsumo {
            seat: C,
            tile: t("3m"),
        },
    );
    act(
        &mut s,
        Action::Discard {
            seat: C,
            tile: t("3m"),
            riichi: false,
        },
    );
    // D がチー（C の下家）: 3m は D の手牌では作れない → エラー
    assert!(
        apply(
            &mut s,
            &Action::Call {
                seat: D,
                kind: MeldKind::Chi,
                tiles: ts("12m"),
            }
        )
        .is_err()
    );
    act(
        &mut s,
        Action::Tsumo {
            seat: D,
            tile: t("1z"),
        },
    );
    act(
        &mut s,
        Action::Discard {
            seat: D,
            tile: t("1z"),
            riichi: false,
        },
    );
    // A が 1z をポン
    let opts = hands::pon_options(&s.seats[0].hand, t("1z"));
    assert_eq!(opts.len(), 1);
    act(
        &mut s,
        Action::Call {
            seat: A,
            kind: MeldKind::Pon,
            tiles: opts[0].clone(),
        },
    );
    assert_eq!(s.turn, Turn::Discard { seat: A });
    assert!(s.seats[3].river.last().unwrap().called);
    assert_eq!(s.seats[0].melds[0].from, Some(D));
    // 同じ牌が5枚になる入力は弾く
    act(
        &mut s,
        Action::Discard {
            seat: A,
            tile: t("2z"),
            riichi: false,
        },
    );
    assert!(
        apply(
            &mut s,
            &Action::Tsumo {
                seat: B,
                tile: t("5z")
            }
        )
        .is_err()
    );
    act(
        &mut s,
        Action::Tsumo {
            seat: B,
            tile: t("1z"),
        },
    );
}

#[test]
fn ankan_then_rinshan_and_win_input() {
    let mut s = started();
    act(
        &mut s,
        Action::Discard {
            seat: A,
            tile: t("5z"),
            riichi: false,
        },
    );
    act(
        &mut s,
        Action::Tsumo {
            seat: B,
            tile: t("9m"),
        },
    );
    act(
        &mut s,
        Action::Discard {
            seat: B,
            tile: t("9m"),
            riichi: false,
        },
    );
    act(
        &mut s,
        Action::Tsumo {
            seat: C,
            tile: t("9s"),
        },
    );
    act(
        &mut s,
        Action::Discard {
            seat: C,
            tile: t("9s"),
            riichi: false,
        },
    );
    act(
        &mut s,
        Action::Tsumo {
            seat: D,
            tile: t("8m"),
        },
    );
    // D の暗槓（4z）
    let (kind, tiles) = hands::self_kan(&s, D, t("4z")).unwrap();
    assert_eq!(kind, KanKind::Ankan);
    let wall = s.wall_remaining;
    act(
        &mut s,
        Action::Kan {
            seat: D,
            kind,
            new_dora_indicator: Some(t("2p")),
            tiles,
        },
    );
    assert_eq!(s.wall_remaining, wall - 1);
    assert_eq!(
        s.turn,
        Turn::Tsumo {
            seat: D,
            rinshan: true
        }
    );
    // 嶺上牌は残り牌数を減らさない
    act(
        &mut s,
        Action::Tsumo {
            seat: D,
            tile: t("9m"),
        },
    );
    assert_eq!(s.wall_remaining, wall - 1);
    assert_eq!(s.seats[3].tile_count(), 14);
    // ツモアガリの入力を手牌から作る（789m 8m9m? は形になっていなくても入力は作れる）
    let w = hands::win_input(&s, D, None).unwrap();
    assert_eq!(w.win_tile, t("9m"));
    assert_eq!(w.closed.len(), 11);
    assert_eq!(w.melds.len(), 1);
    // ロンは直前の打牌から
    act(
        &mut s,
        Action::Discard {
            seat: D,
            tile: t("9m"),
            riichi: false,
        },
    );
    let w = hands::win_input(&s, C, Some(D)).unwrap();
    assert_eq!(w.win_tile, t("9m"));
    assert_eq!(w.closed.len(), 14);
}

#[test]
fn chi_options_and_red_fives() {
    let hand = ts("340m67p");
    let o = hands::chi_options(&hand, t("5m"));
    // 34m・40m（赤5は 0m）
    assert!(o.contains(&ts("34m")));
    assert_eq!(o.len(), 1);
    assert_eq!(hands::chi_options(&ts("40m"), t("3m")), vec![ts("40m")]);
    let hand = ts("055p");
    let o = hands::pon_options(&hand, t("5p"));
    assert_eq!(o.len(), 2, "赤なし・赤ありの2通り");
}

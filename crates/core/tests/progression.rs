//! 卓進行の単体テスト（設計書 8章: 連荘・親流れ・終局・ダブロン・供託の行方・残り牌数と槓）。

use janscope_core::engine::{Overlay, apply};
use janscope_core::log::GameLog;
use janscope_core::rules::{GameConfig, GameLength, MultiRon};
use janscope_core::scoring::WinInput;
use janscope_core::state::*;
use janscope_core::tile::{SeatId, Tile, Wind, parse_tiles};
use janscope_core::view::PublicView;

const A: SeatId = SeatId(0);
const B: SeatId = SeatId(1);
const C: SeatId = SeatId(2);
const D: SeatId = SeatId(3);

fn players() -> [Player; 4] {
    ["選手A", "選手B", "選手C", "選手D"].map(Player::new)
}

fn start(cfg: GameConfig) -> TableState {
    let mut s = TableState::default();
    apply(
        &mut s,
        &Action::StartGame {
            config: cfg,
            players: players(),
            chiicha: A,
        },
    )
    .unwrap();
    s
}

fn hand(s: &mut TableState) {
    apply(s, &Action::StartHand { dice: vec![(3, 4)] }).unwrap();
}

/// 平和のみ（1翻30符）。子ロン 1,000 / 親ロン 1,500
fn pinfu(winner: SeatId, loser: Option<SeatId>) -> WinInput {
    WinInput {
        winner,
        loser,
        closed: parse_tiles("234567m345p678s99p").unwrap(),
        melds: vec![],
        win_tile: "2m".parse().unwrap(),
        riichi: false,
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

fn win(s: &mut TableState, wins: Vec<WinInput>) -> Vec<Overlay> {
    apply(s, &Action::Win { wins }).unwrap()
}

fn draw(s: &mut TableState, tenpai: Vec<SeatId>) {
    apply(
        s,
        &Action::Draw {
            tenpai,
            kind: DrawKind::Exhaustive,
        },
    )
    .unwrap();
}

#[test]
fn ron_and_dealer_rotation() {
    let mut s = start(GameConfig::preset_standard());
    assert_eq!(s.dealer(), A);
    hand(&mut s);
    win(&mut s, vec![pinfu(B, Some(A))]);
    assert_eq!(s.scores(), [24_000, 26_000, 25_000, 25_000]);
    assert_eq!((s.round_wind, s.round_number, s.honba), (Wind::East, 2, 0));
    assert_eq!(s.dealer(), B);
    assert_eq!(s.seat_wind(B), Wind::East);
    assert_eq!(s.phase, Phase::BetweenHands);
}

#[test]
fn dealer_win_renchan_and_honba() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    win(&mut s, vec![pinfu(A, Some(C))]);
    assert_eq!(s.scores(), [26_500, 25_000, 23_500, 25_000]);
    assert_eq!((s.round_number, s.honba), (1, 1));
    // 1本場: ロンは +300
    hand(&mut s);
    win(&mut s, vec![pinfu(B, Some(C))]);
    assert_eq!(s.scores()[1], 25_000 + 1_000 + 300);
    assert_eq!((s.round_number, s.honba), (2, 0));
}

#[test]
fn tsumo_with_honba() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    draw(&mut s, vec![A]); // 親テンパイ → 連荘、1本場
    assert_eq!(s.scores(), [28_000, 24_000, 24_000, 24_000]);
    hand(&mut s);
    let mut w = pinfu(C, None);
    w.closed = parse_tiles("234567m345p678s99p").unwrap();
    win(&mut s, vec![w]);
    // 子の平和ツモ 1翻20符… 平和+ツモ 2翻20符 = 400-700、1本場で各 +100
    assert_eq!(s.last_deltas, [-800, -500, 1_800, -500]);
}

#[test]
fn riichi_sticks_go_to_winner() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    apply(&mut s, &Action::Riichi { seat: B }).unwrap();
    apply(&mut s, &Action::Riichi { seat: C }).unwrap();
    assert_eq!(s.kyotaku, 2);
    let mut w = pinfu(B, Some(D));
    w.riichi = true;
    win(&mut s, vec![w]);
    // 立直平和 2翻30符 2,000 + 供託 2,000
    assert_eq!(s.scores(), [25_000, 24_000 + 4_000, 24_000, 23_000]);
    assert_eq!(s.kyotaku, 0);
}

#[test]
fn riichi_sticks_stay_on_draw() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    apply(&mut s, &Action::Riichi { seat: B }).unwrap();
    draw(&mut s, vec![B]);
    assert_eq!(s.kyotaku, 1);
    assert_eq!(s.scores(), [24_000, 24_000 + 3_000, 24_000, 24_000]);
    // 親ノーテン → 親流れ、本場は増える
    assert_eq!((s.round_number, s.honba), (2, 1));
}

#[test]
fn double_ron() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    apply(&mut s, &Action::Riichi { seat: A }).unwrap(); // 親が供託1本
    // C が放銃、D と A（親）のダブロン。供託は C から近い D
    let ov = win(&mut s, vec![pinfu(A, Some(C)), pinfu(D, Some(C))]);
    assert!(matches!(&ov[0], Overlay::Win { winners, .. } if winners == &vec![D, A]));
    assert_eq!(s.last_deltas, [1_500, 0, -2_500, 1_000 + 1_000]);
    // 親がアガっているので連荘
    assert_eq!((s.round_number, s.honba), (1, 1));
}

#[test]
fn head_bump() {
    let mut s = start(GameConfig {
        multi_ron: MultiRon::HeadBump,
        ..GameConfig::preset_standard()
    });
    hand(&mut s);
    win(&mut s, vec![pinfu(A, Some(C)), pinfu(D, Some(C))]);
    assert_eq!(s.last_deltas, [0, 0, -1_000, 1_000]);
    // 頭ハネで親はアガっていない → 親流れ
    assert_eq!(s.round_number, 2);
}

#[test]
fn wall_and_kan() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    assert_eq!(s.wall_remaining, 70);
    apply(&mut s, &Action::WallDecrement).unwrap();
    apply(&mut s, &Action::WallDecrement).unwrap();
    apply(&mut s, &Action::WallIncrement).unwrap();
    assert_eq!(s.wall_remaining, 69);
    let ind: Tile = "3p".parse().unwrap();
    apply(
        &mut s,
        &Action::Kan {
            seat: B,
            kind: KanKind::Ankan,
            new_dora_indicator: Some(ind),
        },
    )
    .unwrap();
    assert_eq!(s.wall_remaining, 68);
    assert_eq!(s.dora_indicators, vec![ind]);
    apply(&mut s, &Action::SetWallRemaining { value: 0 }).unwrap();
    assert!(apply(&mut s, &Action::WallDecrement).is_err());
    // 残り0のツモは海底
    let mut w = pinfu(B, None);
    w.closed = parse_tiles("234567m345p678s99p").unwrap();
    let sc = janscope_core::preview_win(&s, &w).unwrap();
    assert!(sc.yaku.iter().any(|y| y.name == "海底摸月"));
}

#[test]
fn hanchan_ends_after_south4() {
    let mut s = start(GameConfig::preset_standard());
    // 東1〜南4まで子のアガリで回す（親は毎回流れる）
    for _ in 0..8 {
        hand(&mut s);
        let dealer = s.dealer();
        let winner = dealer.next();
        win(&mut s, vec![pinfu(winner, Some(winner.next()))]);
    }
    assert_eq!(s.phase, Phase::GameOver);
    let fr = s.final_results.as_ref().unwrap();
    assert_eq!(fr.len(), 4);
    let total: f64 = fr.iter().map(|r| r.points).sum();
    assert!(total.abs() < 0.05, "ポイントの合計は0: {total}");
}

#[test]
fn tonpuu_and_all_last_stop() {
    let mut s = start(GameConfig {
        length: GameLength::Tonpuu,
        ..GameConfig::preset_standard()
    });
    for _ in 0..3 {
        hand(&mut s);
        let winner = s.dealer().next();
        win(&mut s, vec![pinfu(winner, Some(winner.next()))]);
    }
    assert!(s.is_all_last());
    assert_eq!(s.dealer(), D);
    // オーラスで親（D）がアガってトップになったら終局（アガリ止め）
    hand(&mut s);
    let mut big = pinfu(D, Some(A));
    big.closed = parse_tiles("123456789m11122z").unwrap();
    big.win_tile = "2z".parse().unwrap();
    win(&mut s, vec![big]);
    assert_eq!(s.rank_order()[0], D);
    assert_eq!(s.phase, Phase::GameOver);
}

#[test]
fn bust_ends() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    apply(
        &mut s,
        &Action::ManualAdjust {
            deltas: [-25_500, 25_500, 0, 0],
            reason: "テスト".into(),
        },
    )
    .unwrap();
    win(&mut s, vec![pinfu(B, Some(C))]);
    assert_eq!(s.phase, Phase::GameOver);
}

#[test]
fn west_extension() {
    let cfg = GameConfig {
        length: GameLength::Tonpuu,
        west_extension: true,
        ..GameConfig::preset_standard()
    };
    let mut s = start(cfg);
    for _ in 0..4 {
        hand(&mut s);
        let winner = s.dealer().next();
        win(&mut s, vec![pinfu(winner, Some(winner.next()))]);
    }
    // 誰も30,000に届いていない → 南入（東風戦の延長）
    assert_eq!(s.phase, Phase::BetweenHands);
    assert!(s.extension);
    assert_eq!((s.round_wind, s.round_number), (Wind::South, 1));
}

#[test]
fn abortive_draw_respects_rules() {
    let mut s = start(GameConfig::preset_pro_like());
    hand(&mut s);
    let r = apply(
        &mut s,
        &Action::Draw {
            tenpai: vec![],
            kind: DrawKind::Abortive(AbortiveKind::KyuushuKyuuhai),
        },
    );
    assert!(r.is_err());
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    apply(
        &mut s,
        &Action::Draw {
            tenpai: vec![],
            kind: DrawKind::Abortive(AbortiveKind::KyuushuKyuuhai),
        },
    )
    .unwrap();
    assert_eq!((s.round_number, s.honba), (1, 1));
}

#[test]
fn noten_dealer_passes_in_pro_rules() {
    let mut s = start(GameConfig::preset_pro_like());
    hand(&mut s);
    draw(&mut s, vec![A]);
    // テンパイ連荘なし → 親テンパイでも親流れ
    assert_eq!((s.round_number, s.honba), (2, 1));
}

#[test]
fn errors_do_not_change_state() {
    let mut s = start(GameConfig::preset_standard());
    let before = s.clone();
    assert!(apply(&mut s, &Action::Riichi { seat: A }).is_err()); // 局開始前
    assert!(apply(&mut s, &Action::StartHand { dice: vec![(7, 1)] }).is_err());
    assert_eq!(s, before);
}

#[test]
fn undo_redo_and_restore() {
    let dir = std::env::temp_dir().join(format!("janscope-test-{}", std::process::id()));
    let path = dir.join("game.jsonl");
    let _ = std::fs::remove_file(&path);
    let mut log = GameLog::create(&path).unwrap();
    log.act(
        Action::StartGame {
            config: GameConfig::default(),
            players: players(),
            chiicha: A,
        },
        1,
    )
    .unwrap();
    log.act(Action::StartHand { dice: vec![(1, 2)] }, 2)
        .unwrap();
    log.act(Action::Riichi { seat: C }, 3).unwrap();
    log.act(Action::WallDecrement, 4).unwrap();
    assert_eq!(log.state().kyotaku, 1);
    log.undo(5).unwrap();
    log.undo(6).unwrap();
    assert_eq!(log.state().kyotaku, 0);
    assert_eq!(log.state().seats[2].score, 25_000);
    log.redo(7).unwrap();
    assert_eq!(log.state().kyotaku, 1);
    let snapshot = log.state().clone();
    drop(log);
    let restored = GameLog::open(&path).unwrap();
    assert_eq!(restored.state(), &snapshot);
    assert!(restored.can_redo(), "Redo の候補も復元される");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn public_view_has_no_hand_tiles() {
    let mut s = start(GameConfig::preset_standard());
    hand(&mut s);
    win(&mut s, vec![pinfu(B, Some(A))]);
    let v = PublicView::from_state(&s);
    let json = serde_json::to_string(&v).unwrap();
    // 手牌（門前の牌・面子分解）が含まれない
    assert!(!json.contains("closed"));
    assert!(!json.contains("groups"));
    assert!(!json.contains("\"7m\""));
    assert!(json.contains("平和"));
}

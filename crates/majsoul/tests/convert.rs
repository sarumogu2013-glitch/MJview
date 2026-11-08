//! 牌譜の変換を、作った記録で確かめる（本物の雀魂の牌譜の項目名に合わせてある）。

use janscope_core::engine::apply;
use janscope_core::state::{Action, Phase, TableState};
use janscope_majsoul::convert::{Record, convert, load};
use serde_json::json;

fn rec(name: &str, body: serde_json::Value) -> Record {
    Record {
        name: format!("lq.{name}"),
        body,
    }
}

fn head() -> serde_json::Value {
    json!({
        "uuid": "test-uuid",
        "config": {"mode": {"mode": 2}},
        "accounts": [
            {"seat": 0, "nickname": "Aさん"}, {"seat": 1, "nickname": "Bさん"},
            {"seat": 2, "nickname": "Cさん"}, {"seat": 3, "nickname": "Dさん"}
        ]
    })
}

fn game() -> Vec<Record> {
    vec![
        // 東1局: 席1 がリーチして、席0 からロン（リーチ・平和… 点数は雀魂の値に合わせる）
        rec(
            "RecordNewRound",
            json!({"chang": 0, "ju": 0, "ben": 0, "liqibang": 0,
            "scores": [25000, 25000, 25000, 25000], "doras": ["1s"], "left_tile_count": 69}),
        ),
        rec(
            "RecordDealTile",
            json!({"seat": 0, "tile": "3m", "left_tile_count": 68}),
        ),
        rec("RecordDiscardTile", json!({"seat": 0, "tile": "9p"})),
        rec(
            "RecordDealTile",
            json!({"seat": 1, "tile": "4m", "left_tile_count": 67}),
        ),
        rec(
            "RecordDiscardTile",
            json!({"seat": 1, "tile": "1z", "is_liqi": true}),
        ),
        rec(
            "RecordDealTile",
            json!({"seat": 2, "tile": "4m", "left_tile_count": 66}),
        ),
        rec("RecordDiscardTile", json!({"seat": 2, "tile": "2m"})),
        rec(
            "RecordHule",
            json!({
                "hules": [{"seat": 1, "zimo": false, "liqi": true,
                    "hand": ["3m","4m","5m","6m","7m","3p","4p","5p","6s","7s","8s","9p","9p"],
                    "ming": [], "hu_tile": "2m", "li_doras": ["5z"],
                    "fans": [{"name": "立直", "val": 1}, {"name": "平和", "val": 1}]}],
                "scores": [25000, 28900, 21100, 25000]
            }),
        ),
        // 東2局: 席2 がツモ（副露あり）
        rec(
            "RecordNewRound",
            json!({"chang": 0, "ju": 1, "ben": 0, "liqibang": 0,
            "scores": [25000, 28900, 21100, 25000], "doras": ["6m"], "left_tile_count": 69}),
        ),
        rec(
            "RecordChiPengGang",
            json!({"seat": 2, "type": 1, "tiles": ["5z","5z","5z"]}),
        ),
        rec(
            "RecordDealTile",
            json!({"seat": 2, "tile": "9s", "left_tile_count": 50}),
        ),
        rec(
            "RecordHule",
            json!({
                "hules": [{"seat": 2, "zimo": true, "liqi": false,
                    "hand": ["1m","2m","3m","4p","5p","6p","7s","8s","1z","1z"],
                    "ming": ["kezi(5z,5z,5z)"], "hu_tile": "9s",
                    "fans": [{"name": "役牌 中", "val": 1}]}],
                "scores": [24300, 28200, 23200, 24300]
            }),
        ),
        // 東3局: 読めない牌のアガリ → 途中流局で終えて点数を合わせる
        rec(
            "RecordNewRound",
            json!({"chang": 0, "ju": 2, "ben": 0, "liqibang": 0,
            "scores": [24300, 28200, 23200, 24300], "doras": ["2p"], "left_tile_count": 69}),
        ),
        rec(
            "RecordHule",
            json!({
                "hules": [{"seat": 0, "zimo": true, "hand": ["xx"], "ming": [], "hu_tile": "1m", "fans": []}],
                "scores": [30300, 26200, 21200, 22300]
            }),
        ),
        // 東4局: 流局（席0・席3 テンパイ）
        rec(
            "RecordNewRound",
            json!({"chang": 0, "ju": 3, "ben": 0, "liqibang": 0,
            "scores": [30300, 26200, 21200, 22300], "doras": ["7z"], "left_tile_count": 69}),
        ),
        rec(
            "RecordNoTile",
            json!({
                "players": [{"tingpai": true}, {"tingpai": false}, {"tingpai": false}, {"tingpai": true}],
                "scores": [{"old_scores": [30300, 26200, 21200, 22300], "delta_scores": [1500, -1500, -1500, 1500]}]
            }),
        ),
    ]
}

#[test]
fn converts_and_replays_to_same_scores() {
    let imp = convert(&head(), &game(), true).unwrap();
    // 名前は匿名
    match &imp.actions[0] {
        Action::StartGame { players, .. } => assert_eq!(players[1].name, "選手B"),
        a => panic!("{a:?}"),
    }
    assert!(
        imp.actions
            .iter()
            .any(|a| matches!(a, Action::Riichi { .. }))
    );
    assert!(imp.actions.iter().any(|a| matches!(a, Action::Win { .. })));
    assert!(imp.actions.iter().any(|a| matches!(a, Action::Draw { .. })));
    assert!(matches!(imp.actions.last(), Some(Action::EndGame)));
    // 東3局は変換で合わせたことが記録に残る
    assert!(
        imp.notes.iter().any(|n| n.contains("途中流局")),
        "{:?}",
        imp.notes
    );

    // 最初から当て直すと、最後の点数が雀魂と同じ（流局の後は終局で供託なし）
    let mut s = TableState::default();
    for a in &imp.actions {
        apply(&mut s, a).unwrap_or_else(|e| panic!("{a:?}: {e}"));
    }
    assert_eq!(s.phase, Phase::GameOver);
    assert_eq!(s.scores(), [31800, 24700, 19700, 23800]);

    // ファイルにして読み戻せる
    let text = serde_json::to_string(&imp).unwrap();
    let back = load(&text).unwrap();
    assert_eq!(back.actions.len(), imp.actions.len());
}

#[test]
fn keeps_names_when_asked_and_rejects_sanma() {
    let imp = convert(&head(), &game(), false).unwrap();
    match &imp.actions[0] {
        Action::StartGame { players, .. } => assert_eq!(players[0].name, "Aさん"),
        a => panic!("{a:?}"),
    }
    let mut h = head();
    h["config"]["mode"]["mode"] = json!(12);
    assert!(convert(&h, &game(), true).unwrap_err().contains("三人"));
}

/// 配牌（tiles0〜3）があれば、ツモ・打牌・鳴きも1枚ずつ入る
#[test]
fn tiles_are_tracked() {
    let t = |s: &str| -> Vec<serde_json::Value> {
        janscope_core::tile::parse_tiles(s)
            .unwrap()
            .iter()
            .map(|x| json!(x.to_string()))
            .collect()
    };
    let recs = vec![
        rec(
            "RecordNewRound",
            json!({"chang": 0, "ju": 0, "ben": 0, "liqibang": 0,
            "scores": [25000, 25000, 25000, 25000], "doras": ["1s"], "left_tile_count": 69,
            "tiles0": t("123m456p789s11225z"), "tiles1": t("777z11m234p567s99s"),
            "tiles2": t("456m11p23s666z333z"), "tiles3": t("789m999p111s4444z")}),
        ),
        // 親が 白 を打つ → 席3 のツモ… ではなく、席1 のツモ 9m・打牌 9m
        rec("RecordDiscardTile", json!({"seat": 0, "tile": "5z"})),
        rec(
            "RecordDealTile",
            json!({"seat": 1, "tile": "9m", "left_tile_count": 68}),
        ),
        rec("RecordDiscardTile", json!({"seat": 1, "tile": "9m"})),
        rec(
            "RecordDealTile",
            json!({"seat": 2, "tile": "1z", "left_tile_count": 67}),
        ),
        rec("RecordDiscardTile", json!({"seat": 2, "tile": "1z"})),
        // 席0 が 東 をポン（自分の2枚 + 席2 の1枚）
        rec(
            "RecordChiPengGang",
            json!({"seat": 0, "type": 1, "tiles": ["1z","1z","1z"], "froms": [0, 0, 2]}),
        ),
        rec("RecordDiscardTile", json!({"seat": 0, "tile": "2z"})),
    ];
    let imp = convert(&head(), &recs, true).unwrap();
    let mut s = TableState::default();
    let mut seen_call = false;
    for a in &imp.actions {
        if matches!(a, Action::Call { .. }) {
            seen_call = true;
        }
        // 最後の局は途中で終わるので、流局（途中流局）までを当てる
        if apply(&mut s, a).is_err() {
            break;
        }
        if s.seats[2].river.len() == 1 && s.seats[0].melds.len() == 1 && s.seats[0].river.len() == 2
        {
            break;
        }
    }
    assert!(seen_call, "ポンが操作に入っていない: {:?}", imp.notes);
    assert_eq!(s.seats[0].river.len(), 2, "{:?}", imp.notes);
    assert!(s.seats[2].river[0].called);
    assert_eq!(s.seats[0].melds.len(), 1);
}

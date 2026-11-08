//! 雀魂の牌譜（`ResGameRecord` と `GameDetailRecords`）を、JanScope の対局履歴（[`Action`] の列）に変換する。
//!
//! - 局の始めごとに、点数・局・本場・供託を雀魂の値に合わせる（ルールの違いで食い違っても、次の局からは正しい）
//! - アガリ・流局は JanScope の操作で当て、点数が雀魂と違えば差を「手動調整」で合わせる。
//!   JanScope の操作として当てられないとき（ルールの違いなど）は、途中流局で局を終えて点数を合わせる
//! - 打牌の1枚1枚は持たない（JanScope の卓状態は河を持たない）。ツモごとに残り牌数、リーチ、槓、ドラ表示牌を入れる
//! - 名前は既定で「選手A〜D」にする（ほかの人の名前を入れないため）

use janscope_core::engine::apply;
use janscope_core::rules::GameConfig;
use janscope_core::scoring::{Meld, MeldKind, WinInput};
use janscope_core::state::{AbortiveKind, Action, DrawKind, KanKind, Phase, Player, TableState};
use janscope_core::tile::{SeatId, Tile, Wind};
use serde_json::Value;

use crate::proto::{Schema, unb64};

pub use janscope_core::dummy::{DummyFile as Imported, FORMAT, load};

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn int(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}
fn boolean(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn string<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn tile(s: &str) -> Result<Tile, String> {
    s.parse().map_err(|_| format!("牌 {s} を読めません"))
}
fn tiles(v: &[Value]) -> Result<Vec<Tile>, String> {
    v.iter().map(|t| tile(t.as_str().unwrap_or(""))).collect()
}
fn scores4(v: &[Value]) -> Option<[i32; 4]> {
    if v.len() < 4 {
        return None;
    }
    Some([0, 1, 2, 3].map(|i| v[i].as_i64().unwrap_or(0) as i32))
}

/// 牌譜の1件（`Wrapper` をほどいたもの）
pub struct Record {
    pub name: String,
    pub body: Value,
}

/// `GameDetailRecords` の bytes（`Wrapper` に包まれている）から、記録を順に取り出す
pub fn records(schema: &Schema, data: &[u8]) -> Result<Vec<Record>, String> {
    let w = schema.decode("lq.Wrapper", data)?;
    let name = string(&w, "name").trim_start_matches('.').to_string();
    let inner = w.get("data").map(unb64).unwrap_or_default();
    let detail = schema.decode(
        if name.is_empty() {
            "lq.GameDetailRecords"
        } else {
            &name
        },
        &inner,
    )?;
    let mut out = vec![];
    let mut push = |b: &[u8]| -> Result<(), String> {
        if b.is_empty() {
            return Ok(());
        }
        let w = schema.decode("lq.Wrapper", b)?;
        let n = string(&w, "name").trim_start_matches('.').to_string();
        let body = schema.decode(&n, &w.get("data").map(unb64).unwrap_or_default())?;
        out.push(Record { name: n, body });
        Ok(())
    };
    // 古い形式は records、新しい形式は actions[].result
    for r in arr(&detail, "records") {
        push(&unb64(r))?;
    }
    for a in arr(&detail, "actions") {
        if let Some(r) = a.get("result") {
            push(&unb64(r))?;
        }
    }
    if out.is_empty() {
        return Err("牌譜に記録がありません".into());
    }
    Ok(out)
}

struct Conv {
    s: TableState,
    actions: Vec<Action>,
    notes: Vec<String>,
    last_discarder: Option<u8>,
    last_kakan: Option<u8>,
    dora: Vec<Tile>,
}

impl Conv {
    /// 当てて、だめなら Err（操作は加えない）
    fn try_act(&mut self, a: Action) -> Result<(), String> {
        let mut t = self.s.clone();
        apply(&mut t, &a).map_err(|e| e.to_string())?;
        self.s = t;
        self.actions.push(a);
        Ok(())
    }
    /// 当てる。だめなら記録だけ残して飛ばす
    fn act(&mut self, a: Action) {
        let label = format!("{a:?}");
        if let Err(e) = self.try_act(a) {
            self.notes.push(format!(
                "飛ばした操作: {} （{e}）",
                label.chars().take(60).collect::<String>()
            ));
        }
    }
    /// 点数を雀魂の値に合わせる
    fn sync_scores(&mut self, want: [i32; 4], why: &str) {
        let have = self.s.scores();
        let d = [0, 1, 2, 3].map(|i| want[i] - have[i]);
        if d != [0; 4] {
            self.notes
                .push(format!("{why}: 点数を雀魂に合わせました {d:?}"));
            self.act(Action::ManualAdjust {
                deltas: d,
                reason: format!("雀魂の点数に合わせる（{why}）"),
            });
        }
    }
    fn sync_dora(&mut self, rec: &Value) {
        let d = arr(rec, "doras");
        if d.is_empty() {
            return;
        }
        if let Ok(ts) = tiles(d) {
            if ts != self.dora && self.s.phase == Phase::InHand {
                self.dora = ts.clone();
                self.act(Action::SetDora { tiles: ts });
            }
        }
    }
    /// アガリ・流局を当てられなかったときに局を終える
    fn end_hand_fallback(&mut self, want: Option<[i32; 4]>, why: &str) {
        if self.s.phase == Phase::InHand {
            let _ = self.try_act(Action::Draw {
                tenpai: vec![],
                kind: DrawKind::Abortive(AbortiveKind::KyuushuKyuuhai),
            });
            self.notes.push(format!(
                "{why}: JanScope の操作で当てられないので、途中流局で局を終えました"
            ));
        }
        if let Some(w) = want {
            self.sync_scores(w, why);
        }
    }
}

fn meld_from_ming(s: &str, winner: u8) -> Result<Meld, String> {
    // 例: "shunzi(1m,2m,3m)" "kezi(5z,5z,5z)" "minggang(...)" "angang(...)"
    let (kind, rest) = s
        .split_once('(')
        .ok_or_else(|| format!("副露 {s} を読めません"))?;
    let ts: Result<Vec<Tile>, String> = rest
        .trim_end_matches(')')
        .split(',')
        .map(|t| tile(t.trim()))
        .collect();
    let ts = ts?;
    let kamicha = Some(SeatId((winner + 3) % 4));
    let (kind, from) = match kind {
        "shunzi" => (MeldKind::Chi, kamicha),
        "kezi" => (MeldKind::Pon, kamicha),
        "minggang" => (MeldKind::Minkan, kamicha),
        "angang" => (MeldKind::Ankan, None),
        k => return Err(format!("副露の種類 {k} を読めません")),
    };
    Ok(Meld {
        kind,
        tiles: ts,
        from,
    })
}

fn fan_names(h: &Value) -> Vec<String> {
    arr(h, "fans")
        .iter()
        .map(|f| string(f, "name").to_string())
        .collect()
}

fn win_input(h: &Value, loser: Option<u8>) -> Result<WinInput, String> {
    let winner = int(h, "seat") as u8;
    let mut closed = tiles(arr(h, "hand"))?;
    let hu = tile(string(h, "hu_tile"))?;
    closed.push(hu);
    let melds = arr(h, "ming")
        .iter()
        .map(|m| meld_from_ming(m.as_str().unwrap_or(""), winner))
        .collect::<Result<Vec<_>, _>>()?;
    let fans = fan_names(h);
    let has = |k: &[&str]| fans.iter().any(|f| k.iter().any(|x| f.contains(x)));
    let zimo = boolean(h, "zimo");
    let double = has(&["两立直", "両立直", "ダブル立直", "Double Riichi"]);
    let riichi = boolean(h, "liqi") || double || has(&["立直", "Riichi"]);
    Ok(WinInput {
        winner: SeatId(winner),
        loser: if zimo { None } else { loser.map(SeatId) },
        closed,
        melds,
        win_tile: hu,
        riichi,
        double_riichi: double,
        ippatsu: has(&["一发", "一発", "Ippatsu"]),
        haitei: has(&["海底", "河底", "Haitei", "Houtei"]),
        rinshan: has(&["岭上", "嶺上", "Rinshan"]),
        chankan: has(&["抢杠", "槍槓", "Chankan"]),
        tenhou: has(&["天和", "地和", "Tenhou", "Chiihou"]),
        ura_indicators: if riichi {
            tiles(arr(h, "li_doras")).unwrap_or_default()
        } else {
            vec![]
        },
        pao: None,
    })
}

fn abortive(t: i64) -> AbortiveKind {
    match t {
        2 => AbortiveKind::SuufonRenda,
        3 => AbortiveKind::SuukanSanra,
        4 => AbortiveKind::SuuchaRiichi,
        5 => AbortiveKind::SanchaHou,
        _ => AbortiveKind::KyuushuKyuuhai,
    }
}

/// 変換する。`head` は `ResGameRecord.head`（RecordGame）、`recs` は [`records`] の結果
pub fn convert(head: &Value, recs: &[Record], anonymize: bool) -> Result<Imported, String> {
    let mode = head
        .get("config")
        .and_then(|c| c.get("mode"))
        .map(|m| int(m, "mode"))
        .unwrap_or(2);
    if mode >= 11 {
        return Err("三人麻雀の牌譜は取り込めません（JanScope は四人麻雀だけ）".into());
    }
    let mut config = if mode == 1 {
        GameConfig::preset_tonpuu()
    } else {
        GameConfig::preset_standard()
    };
    config.name = format!("雀魂（{}）", if mode == 1 { "東風" } else { "半荘" });
    let mut names = ["選手A", "選手B", "選手C", "選手D"].map(String::from);
    if !anonymize {
        for a in arr(head, "accounts") {
            let seat = int(a, "seat") as usize;
            let n = string(a, "nickname");
            if seat < 4 && !n.is_empty() {
                names[seat] = n.to_string();
            }
        }
    }
    let uuid = string(head, "uuid").to_string();
    let mut c = Conv {
        s: TableState::default(),
        actions: vec![],
        notes: vec![],
        last_discarder: None,
        last_kakan: None,
        dora: vec![],
    };
    // 雀魂の席 0 が起家。JanScope の席 A〜D をそのまま席 0〜3 にする
    c.try_act(Action::StartGame {
        config,
        players: names.map(Player::new),
        chiicha: SeatId(0),
    })
    .map_err(|e| format!("半荘を始められません: {e}"))?;

    for r in recs {
        let b = &r.body;
        match r.name.rsplit('.').next().unwrap_or("") {
            "RecordNewRound" => {
                if c.s.phase == Phase::InHand {
                    c.end_hand_fallback(None, "前の局の終わりがない");
                }
                if c.s.phase == Phase::GameOver {
                    c.notes
                        .push("JanScope のルールでは終局になったので、ここで打ち切りました".into());
                    break;
                }
                let label = format!(
                    "{}{}局",
                    ["東", "南", "西", "北"][int(b, "chang").clamp(0, 3) as usize],
                    int(b, "ju") + 1
                );
                if let Some(want) = scores4(arr(b, "scores")) {
                    c.sync_scores(want, &label);
                }
                let wind = [Wind::East, Wind::South, Wind::West, Wind::North]
                    [int(b, "chang").clamp(0, 3) as usize];
                c.act(Action::SetRound {
                    wind,
                    number: (int(b, "ju").clamp(0, 3) + 1) as u8,
                    honba: int(b, "ben").max(0) as u32,
                    kyotaku: int(b, "liqibang").max(0) as u32,
                });
                c.act(Action::StartHand { dice: vec![] });
                c.dora.clear();
                let mut d = tiles(arr(b, "doras")).unwrap_or_default();
                if d.is_empty() {
                    if let Ok(t) = tile(string(b, "dora")) {
                        d.push(t);
                    }
                }
                if !d.is_empty() {
                    c.dora = d.clone();
                    c.act(Action::SetDora { tiles: d });
                }
                let left = int(b, "left_tile_count");
                if left > 0 {
                    c.act(Action::SetWallRemaining { value: left as u32 });
                }
                c.last_discarder = None;
                c.last_kakan = None;
            }
            "RecordDealTile" => {
                let left = int(b, "left_tile_count");
                if left >= 0
                    && b.get("left_tile_count").is_some()
                    && c.s.wall_remaining != left as u32
                {
                    c.act(Action::SetWallRemaining { value: left as u32 });
                }
                c.sync_dora(b);
            }
            "RecordDiscardTile" => {
                let seat = int(b, "seat") as u8;
                if (boolean(b, "is_liqi") || boolean(b, "is_wliqi"))
                    && !c.s.seats[seat as usize].riichi
                {
                    c.act(Action::Riichi { seat: SeatId(seat) });
                }
                c.last_discarder = Some(seat);
                c.last_kakan = None;
                c.sync_dora(b);
            }
            "RecordChiPengGang" => {
                // 2 = 大明槓（チー・ポンは操作にしない。アガリのときの副露は牌譜から入れる）
                if int(b, "type") == 2 {
                    c.act(Action::Kan {
                        seat: SeatId(int(b, "seat") as u8),
                        kind: KanKind::Minkan,
                        new_dora_indicator: None,
                    });
                }
                c.sync_dora(b);
            }
            "RecordAnGangAddGang" => {
                let seat = int(b, "seat") as u8;
                let kind = if int(b, "type") == 3 {
                    KanKind::Ankan
                } else {
                    KanKind::Kakan
                };
                if kind == KanKind::Kakan {
                    c.last_kakan = Some(seat);
                }
                c.act(Action::Kan {
                    seat: SeatId(seat),
                    kind,
                    new_dora_indicator: None,
                });
                c.sync_dora(b);
            }
            "RecordHule" => {
                let want = scores4(arr(b, "scores")).or_else(|| {
                    let old = scores4(arr(b, "old_scores"))?;
                    let d = scores4(arr(b, "delta_scores"))?;
                    Some([0, 1, 2, 3].map(|i| old[i] + d[i]))
                });
                let loser = c.last_kakan.or(c.last_discarder);
                let wins: Result<Vec<WinInput>, String> = arr(b, "hules")
                    .iter()
                    .map(|h| win_input(h, loser))
                    .collect();
                match wins {
                    Ok(w) if !w.is_empty() => match c.try_act(Action::Win { wins: w }) {
                        Ok(()) => {
                            if let Some(want) = want {
                                c.sync_scores(want, "アガリ");
                            }
                        }
                        Err(e) => c.end_hand_fallback(want, &format!("アガリ（{e}）")),
                    },
                    Ok(_) => c.end_hand_fallback(want, "アガリ（中身なし）"),
                    Err(e) => c.end_hand_fallback(want, &format!("アガリ（{e}）")),
                }
            }
            "RecordNoTile" => {
                let tenpai: Vec<SeatId> = arr(b, "players")
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| boolean(p, "tingpai"))
                    .map(|(i, _)| SeatId(i as u8))
                    .collect();
                let want = arr(b, "scores").first().and_then(|sc| {
                    let old = scores4(arr(sc, "old_scores"))?;
                    let d = scores4(arr(sc, "delta_scores")).unwrap_or([0; 4]);
                    Some([0, 1, 2, 3].map(|i| old[i] + d[i]))
                });
                let kind = if boolean(b, "liujumanguan") {
                    // 流し満貫の達成者は点数の動きから（受け取った人）
                    let ms: Vec<SeatId> = arr(b, "scores")
                        .iter()
                        .filter_map(|sc| {
                            let d = scores4(arr(sc, "delta_scores"))?;
                            (0..4).max_by_key(|&i| d[i]).map(|i| SeatId(i as u8))
                        })
                        .collect();
                    DrawKind::NagashiMangan(ms)
                } else {
                    DrawKind::Exhaustive
                };
                match c.try_act(Action::Draw { tenpai, kind }) {
                    Ok(()) => {
                        if let Some(w) = want {
                            c.sync_scores(w, "流局");
                        }
                    }
                    Err(e) => c.end_hand_fallback(want, &format!("流局（{e}）")),
                }
            }
            "RecordLiuJu" => {
                let kind = DrawKind::Abortive(abortive(int(b, "type")));
                if let Err(e) = c.try_act(Action::Draw {
                    tenpai: vec![],
                    kind,
                }) {
                    c.end_hand_fallback(None, &format!("途中流局（{e}）"));
                }
            }
            _ => {}
        }
    }
    if c.s.phase == Phase::InHand {
        c.end_hand_fallback(None, "最後の局の終わりがない");
    }
    if c.s.phase == Phase::BetweenHands {
        c.act(Action::EndGame);
    }
    let title = format!(
        "雀魂 {}{}",
        if mode == 1 { "東風戦" } else { "半荘戦" },
        if uuid.is_empty() {
            String::new()
        } else {
            format!("（{uuid}）")
        }
    );
    Ok(Imported {
        format: FORMAT.into(),
        source: "majsoul".into(),
        uuid,
        title,
        actions: c.actions,
        notes: c.notes,
    })
}

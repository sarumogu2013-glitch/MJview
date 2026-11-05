//! 操作（Action）を卓状態に適用する（局の進行・精算。設計書 5.1・5.2）。
//!
//! 精算の約束（ルールで変えられない部分）:
//! - ダブロン・トリロン: 本場は各アガリに付く。供託は放銃者から見て近い（下家側の）人が受け取る（上家取り）
//! - 包（パオ）: 役満のツモは責任者が全額（ロン相当）を払う。ロンは放銃者と責任者が半分ずつ（本場は放銃者）
//! - 流し満貫: 子は 2,000-4,000、親は 4,000オール相当（本場なし）。ノーテン罰符は取らない
//! - 西入（延長）: 誰かが返し以上になった時点で終局。西4局で必ず終局

use serde::{Deserialize, Serialize};

use crate::rules::{MultiRon, TieBreak};
use crate::scoring::{Limit, ScoreContext, ScoreError, WinInput, WinScore, score_win};
use crate::state::*;
use crate::tile::{SeatId, Wind};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum EngineError {
    #[error("{0}")]
    Invalid(String),
    #[error("アガリの計算ができません（{seat}）: {error}")]
    Score { seat: char, error: ScoreError },
}

fn invalid<T>(msg: impl Into<String>) -> Result<T, EngineError> {
    Err(EngineError::Invalid(msg.into()))
}

/// 演出用の出来事（設計書 6章 OverlayEvent）。選手画面の自動切替のきっかけにも使う。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Overlay {
    GameStarted,
    HandStarted,
    Riichi {
        seat: SeatId,
    },
    Kan {
        seat: SeatId,
    },
    /// アガリ演出（history の最後の局）。winners は演出の順
    Win {
        winners: Vec<SeatId>,
        loser: Option<SeatId>,
    },
    Draw,
    ScoreChange {
        deltas: [i32; 4],
    },
    GameOver,
}

/// アガリのプレビュー（状態は変えない。設計書 4.4「プレビュー → 放送に出す」）。
pub fn preview_win(state: &TableState, input: &WinInput) -> Result<WinScore, EngineError> {
    score_in_state(state, input)
}

fn score_in_state(state: &TableState, input: &WinInput) -> Result<WinScore, EngineError> {
    let mut inp = input.clone();
    if state.phase == Phase::InHand && state.wall_remaining == 0 && !inp.rinshan {
        inp.haitei = true;
    }
    let ctx = ScoreContext {
        config: &state.config,
        round_wind: state.round_wind,
        seat_wind: state.seat_wind(input.winner),
        dora_indicators: &state.dora_indicators,
    };
    score_win(&inp, &ctx).map_err(|error| EngineError::Score {
        seat: input.winner.label(),
        error,
    })
}

/// 操作を適用する。失敗したら状態は変わらない。
pub fn apply(state: &mut TableState, action: &Action) -> Result<Vec<Overlay>, EngineError> {
    let mut next = state.clone();
    let ov = apply_inner(&mut next, action)?;
    *state = next;
    Ok(ov)
}

fn need_phase(state: &TableState, phases: &[Phase], what: &str) -> Result<(), EngineError> {
    if phases.contains(&state.phase) {
        Ok(())
    } else {
        let now = match state.phase {
            Phase::Setup => "半荘の開始前",
            Phase::BetweenHands => "局の開始前",
            Phase::InHand => "局の途中",
            Phase::GameOver => "終局後",
        };
        invalid(format!("{now}なので「{what}」はできません"))
    }
}

fn apply_inner(s: &mut TableState, action: &Action) -> Result<Vec<Overlay>, EngineError> {
    use Action::*;
    match action {
        StartGame {
            config,
            players,
            chiicha,
        } => {
            let errs = config.validate();
            if !errs.is_empty() {
                return invalid(format!(
                    "ルールの設定に誤りがあります: {}",
                    errs.join(" / ")
                ));
            }
            *s = TableState::default();
            s.config = config.clone();
            s.chiicha = *chiicha;
            for (i, p) in players.iter().enumerate() {
                s.seats[i].player = p.clone();
                s.seats[i].score = config.start_points;
            }
            s.phase = Phase::BetweenHands;
            s.wall_remaining = config.wall_start;
            Ok(vec![Overlay::GameStarted])
        }
        StartHand { dice } => {
            need_phase(s, &[Phase::BetweenHands], "局開始")?;
            check_dice(dice, s.config.dice_twice)?;
            s.dice = dice.clone();
            s.wall_remaining = s.config.wall_start;
            s.dora_indicators.clear();
            s.kans_this_hand = 0;
            s.last_deltas = [0; 4];
            for seat in &mut s.seats {
                seat.riichi = false;
                seat.melds.clear();
            }
            s.phase = Phase::InHand;
            Ok(vec![Overlay::HandStarted])
        }
        SetDice { dice } => {
            need_phase(s, &[Phase::InHand, Phase::BetweenHands], "サイコロの修正")?;
            check_dice(dice, s.config.dice_twice)?;
            s.dice = dice.clone();
            Ok(vec![])
        }
        WallDecrement => {
            need_phase(s, &[Phase::InHand], "残り牌数 −1")?;
            if s.wall_remaining == 0 {
                return invalid("残り牌数はすでに0です");
            }
            s.wall_remaining -= 1;
            Ok(vec![])
        }
        WallIncrement => {
            need_phase(s, &[Phase::InHand], "残り牌数 +1")?;
            if s.wall_remaining >= s.config.wall_start {
                return invalid("残り牌数は初期値より増やせません");
            }
            s.wall_remaining += 1;
            Ok(vec![])
        }
        SetWallRemaining { value } => {
            need_phase(s, &[Phase::InHand], "残り牌数の修正")?;
            if *value > s.config.wall_start {
                return invalid(format!("残り牌数は0〜{}です", s.config.wall_start));
            }
            s.wall_remaining = *value;
            Ok(vec![])
        }
        Riichi { seat } => {
            need_phase(s, &[Phase::InHand], "立直")?;
            let st = &mut s.seats[seat.index()];
            if st.riichi {
                return invalid(format!("{}はすでに立直しています", st.player.name));
            }
            st.riichi = true;
            st.score -= 1_000;
            s.kyotaku += 1;
            Ok(vec![Overlay::Riichi { seat: *seat }])
        }
        CancelRiichi { seat } => {
            need_phase(s, &[Phase::InHand], "立直の取り消し")?;
            let st = &mut s.seats[seat.index()];
            if !st.riichi {
                return invalid(format!("{}は立直していません", st.player.name));
            }
            st.riichi = false;
            st.score += 1_000;
            s.kyotaku = s.kyotaku.saturating_sub(1);
            Ok(vec![])
        }
        Kan {
            seat,
            kind: _,
            new_dora_indicator,
        } => {
            need_phase(s, &[Phase::InHand], "槓")?;
            if s.kans_this_hand >= 4 {
                return invalid("この局の槓はすでに4回です");
            }
            s.kans_this_hand += 1;
            s.wall_remaining = s.wall_remaining.saturating_sub(1);
            if let Some(t) = new_dora_indicator {
                if s.dora_indicators.len() >= 5 {
                    return invalid("ドラ表示牌は5枚までです");
                }
                s.dora_indicators.push(*t);
            }
            Ok(vec![Overlay::Kan { seat: *seat }])
        }
        SetDora { tiles } => {
            need_phase(s, &[Phase::InHand], "ドラ表示牌の入力")?;
            if tiles.len() > 5 {
                return invalid("ドラ表示牌は5枚までです");
            }
            s.dora_indicators = tiles.clone();
            Ok(vec![])
        }
        Win { wins } => {
            need_phase(s, &[Phase::InHand], "アガリ")?;
            settle_wins(s, wins)
        }
        Draw { tenpai, kind } => {
            need_phase(s, &[Phase::InHand], "流局")?;
            settle_draw(s, tenpai, kind)
        }
        ManualAdjust { deltas, reason } => {
            need_phase(
                s,
                &[Phase::InHand, Phase::BetweenHands, Phase::GameOver],
                "点数の手動調整",
            )?;
            if reason.trim().is_empty() {
                return invalid("調整の理由を入力してください");
            }
            for i in 0..4 {
                s.seats[i].score += deltas[i];
            }
            s.last_deltas = *deltas;
            s.history.push(HandRecord {
                wind: s.round_wind,
                number: s.round_number,
                honba: s.honba,
                kyotaku_before: s.kyotaku,
                outcome: HandOutcome::Adjust {
                    reason: reason.clone(),
                },
                deltas: *deltas,
                scores_after: s.scores(),
                dora_indicators: s.dora_indicators.clone(),
            });
            if s.phase == Phase::GameOver {
                s.final_results = Some(final_results(s));
            }
            Ok(vec![Overlay::ScoreChange { deltas: *deltas }])
        }
        SetRound {
            wind,
            number,
            honba,
            kyotaku,
        } => {
            need_phase(
                s,
                &[Phase::InHand, Phase::BetweenHands],
                "局・本場・供託の修正",
            )?;
            if !(1..=4).contains(number) {
                return invalid("局数は1〜4です");
            }
            s.round_wind = *wind;
            s.round_number = *number;
            s.honba = *honba;
            s.kyotaku = *kyotaku;
            s.extension = wind.index() > s.config.last_regular_wind().index();
            Ok(vec![])
        }
        SetPlayer { seat, player } => {
            s.seats[seat.index()].player = player.clone();
            Ok(vec![])
        }
        SetConfig { config } => {
            need_phase(
                s,
                &[Phase::InHand, Phase::BetweenHands, Phase::GameOver],
                "ルールの変更",
            )?;
            let errs = config.validate();
            if !errs.is_empty() {
                return invalid(format!(
                    "ルールの設定に誤りがあります: {}",
                    errs.join(" / ")
                ));
            }
            s.config = config.clone();
            if s.phase == Phase::GameOver {
                s.final_results = Some(final_results(s));
            }
            Ok(vec![])
        }
        EndGame => {
            need_phase(s, &[Phase::InHand, Phase::BetweenHands], "終局")?;
            finish(s);
            Ok(vec![Overlay::GameOver])
        }
    }
}

fn check_dice(dice: &[(u8, u8)], twice: bool) -> Result<(), EngineError> {
    let max = if twice { 2 } else { 1 };
    if dice.len() > max {
        return invalid(format!("サイコロは{max}回分まで入力できます"));
    }
    if dice
        .iter()
        .any(|&(a, b)| !(1..=6).contains(&a) || !(1..=6).contains(&b))
    {
        return invalid("サイコロの目は1〜6です");
    }
    Ok(())
}

fn ceil100(v: i64) -> i32 {
    ((v + 99).div_euclid(100) * 100) as i32
}

fn settle_wins(s: &mut TableState, wins: &[WinInput]) -> Result<Vec<Overlay>, EngineError> {
    if wins.is_empty() {
        return invalid("アガリがありません");
    }
    let mut winners: Vec<SeatId> = wins.iter().map(|w| w.winner).collect();
    winners.sort();
    winners.dedup();
    if winners.len() != wins.len() {
        return invalid("同じ人のアガリが2件あります");
    }
    let tsumo = wins.iter().any(|w| w.is_tsumo());
    if tsumo && wins.len() > 1 {
        return invalid("ツモのアガリは1件だけです");
    }
    let loser = wins[0].loser;
    if wins.iter().any(|w| w.loser != loser) {
        return invalid("ダブロンの放銃者が一致しません");
    }
    if let Some(l) = loser {
        if wins.iter().any(|w| w.winner == l) {
            return invalid("放銃者とアガった人が同じです");
        }
    }
    for w in wins {
        if w.pao == Some(w.winner) {
            return invalid("包の責任者がアガった人と同じです");
        }
        if (w.riichi || w.double_riichi) && !s.seats[w.winner.index()].riichi {
            return invalid(format!(
                "{}は立直していません（先に立直を入力してください）",
                s.seats[w.winner.index()].player.name
            ));
        }
    }
    // 放銃者から近い順（上家取りの順）
    let mut ordered: Vec<&WinInput> = wins.iter().collect();
    if let Some(l) = loser {
        ordered.sort_by_key(|w| w.winner.distance_from(l));
    }
    if ordered.len() == 3 && s.config.abortive.sancha_hou {
        return settle_draw(s, &[], &DrawKind::Abortive(AbortiveKind::SanchaHou));
    }
    if ordered.len() > 1 && s.config.multi_ron == MultiRon::HeadBump {
        ordered.truncate(1);
    }

    let dealer = s.dealer();
    let hp = s.config.honba_points as i64;
    let honba = s.honba as i64;
    let kyotaku_before = s.kyotaku;
    let mut records = Vec::new();
    let mut total = [0i32; 4];
    for (i, w) in ordered.iter().enumerate() {
        let score = score_in_state(s, w)?;
        let mut d = [0i32; 4];
        let win = w.winner.index();
        let pao = w
            .pao
            .filter(|_| matches!(score.limit, Some(Limit::Yakuman(_))));
        if let Some(l) = w.loser {
            let pay = score.ron_points as i64;
            let bonus = hp * honba;
            if let Some(p) = pao.filter(|p| *p != l) {
                let half = ceil100(pay / 2);
                d[p.index()] -= half;
                d[l.index()] -= (pay as i32 - half) + bonus as i32;
            } else {
                d[l.index()] -= (pay + bonus) as i32;
            }
            d[win] += (pay + bonus) as i32;
        } else if let Some(p) = pao {
            let full = if score.is_dealer {
                score.tsumo_dealer_pays * 3
            } else {
                score.tsumo_dealer_pays + 2 * score.tsumo_child_pays
            };
            let bonus = (hp * honba) as i32;
            d[p.index()] -= full + bonus;
            d[win] += full + bonus;
        } else {
            for o in SeatId::ALL {
                if o == w.winner {
                    continue;
                }
                let base = if o == dealer || score.is_dealer {
                    score.tsumo_dealer_pays
                } else {
                    score.tsumo_child_pays
                };
                let pay = base + (hp / 3 * honba) as i32;
                d[o.index()] -= pay;
                d[win] += pay;
            }
        }
        if i == 0 {
            d[win] += kyotaku_before as i32 * 1_000;
        }
        for k in 0..4 {
            total[k] += d[k];
        }
        records.push(WinRecord {
            input: (*w).clone(),
            score,
            deltas: d,
        });
    }
    s.kyotaku = 0;
    for k in 0..4 {
        s.seats[k].score += total[k];
    }
    s.last_deltas = total;
    let winners: Vec<SeatId> = records.iter().map(|r| r.input.winner).collect();
    let renchan = winners.contains(&dealer);
    s.history.push(HandRecord {
        wind: s.round_wind,
        number: s.round_number,
        honba: s.honba,
        kyotaku_before,
        outcome: HandOutcome::Win(records),
        deltas: total,
        scores_after: s.scores(),
        dora_indicators: s.dora_indicators.clone(),
    });
    let mut ov = vec![
        Overlay::Win { winners, loser },
        Overlay::ScoreChange { deltas: total },
    ];
    if advance(s, renchan, false) {
        ov.push(Overlay::GameOver);
    }
    Ok(ov)
}

fn settle_draw(
    s: &mut TableState,
    tenpai: &[SeatId],
    kind: &DrawKind,
) -> Result<Vec<Overlay>, EngineError> {
    let dealer = s.dealer();
    let mut d = [0i32; 4];
    let mut tp: Vec<SeatId> = tenpai.to_vec();
    tp.sort();
    tp.dedup();
    let renchan;
    match kind {
        DrawKind::Exhaustive => {
            let n = tp.len();
            if (1..=3).contains(&n) {
                let pen = s.config.noten_penalty;
                for seat in SeatId::ALL {
                    if tp.contains(&seat) {
                        d[seat.index()] += pen / n as i32;
                    } else {
                        d[seat.index()] -= pen / (4 - n) as i32;
                    }
                }
            }
            renchan = if s.config.tenpai_renchan {
                tp.contains(&dealer)
            } else {
                false
            };
        }
        DrawKind::NagashiMangan(seats) => {
            if seats.is_empty() {
                return invalid("流し満貫の達成者を選んでください");
            }
            for &w in seats {
                for o in SeatId::ALL {
                    if o == w {
                        continue;
                    }
                    let pay = if w == dealer || o == dealer {
                        4_000
                    } else {
                        2_000
                    };
                    d[o.index()] -= pay;
                    d[w.index()] += pay;
                }
            }
            renchan = if s.config.tenpai_renchan {
                tp.contains(&dealer) || seats.contains(&dealer)
            } else {
                false
            };
        }
        DrawKind::Abortive(k) => {
            let a = s.config.abortive;
            let enabled = match k {
                AbortiveKind::KyuushuKyuuhai => a.kyuushu_kyuuhai,
                AbortiveKind::SuufonRenda => a.suufon_renda,
                AbortiveKind::SuukanSanra => a.suukan_sanra,
                AbortiveKind::SuuchaRiichi => a.suucha_riichi,
                AbortiveKind::SanchaHou => a.sancha_hou,
            };
            if !enabled {
                return invalid(format!("このルールでは{}はありません", k.name_ja()));
            }
            renchan = true;
        }
    }
    for k in 0..4 {
        s.seats[k].score += d[k];
    }
    s.last_deltas = d;
    s.history.push(HandRecord {
        wind: s.round_wind,
        number: s.round_number,
        honba: s.honba,
        kyotaku_before: s.kyotaku,
        outcome: HandOutcome::Draw {
            tenpai: tp,
            kind: kind.clone(),
        },
        deltas: d,
        scores_after: s.scores(),
        dora_indicators: s.dora_indicators.clone(),
    });
    let mut ov = vec![Overlay::Draw, Overlay::ScoreChange { deltas: d }];
    if advance(s, renchan, true) {
        ov.push(Overlay::GameOver);
    }
    Ok(ov)
}

/// 局を進める。終局したら true。
fn advance(s: &mut TableState, renchan: bool, was_draw: bool) -> bool {
    let cfg = &s.config;
    let top = s.rank_order()[0];
    let top_score = s.seats[top.index()].score;
    let end = if cfg.bust_ends && s.seats.iter().any(|x| x.score < 0) {
        true
    } else if s.extension {
        top_score >= cfg.return_points
            || (s.round_number == 4 && !renchan)
            || s.round_wind == Wind::North
    } else if s.is_all_last() {
        if renchan {
            cfg.all_last_dealer_stop && top == s.dealer() && unique_top(s)
        } else {
            !(cfg.west_extension && top_score < cfg.return_points)
        }
    } else {
        false
    };
    if end {
        finish(s);
        return true;
    }
    if renchan {
        s.honba += 1;
    } else {
        s.honba = if was_draw { s.honba + 1 } else { 0 };
        if s.round_number == 4 {
            s.round_number = 1;
            s.round_wind = s.round_wind.next();
            if s.round_wind.index() > s.config.last_regular_wind().index() {
                s.extension = true;
            }
        } else {
            s.round_number += 1;
        }
    }
    s.phase = Phase::BetweenHands;
    false
}

fn unique_top(s: &TableState) -> bool {
    let o = s.rank_order();
    s.seats[o[0].index()].score > s.seats[o[1].index()].score
}

fn finish(s: &mut TableState) {
    if s.config.leftover_kyotaku_to_top && s.kyotaku > 0 {
        let top = s.rank_order()[0];
        s.seats[top.index()].score += s.kyotaku as i32 * 1_000;
        s.kyotaku = 0;
    }
    s.phase = Phase::GameOver;
    s.final_results = Some(final_results(s));
}

/// 最終順位とポイント（ウマ・オカ込み）。
pub fn final_results(s: &TableState) -> Vec<FinalResult> {
    let cfg = &s.config;
    let order = s.rank_order();
    let mut bonus = cfg.uma.map(|u| u as f64);
    bonus[0] += cfg.oka_points() as f64;
    let mut out: Vec<FinalResult> = Vec::new();
    let mut i = 0;
    while i < 4 {
        let sc = s.seats[order[i].index()].score;
        let mut j = i + 1;
        if cfg.tie_break == TieBreak::Split {
            while j < 4 && s.seats[order[j].index()].score == sc {
                j += 1;
            }
        }
        let share = bonus[i..j].iter().sum::<f64>() / (j - i) as f64;
        for seat in &order[i..j] {
            let pts = (sc - cfg.return_points) as f64 / 1_000.0 + share / 1_000.0;
            out.push(FinalResult {
                seat: *seat,
                rank: i as u8 + 1,
                score: sc,
                points: (pts * 10.0).round() / 10.0,
            });
        }
        i = j;
    }
    out
}

//! 大会通算（個人戦、設計書 5.6）。半荘ごとの最終結果を貯め、選手別の通算順位を出す。CSV で書き出せる。
//!
//! 選手は名前で同一人物とみなす（所属が変わっても同じ名前なら同じ選手）。

use serde::{Deserialize, Serialize};

use crate::state::{FinalResult, Phase, Player, TableState};

/// 1半荘の記録。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameRecord {
    /// 重複・取り消しの判定に使うかぎ（操作ログのファイル名と半荘の番号）
    pub key: String,
    /// 終局した時刻（UNIX ミリ秒）
    pub finished_at: i64,
    /// 席A〜Dの選手
    pub players: [Player; 4],
    pub results: Vec<FinalResult>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tournament {
    pub name: String,
    pub games: Vec<GameRecord>,
}

/// 選手1人の通算成績。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standing {
    pub rank: u32,
    pub name: String,
    pub affiliation: Option<String>,
    pub games: u32,
    /// 通算ポイント（小数1桁）
    pub points: f64,
    /// 1〜4着の回数
    pub placements: [u32; 4],
    pub average_rank: f64,
    /// 最終持ち点の合計
    pub total_score: i64,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

impl Tournament {
    /// 卓の状態を見て、終局していれば記録する（同じかぎがあれば置き換える）。
    /// 終局していなければ、そのかぎの記録を消す（終局を Undo したとき）。変化があれば true。
    pub fn sync_game(&mut self, key: &str, state: &TableState, now: i64) -> bool {
        let pos = self.games.iter().position(|g| g.key == key);
        match (state.phase, &state.final_results, pos) {
            (Phase::GameOver, Some(fr), Some(i)) => {
                let players = std::array::from_fn(|k| state.seats[k].player.clone());
                if self.games[i].results != *fr || self.games[i].players != players {
                    self.games[i].results = fr.clone();
                    self.games[i].players = players;
                    true
                } else {
                    false
                }
            }
            (Phase::GameOver, Some(fr), None) => {
                self.games.push(GameRecord {
                    key: key.to_string(),
                    finished_at: now,
                    players: std::array::from_fn(|k| state.seats[k].player.clone()),
                    results: fr.clone(),
                });
                true
            }
            (_, _, Some(i)) => {
                self.games.remove(i);
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let n = self.games.len();
        self.games.retain(|g| g.key != key);
        n != self.games.len()
    }

    /// 通算順位（ポイントの高い順。同点は平均順位、次に名前）。
    pub fn standings(&self) -> Vec<Standing> {
        let mut map: Vec<Standing> = Vec::new();
        for g in &self.games {
            for r in &g.results {
                let p = &g.players[r.seat.index()];
                let i = match map.iter().position(|s| s.name == p.name) {
                    Some(i) => i,
                    None => {
                        map.push(Standing {
                            rank: 0,
                            name: p.name.clone(),
                            affiliation: None,
                            games: 0,
                            points: 0.0,
                            placements: [0; 4],
                            average_rank: 0.0,
                            total_score: 0,
                        });
                        map.len() - 1
                    }
                };
                let s = &mut map[i];
                if p.affiliation.is_some() {
                    s.affiliation = p.affiliation.clone();
                }
                s.games += 1;
                s.points = round1(s.points + r.points);
                s.placements[(r.rank.clamp(1, 4) - 1) as usize] += 1;
                s.total_score += r.score as i64;
            }
        }
        for s in &mut map {
            let sum: u32 = s
                .placements
                .iter()
                .enumerate()
                .map(|(i, n)| (i as u32 + 1) * n)
                .sum();
            s.average_rank = if s.games > 0 {
                (sum as f64 / s.games as f64 * 100.0).round() / 100.0
            } else {
                0.0
            };
        }
        map.sort_by(|a, b| {
            b.points
                .partial_cmp(&a.points)
                .unwrap()
                .then(a.average_rank.partial_cmp(&b.average_rank).unwrap())
                .then(a.name.cmp(&b.name))
        });
        let mut prev: Option<(f64, f64)> = None;
        let mut rank = 0;
        for (i, s) in map.iter_mut().enumerate() {
            let k = (s.points, s.average_rank);
            if prev != Some(k) {
                rank = i as u32 + 1;
                prev = Some(k);
            }
            s.rank = rank;
        }
        map
    }

    /// 通算順位の CSV（Excel で開けるよう UTF-8 BOM 付き）。
    pub fn standings_csv(&self) -> String {
        let mut out = String::from(
            "\u{feff}順位,選手,所属,半荘数,通算ポイント,1着,2着,3着,4着,平均順位,持ち点合計\n",
        );
        for s in self.standings() {
            out.push_str(&format!(
                "{},{},{},{},{:.1},{},{},{},{},{:.2},{}\n",
                s.rank,
                csv_field(&s.name),
                csv_field(s.affiliation.as_deref().unwrap_or("")),
                s.games,
                s.points,
                s.placements[0],
                s.placements[1],
                s.placements[2],
                s.placements[3],
                s.average_rank,
                s.total_score
            ));
        }
        out
    }

    /// 半荘ごとの結果の CSV。
    pub fn games_csv(&self) -> String {
        let mut out = String::from("\u{feff}半荘,終局時刻(ms),席,選手,所属,着順,持ち点,ポイント\n");
        for (n, g) in self.games.iter().enumerate() {
            for r in &g.results {
                let p = &g.players[r.seat.index()];
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{:.1}\n",
                    n + 1,
                    g.finished_at,
                    r.seat.label(),
                    csv_field(&p.name),
                    csv_field(p.affiliation.as_deref().unwrap_or("")),
                    r.rank,
                    r.score,
                    r.points
                ));
            }
        }
        out
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::apply;
    use crate::rules::GameConfig;
    use crate::state::Action;
    use crate::tile::SeatId;

    fn finished(names: [&str; 4], deltas: [i32; 4]) -> TableState {
        let mut s = TableState::default();
        let players = names.map(Player::new);
        apply(
            &mut s,
            &Action::StartGame {
                config: GameConfig::preset_standard(),
                players,
                chiicha: SeatId(0),
            },
        )
        .unwrap();
        apply(
            &mut s,
            &Action::ManualAdjust {
                deltas,
                reason: "テスト".into(),
            },
        )
        .unwrap();
        apply(&mut s, &Action::EndGame).unwrap();
        s
    }

    #[test]
    fn records_and_standings() {
        let mut t = Tournament::default();
        let g1 = finished(["A", "B", "C", "D"], [15_000, 5_000, -5_000, -15_000]);
        assert!(t.sync_game("log1#1", &g1, 1));
        assert!(!t.sync_game("log1#1", &g1, 2), "同じ記録は増えない");
        let g2 = finished(["B", "A", "E", "C"], [20_000, 0, -5_000, -15_000]);
        t.sync_game("log1#2", &g2, 3);
        let st = t.standings();
        assert_eq!(st.len(), 5);
        let a = st.iter().find(|s| s.name == "A").unwrap();
        assert_eq!(a.games, 2);
        assert_eq!(a.placements, [1, 1, 0, 0]);
        // ポイントの合計は0
        let total: f64 = st.iter().map(|s| s.points).sum();
        assert!(total.abs() < 0.05);
        assert_eq!(st[0].rank, 1);
        assert!(st.windows(2).all(|w| w[0].points >= w[1].points));
    }

    #[test]
    fn undo_of_game_end_removes_record() {
        let mut t = Tournament::default();
        let mut g = finished(["A", "B", "C", "D"], [1000, 0, 0, -1000]);
        t.sync_game("k", &g, 1);
        assert_eq!(t.games.len(), 1);
        g.phase = Phase::BetweenHands;
        g.final_results = None;
        assert!(t.sync_game("k", &g, 2));
        assert!(t.games.is_empty());
    }

    #[test]
    fn csv_escapes_and_has_header() {
        let mut t = Tournament::default();
        let g = finished(["山田, 太郎", "B", "C", "D"], [0, 0, 0, 0]);
        t.sync_game("k", &g, 1);
        let csv = t.standings_csv();
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"山田, 太郎\""));
        assert_eq!(csv.lines().count(), 5);
        assert_eq!(t.games_csv().lines().count(), 5);
    }
}

//! 機能の確認（レビュー）用のダミー対局データ。サーバーの `--dummy` と、クライアントの `--dummy-server` が使う。
//!
//! 普通の操作（[`Action`]）を並べたものなので、取り消し・やり直しや、ここからの続きの操作もそのまま試せる。
//! 中身: 東1局で南家がロン → 東2局でツモ → 東3局の途中（ドラ表示・山の残り・北家のリーチ）

use serde::{Deserialize, Serialize};

use crate::rules::GameConfig;
use crate::scoring::WinInput;
use crate::state::{Action, Player};
use crate::tile::{SeatId, parse_tiles};

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
    v.extend(std::iter::repeat_n(Action::WallDecrement, 28));
    v.push(Action::Riichi { seat: SeatId(3) });
    v
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
    }
}

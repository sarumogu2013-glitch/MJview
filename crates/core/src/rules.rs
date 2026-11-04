//! 大会ルール（設計書 5.3）。保存形式は JSON、編集は設定UIから行う。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameLength {
    /// 東風戦
    Tonpuu,
    /// 東南戦（半荘）
    Hanchan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MultiRon {
    /// 頭ハネ（放銃者から見て下家側に近い1人だけ）
    HeadBump,
    /// 全員のアガリを認める
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieBreak {
    /// 起家に近い順に上位
    SeatOrder,
    /// 同順位としてウマを等分
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoubleWindPairFu {
    Two,
    Four,
}

/// 途中流局の種類ごとの有無。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortiveDraws {
    pub kyuushu_kyuuhai: bool,
    pub suufon_renda: bool,
    pub suukan_sanra: bool,
    pub suucha_riichi: bool,
    /// 三家和（3人同時ロンで流局）
    pub sancha_hou: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameConfig {
    /// プリセット名
    pub name: String,
    /// 持ち点
    pub start_points: i32,
    /// 返し
    pub return_points: i32,
    /// ウマ（1位〜4位、点数単位。例: +20,000 / +10,000 / −10,000 / −20,000）
    pub uma: [i32; 4],
    /// オカ（返し − 持ち点）×4 をトップに与えるか
    pub oka: bool,
    /// 赤ドラの枚数（萬・筒・索の5に1枚ずつ、0〜3）
    pub red_fives: u8,
    /// 喰いタン
    pub kuitan: bool,
    /// 後付け
    pub atozuke: bool,
    pub length: GameLength,
    /// 西入（規定の局が終わってトップが返し未満なら延長）
    pub west_extension: bool,
    /// 箱下終了（誰かが0点未満で終局）
    pub bust_ends: bool,
    pub multi_ron: MultiRon,
    /// 切り上げ満貫（4翻30符・3翻60符を満貫に）
    pub kiriage_mangan: bool,
    /// 数え役満（13翻以上を役満に）
    pub kazoe_yakuman: bool,
    /// ダブル役満（四暗刻単騎・国士十三面・純正九蓮・大四喜を2倍）
    pub double_yakuman: bool,
    /// 複合役満（役満の重複を加算）
    pub multiple_yakuman: bool,
    /// 本場1本あたりの点数（ロンで放銃者が払う合計。ツモは1/3ずつ）
    pub honba_points: i32,
    /// 連風牌の雀頭の符
    pub double_wind_pair_fu: DoubleWindPairFu,
    pub abortive: AbortiveDraws,
    pub tie_break: TieBreak,
    /// 局開始時の残り牌数（ツモれる枚数）
    pub wall_start: u32,
    /// サイコロの2度振り
    pub dice_twice: bool,
    /// ノーテン罰符の合計
    pub noten_penalty: i32,
    /// 流局時、親がテンパイなら連荘（false ならノーテン・テンパイに関わらず親流れ）
    pub tenpai_renchan: bool,
    /// オーラスで親がトップのとき、親のアガリ・テンパイで終局する（アガリ止め）
    pub all_last_dealer_stop: bool,
    /// 終局時に残った供託をトップに与える
    pub leftover_kyotaku_to_top: bool,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self::preset_standard()
    }
}

impl GameConfig {
    /// 一般的な競技ルール（25,000持ち30,000返し、ウマ10-20、赤3、喰いタン・後付けあり）。
    pub fn preset_standard() -> Self {
        GameConfig {
            name: "標準（赤あり・ウマ10-20）".into(),
            start_points: 25_000,
            return_points: 30_000,
            uma: [20_000, 10_000, -10_000, -20_000],
            oka: true,
            red_fives: 3,
            kuitan: true,
            atozuke: true,
            length: GameLength::Hanchan,
            west_extension: false,
            bust_ends: true,
            multi_ron: MultiRon::All,
            kiriage_mangan: false,
            kazoe_yakuman: true,
            double_yakuman: true,
            multiple_yakuman: true,
            honba_points: 300,
            double_wind_pair_fu: DoubleWindPairFu::Four,
            abortive: AbortiveDraws {
                kyuushu_kyuuhai: true,
                suufon_renda: true,
                suukan_sanra: true,
                suucha_riichi: true,
                sancha_hou: false,
            },
            tie_break: TieBreak::SeatOrder,
            wall_start: 70,
            dice_twice: false,
            noten_penalty: 3_000,
            tenpai_renchan: true,
            all_last_dealer_stop: true,
            leftover_kyotaku_to_top: true,
        }
    }

    /// 赤なしの競技ルールの初期値（30,000持ち、オカなし、ウマ15-5、切り上げ満貫、途中流局なし、ノーテン親流れ）。
    pub fn preset_pro_like() -> Self {
        GameConfig {
            name: "競技（赤なし・ウマ5-15）".into(),
            start_points: 30_000,
            return_points: 30_000,
            uma: [15_000, 5_000, -5_000, -15_000],
            oka: false,
            red_fives: 0,
            kiriage_mangan: true,
            kazoe_yakuman: false,
            double_yakuman: false,
            multiple_yakuman: false,
            double_wind_pair_fu: DoubleWindPairFu::Two,
            abortive: AbortiveDraws {
                kyuushu_kyuuhai: false,
                suufon_renda: false,
                suukan_sanra: false,
                suucha_riichi: false,
                sancha_hou: false,
            },
            bust_ends: false,
            tenpai_renchan: false,
            all_last_dealer_stop: false,
            leftover_kyotaku_to_top: true,
            ..Self::preset_standard()
        }
    }

    /// 東風戦の初期値。
    pub fn preset_tonpuu() -> Self {
        GameConfig {
            name: "東風戦（標準）".into(),
            length: GameLength::Tonpuu,
            ..Self::preset_standard()
        }
    }

    pub fn presets() -> Vec<GameConfig> {
        vec![
            Self::preset_standard(),
            Self::preset_pro_like(),
            Self::preset_tonpuu(),
        ]
    }

    /// 入力値の検証（設定UIでその場に出すエラー）。
    pub fn validate(&self) -> Vec<String> {
        let mut e = Vec::new();
        if self.start_points <= 0 {
            e.push("持ち点は正の数にしてください".into());
        }
        if self.return_points < self.start_points {
            e.push("返しは持ち点以上にしてください".into());
        }
        if self.uma.iter().sum::<i32>() != 0 {
            e.push("ウマの合計は0にしてください".into());
        }
        if self.red_fives > 3 {
            e.push("赤ドラは0〜3枚です".into());
        }
        if self.honba_points < 0 || self.honba_points % 300 != 0 {
            e.push("本場の点数は300の倍数にしてください（ツモで3人に分けるため）".into());
        }
        if self.noten_penalty < 0 || self.noten_penalty % 3_000 != 0 {
            e.push("ノーテン罰符は3,000の倍数にしてください".into());
        }
        if self.wall_start == 0 || self.wall_start > 122 {
            e.push("局開始時の残り牌数は1〜122にしてください".into());
        }
        e
    }

    /// オカ（トップの加算、点数単位）。
    pub fn oka_points(&self) -> i32 {
        if self.oka {
            (self.return_points - self.start_points) * 4
        } else {
            0
        }
    }

    /// 規定の最終局の場風（東風戦=東、東南戦=南）。
    pub fn last_regular_wind(&self) -> crate::tile::Wind {
        match self.length {
            GameLength::Tonpuu => crate::tile::Wind::East,
            GameLength::Hanchan => crate::tile::Wind::South,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid() {
        for p in GameConfig::presets() {
            assert!(p.validate().is_empty(), "{}: {:?}", p.name, p.validate());
        }
    }

    #[test]
    fn partial_json_uses_defaults() {
        let c: GameConfig = serde_json::from_str(r#"{"name":"x","red_fives":0}"#).unwrap();
        assert_eq!(c.red_fives, 0);
        assert_eq!(c.start_points, 25_000);
    }
}

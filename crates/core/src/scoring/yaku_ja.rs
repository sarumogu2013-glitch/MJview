//! mahc の役名・符（英語）→ 日本語（設計書 5.4）。

use mahc::fu::Fu;
use mahc::yaku::Yaku;

/// 役名。mahc の役は列挙型なので、すべて対応表に載っている（新しい役が増えるとコンパイルエラーで気づける）。
pub fn yaku_ja(y: &Yaku) -> &'static str {
    match y {
        Yaku::Riichi => "立直",
        Yaku::DoubleRiichi => "ダブル立直",
        Yaku::Ippatsu => "一発",
        Yaku::MenzenTsumo => "門前清自摸和",
        Yaku::Pinfu => "平和",
        Yaku::Tanyao => "断幺九",
        Yaku::Iipeikou => "一盃口",
        Yaku::Yakuhai => "役牌",
        Yaku::Haitei => "海底摸月",
        Yaku::RinshanKaihou => "嶺上開花",
        Yaku::Chankan => "槍槓",
        Yaku::Toitoi => "対々和",
        Yaku::Ittsuu => "一気通貫",
        Yaku::SanshokuDoujun => "三色同順",
        Yaku::SanshokuDoukou => "三色同刻",
        Yaku::Chantaiyao => "混全帯幺九",
        Yaku::JunchanTaiyao => "純全帯幺九",
        Yaku::Sanankou => "三暗刻",
        Yaku::Sankantsu => "三槓子",
        Yaku::Honroutou => "混老頭",
        Yaku::Shousangen => "小三元",
        Yaku::Chiitoitsu => "七対子",
        Yaku::Honitsu => "混一色",
        Yaku::Ryanpeikou => "二盃口",
        Yaku::Chinitsu => "清一色",
        Yaku::KazoeYakuman => "数え役満",
        Yaku::KokushiMusou => "国士無双",
        Yaku::KokushiMusou13SidedWait => "国士無双十三面待ち",
        Yaku::Suuankou => "四暗刻",
        Yaku::SuuankouTankiWait => "四暗刻単騎",
        Yaku::Daisangen => "大三元",
        Yaku::Shousuushii => "小四喜",
        Yaku::Daisuushii => "大四喜",
        Yaku::Tsuuiisou => "字一色",
        Yaku::Chinroutou => "清老頭",
        Yaku::Ryuuiisou => "緑一色",
        Yaku::ChuurenPoutou => "九蓮宝燈",
        Yaku::ChuurenPoutou9SidedWait => "純正九蓮宝燈",
        Yaku::Suukantsu => "四槓子",
        Yaku::Tenhou => "天和",
        Yaku::Chiihou => "地和",
        Yaku::Daichiishin => "大七星",
    }
}

pub fn fu_ja(f: &Fu) -> &'static str {
    match f {
        Fu::BasePoints => "副底",
        Fu::BasePointsChitoi => "七対子",
        Fu::ClosedRon => "門前ロン",
        Fu::Tsumo => "ツモ",
        Fu::NonSimpleClosedTriplet => "暗刻（幺九牌）",
        Fu::SimpleClosedTriplet => "暗刻（中張牌）",
        Fu::NonSimpleOpenTriplet => "明刻（幺九牌）",
        Fu::SimpleOpenTriplet => "明刻（中張牌）",
        Fu::NonSimpleClosedKan => "暗槓（幺九牌）",
        Fu::SimpleClosedKan => "暗槓（中張牌）",
        Fu::NonSimpleOpenKan => "明槓（幺九牌）",
        Fu::SimpleOpenKan => "明槓（中張牌）",
        Fu::Toitsu => "役牌の雀頭",
        Fu::SingleWait => "待ち（嵌張・辺張・単騎）",
    }
}

//! 牌・風・席。
//!
//! 牌の文字表記は学習ツール（training/）と同じ: `1m`〜`9m`, `1p`〜`9p`, `1s`〜`9s`, `1z`〜`7z`（東南西北白發中）,
//! 赤5は `0m` `0p` `0s`。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// 牌1枚。`kind` は 0〜33（萬子 0..9, 筒子 9..18, 索子 18..27, 字牌 27..34）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    kind: u8,
    red: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Suit {
    Man,
    Pin,
    Sou,
    Honor,
}

impl Suit {
    pub fn char(self) -> char {
        match self {
            Suit::Man => 'm',
            Suit::Pin => 'p',
            Suit::Sou => 's',
            Suit::Honor => 'z',
        }
    }
}

const HONOR_JA: [&str; 7] = ["東", "南", "西", "北", "白", "發", "中"];
const NUM_JA: [&str; 9] = ["一", "二", "三", "四", "五", "六", "七", "八", "九"];

impl Tile {
    pub const KINDS: u8 = 34;
    /// 不明の牌（手入力で分からなかった牌。表記は `?`）。点数計算には使えない
    pub const UNKNOWN: Tile = Tile {
        kind: 34,
        red: false,
    };

    pub fn new(kind: u8) -> Self {
        assert!(kind < Self::KINDS, "牌の種類は0〜33");
        Tile { kind, red: false }
    }

    /// 赤5。`suit` は萬筒索のどれか。
    pub fn red_five(suit: Suit) -> Self {
        let base = match suit {
            Suit::Man => 0,
            Suit::Pin => 9,
            Suit::Sou => 18,
            Suit::Honor => panic!("字牌に赤はない"),
        };
        Tile {
            kind: base + 4,
            red: true,
        }
    }

    pub fn from_suit_number(suit: Suit, n: u8) -> Self {
        match suit {
            Suit::Man => Tile::new(n - 1),
            Suit::Pin => Tile::new(9 + n - 1),
            Suit::Sou => Tile::new(18 + n - 1),
            Suit::Honor => Tile::new(27 + n - 1),
        }
    }

    pub fn kind(self) -> u8 {
        self.kind
    }
    /// 不明の牌か
    pub fn is_unknown(self) -> bool {
        self.kind >= Self::KINDS
    }
    pub fn is_red(self) -> bool {
        self.red
    }
    /// 赤を外した牌。
    pub fn plain(self) -> Self {
        Tile {
            kind: self.kind,
            red: false,
        }
    }
    pub fn suit(self) -> Suit {
        match self.kind {
            0..=8 => Suit::Man,
            9..=17 => Suit::Pin,
            18..=26 => Suit::Sou,
            _ => Suit::Honor,
        }
    }
    /// 数牌は1〜9、字牌は1〜7。
    pub fn number(self) -> u8 {
        self.kind % 9 + 1
    }
    pub fn is_honor(self) -> bool {
        self.kind >= 27
    }
    pub fn is_terminal_or_honor(self) -> bool {
        self.is_honor() || self.number() == 1 || self.number() == 9
    }

    /// ドラ表示牌からドラを求める（9→1、北→東、中→白）。
    pub fn dora_from_indicator(self) -> Tile {
        let k = self.kind;
        let next = match k {
            0..=26 => (k / 9) * 9 + (k % 9 + 1) % 9,
            27..=30 => 27 + (k - 27 + 1) % 4,
            _ => 31 + (k - 31 + 1) % 3,
        };
        Tile::new(next)
    }

    /// 理牌の並び（萬→筒→索→字、赤5は5と同じ位置で通常の5の後）。
    pub fn sort_key(self) -> (u8, bool) {
        (self.kind, self.red)
    }

    /// 日本語の表示名（例: 一萬, 赤五筒, 東）。
    pub fn name_ja(self) -> String {
        if self.is_unknown() {
            return "不明".into();
        }
        if self.is_honor() {
            return HONOR_JA[(self.kind - 27) as usize].to_string();
        }
        let suit = match self.suit() {
            Suit::Man => "萬",
            Suit::Pin => "筒",
            Suit::Sou => "索",
            Suit::Honor => unreachable!(),
        };
        let red = if self.red { "赤" } else { "" };
        format!("{red}{}{suit}", NUM_JA[(self.number() - 1) as usize])
    }

    /// 34種すべて（赤なし）。
    pub fn all_kinds() -> impl Iterator<Item = Tile> {
        (0..Self::KINDS).map(Tile::new)
    }
}

impl fmt::Display for Tile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unknown() {
            return write!(f, "?");
        }
        let n = if self.red { 0 } else { self.number() };
        write!(f, "{n}{}", self.suit().char())
    }
}

impl fmt::Debug for Tile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("牌の表記が不正です: {0:?}")]
pub struct ParseTileError(pub String);

impl FromStr for Tile {
    type Err = ParseTileError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseTileError(s.to_string());
        if s.trim() == "?" {
            return Ok(Tile::UNKNOWN);
        }
        let b = s.trim().as_bytes();
        if b.len() != 2 || !b[0].is_ascii_digit() {
            return Err(err());
        }
        let n = b[0] - b'0';
        let suit = match b[1] {
            b'm' => Suit::Man,
            b'p' => Suit::Pin,
            b's' => Suit::Sou,
            b'z' => Suit::Honor,
            _ => return Err(err()),
        };
        match (suit, n) {
            (Suit::Honor, 1..=7) => Ok(Tile::from_suit_number(suit, n)),
            (Suit::Honor, _) => Err(err()),
            (_, 0) => Ok(Tile::red_five(suit)),
            (_, 1..=9) => Ok(Tile::from_suit_number(suit, n)),
            _ => Err(err()),
        }
    }
}

impl Serialize for Tile {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Tile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// `"123m 456p 11z"` のような連続表記を牌の列にする（テスト・入力補助用）。
pub fn parse_tiles(s: &str) -> Result<Vec<Tile>, ParseTileError> {
    let mut out = Vec::new();
    let mut digits: Vec<u8> = Vec::new();
    for c in s.chars() {
        match c {
            '0'..='9' => digits.push(c as u8),
            'm' | 'p' | 's' | 'z' => {
                if digits.is_empty() {
                    return Err(ParseTileError(s.to_string()));
                }
                for d in digits.drain(..) {
                    out.push(format!("{}{c}", d as char).parse()?);
                }
            }
            c if c.is_whitespace() => {}
            _ => return Err(ParseTileError(s.to_string())),
        }
    }
    if !digits.is_empty() {
        return Err(ParseTileError(s.to_string()));
    }
    Ok(out)
}

/// 風。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Wind {
    East,
    South,
    West,
    North,
}

impl Wind {
    pub const ALL: [Wind; 4] = [Wind::East, Wind::South, Wind::West, Wind::North];

    pub fn index(self) -> u8 {
        self as u8
    }
    pub fn from_index(i: u8) -> Wind {
        Wind::ALL[(i % 4) as usize]
    }
    pub fn next(self) -> Wind {
        Wind::from_index(self.index() + 1)
    }
    pub fn tile(self) -> Tile {
        Tile::new(27 + self.index())
    }
    pub fn name_ja(self) -> &'static str {
        ["東", "南", "西", "北"][self.index() as usize]
    }
}

/// 物理的な席（A〜D）。席の並びは卓の反時計回り（A の下家が B）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeatId(pub u8);

impl SeatId {
    pub const ALL: [SeatId; 4] = [SeatId(0), SeatId(1), SeatId(2), SeatId(3)];

    pub fn index(self) -> usize {
        self.0 as usize
    }
    /// 下家（次の手番）。
    pub fn next(self) -> SeatId {
        SeatId((self.0 + 1) % 4)
    }
    /// `from` から見て何人先か（0〜3）。
    pub fn distance_from(self, from: SeatId) -> u8 {
        (self.0 + 4 - from.0) % 4
    }
    pub fn label(self) -> char {
        (b'A' + self.0) as char
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        for s in ["1m", "9p", "0s", "5z", "7z", "0m"] {
            assert_eq!(s.parse::<Tile>().unwrap().to_string(), s);
        }
        assert!("8z".parse::<Tile>().is_err());
        assert!("0z".parse::<Tile>().is_err());
        let t = parse_tiles("123m 0p 77z").unwrap();
        assert_eq!(t.len(), 6);
        assert!(t[3].is_red());
        assert_eq!(t[3].plain(), "5p".parse().unwrap());
    }

    #[test]
    fn dora() {
        let d = |s: &str| s.parse::<Tile>().unwrap().dora_from_indicator().to_string();
        assert_eq!(d("9m"), "1m");
        assert_eq!(d("0p"), "6p");
        assert_eq!(d("4z"), "1z");
        assert_eq!(d("7z"), "5z");
        assert_eq!(d("5z"), "6z");
        assert_eq!(d("3z"), "4z");
    }

    #[test]
    fn names() {
        assert_eq!("0p".parse::<Tile>().unwrap().name_ja(), "赤五筒");
        assert_eq!("6z".parse::<Tile>().unwrap().name_ja(), "發");
    }

    #[test]
    fn serde_roundtrip() {
        let t: Vec<Tile> = parse_tiles("19m0s7z").unwrap();
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(j, r#"["1m","9m","0s","7z"]"#);
        assert_eq!(serde_json::from_str::<Vec<Tile>>(&j).unwrap(), t);
    }
}

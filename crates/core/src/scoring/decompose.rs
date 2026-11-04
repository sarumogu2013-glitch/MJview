//! 面子分解（設計書 4.5「面子分解は本ソフト側で行ってから mahc に渡す」）。
//!
//! 門前部分（副露を除いた手牌、アガリ牌を含む）を、雀頭＋面子・七対子・国士無双のすべての形に分解する。

use crate::tile::Tile;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupKind {
    Sequence,
    Triplet,
    Pair,
    /// 国士無双の単独の牌
    Single,
}

/// 門前部分の1組（牌は赤を外した種類で持つ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Group {
    pub kind: GroupKind,
    /// 順子は先頭の牌、それ以外はその牌
    pub first: u8,
}

impl Group {
    pub fn kinds(&self) -> Vec<u8> {
        match self.kind {
            GroupKind::Sequence => vec![self.first, self.first + 1, self.first + 2],
            GroupKind::Triplet => vec![self.first; 3],
            GroupKind::Pair => vec![self.first; 2],
            GroupKind::Single => vec![self.first],
        }
    }
    pub fn contains(&self, kind: u8) -> bool {
        self.kinds().contains(&kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Standard(Vec<Group>),
    Chiitoitsu(Vec<Group>),
    Kokushi(Vec<Group>),
}

impl Shape {
    pub fn groups(&self) -> &[Group] {
        match self {
            Shape::Standard(g) | Shape::Chiitoitsu(g) | Shape::Kokushi(g) => g,
        }
    }
}

pub fn counts(tiles: &[Tile]) -> [u8; 34] {
    let mut c = [0u8; 34];
    for t in tiles {
        c[t.kind() as usize] += 1;
    }
    c
}

/// 門前部分 `tiles`（3n+2 枚）のアガリ形をすべて返す。`n_melds` は副露の数（七対子・国士は副露なしのときだけ）。
pub fn decompose(tiles: &[Tile], n_melds: usize) -> Vec<Shape> {
    let c = counts(tiles);
    let mut out = Vec::new();
    if tiles.len() % 3 != 2 {
        return out;
    }
    let need = (tiles.len() - 2) / 3;
    for p in 0..34 {
        if c[p] >= 2 {
            let mut cc = c;
            cc[p] -= 2;
            let mut acc = vec![Group {
                kind: GroupKind::Pair,
                first: p as u8,
            }];
            mentsu(&mut cc, 0, &mut acc, need, &mut out);
        }
    }
    if n_melds == 0 && tiles.len() == 14 {
        if c.iter().filter(|&&n| n == 2).count() == 7 {
            out.push(Shape::Chiitoitsu(
                (0..34)
                    .filter(|&k| c[k] == 2)
                    .map(|k| Group {
                        kind: GroupKind::Pair,
                        first: k as u8,
                    })
                    .collect(),
            ));
        }
        const ORPHANS: [usize; 13] = [0, 8, 9, 17, 18, 26, 27, 28, 29, 30, 31, 32, 33];
        if ORPHANS.iter().all(|&k| c[k] >= 1)
            && ORPHANS.iter().map(|&k| c[k] as usize).sum::<usize>() == 14
        {
            out.push(Shape::Kokushi(
                ORPHANS
                    .iter()
                    .map(|&k| Group {
                        kind: if c[k] == 2 {
                            GroupKind::Pair
                        } else {
                            GroupKind::Single
                        },
                        first: k as u8,
                    })
                    .collect(),
            ));
        }
    }
    out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    out.dedup();
    out
}

fn mentsu(c: &mut [u8; 34], from: usize, acc: &mut Vec<Group>, need: usize, out: &mut Vec<Shape>) {
    let Some(k) = (from..34).find(|&k| c[k] > 0) else {
        if acc.len() == need + 1 {
            let mut g = acc.clone();
            g.sort();
            out.push(Shape::Standard(g));
        }
        return;
    };
    if c[k] >= 3 {
        c[k] -= 3;
        acc.push(Group {
            kind: GroupKind::Triplet,
            first: k as u8,
        });
        mentsu(c, k, acc, need, out);
        acc.pop();
        c[k] += 3;
    }
    if k < 27 && k % 9 <= 6 && c[k + 1] > 0 && c[k + 2] > 0 {
        c[k] -= 1;
        c[k + 1] -= 1;
        c[k + 2] -= 1;
        acc.push(Group {
            kind: GroupKind::Sequence,
            first: k as u8,
        });
        mentsu(c, k, acc, need, out);
        acc.pop();
        c[k] += 1;
        c[k + 1] += 1;
        c[k + 2] += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::parse_tiles;

    #[test]
    fn standard() {
        let t = parse_tiles("111222333m456p77z").unwrap();
        let s = decompose(&t, 0);
        // 111 222 333 と 123 123 123 の2通り
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn chiitoi_and_ryanpeikou() {
        let t = parse_tiles("112233m112233p55z").unwrap();
        let s = decompose(&t, 0);
        assert!(s.iter().any(|x| matches!(x, Shape::Chiitoitsu(_))));
        assert!(s.iter().any(|x| matches!(x, Shape::Standard(_))));
    }

    #[test]
    fn kokushi() {
        let t = parse_tiles("19m19p19s12345677z").unwrap();
        let s = decompose(&t, 0);
        assert_eq!(s.len(), 1);
        assert!(matches!(s[0], Shape::Kokushi(_)));
    }

    #[test]
    fn not_winning() {
        let t = parse_tiles("123456789m1235p").unwrap();
        assert!(decompose(&t, 0).is_empty());
    }

    #[test]
    fn with_melds() {
        let t = parse_tiles("234m55p").unwrap();
        assert_eq!(decompose(&t, 3).len(), 1);
    }
}

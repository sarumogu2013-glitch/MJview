//! 出力ウィンドウのシーン切替（設計書 4.3）。プレビュー（PVW）で選び、「切替」で放送（PGM）へ。
//! トランジションは カット／フェード（黒を経由）／ワイプ。時刻は秒の f64。

use serde::{Deserialize, Serialize};

use crate::tile::SeatId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneKind {
    /// 対局画面（4.2）
    Game,
    /// 顔カメラを全画面
    Fullscreen(SeatId),
    /// 選手紹介
    PlayerIntro,
    /// 待機・休憩
    Waiting,
    /// 半荘結果
    FinalResult,
    /// 大会の通算順位
    Standings,
}

impl SceneKind {
    pub fn name_ja(self) -> String {
        match self {
            SceneKind::Game => "対局".into(),
            SceneKind::Fullscreen(s) => format!("全画面映像（席{}）", s.label()),
            SceneKind::PlayerIntro => "選手紹介".into(),
            SceneKind::Waiting => "待機・休憩".into(),
            SceneKind::FinalResult => "半荘結果".into(),
            SceneKind::Standings => "通算順位".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    Cut,
    Fade,
    Wipe,
}

impl TransitionKind {
    pub fn name_ja(self) -> &'static str {
        match self {
            TransitionKind::Cut => "カット",
            TransitionKind::Fade => "フェード",
            TransitionKind::Wipe => "ワイプ",
        }
    }
}

/// 画面全体に重ねる幕（トランジション中の見た目）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curtain {
    None,
    /// 黒の不透明度 0〜1
    Fade(f32),
    /// 画面の横幅に対する割合で、黒い帯の左端と右端（0〜1）
    Wipe {
        left: f32,
        right: f32,
    },
}

#[derive(Clone, Debug)]
pub struct SceneSwitcher {
    pub program: SceneKind,
    pub preview: SceneKind,
    pub transition: TransitionKind,
    /// トランジションの時間（秒）
    pub duration: f64,
    active: Option<(SceneKind, TransitionKind, f64)>,
}

impl Default for SceneSwitcher {
    fn default() -> Self {
        SceneSwitcher {
            program: SceneKind::Game,
            preview: SceneKind::Game,
            transition: TransitionKind::Fade,
            duration: 0.8,
            active: None,
        }
    }
}

impl SceneSwitcher {
    /// プレビューのシーンを放送に出す。トランジション中なら何もしない。
    pub fn take(&mut self, now: f64) -> bool {
        if self.active.is_some() || self.preview == self.program {
            return false;
        }
        if self.transition == TransitionKind::Cut || self.duration <= 0.0 {
            self.program = self.preview;
        } else {
            self.active = Some((self.preview, self.transition, now));
        }
        true
    }

    pub fn in_transition(&self) -> bool {
        self.active.is_some()
    }

    /// 毎フレーム呼ぶ。途中で放送のシーンが入れ替わる。今の幕を返す。
    pub fn update(&mut self, now: f64) -> Curtain {
        let Some((target, kind, start)) = self.active else {
            return Curtain::None;
        };
        let p = ((now - start) / self.duration).clamp(0.0, 1.0) as f32;
        if p >= 0.5 {
            self.program = target;
        }
        if p >= 1.0 {
            self.active = None;
            return Curtain::None;
        }
        match kind {
            TransitionKind::Cut => Curtain::None,
            // 前半で黒くなり、後半で明ける
            TransitionKind::Fade => Curtain::Fade(1.0 - (2.0 * p - 1.0).abs()),
            // 前半で左から覆い、後半で左から開ける
            TransitionKind::Wipe => {
                if p < 0.5 {
                    Curtain::Wipe {
                        left: 0.0,
                        right: 2.0 * p,
                    }
                } else {
                    Curtain::Wipe {
                        left: 2.0 * p - 1.0,
                        right: 1.0,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_is_immediate() {
        let mut s = SceneSwitcher {
            transition: TransitionKind::Cut,
            ..Default::default()
        };
        s.preview = SceneKind::Waiting;
        assert!(s.take(0.0));
        assert_eq!(s.program, SceneKind::Waiting);
        assert_eq!(s.update(0.0), Curtain::None);
    }

    #[test]
    fn fade_switches_at_midpoint() {
        let mut s = SceneSwitcher {
            transition: TransitionKind::Fade,
            duration: 1.0,
            ..Default::default()
        };
        s.preview = SceneKind::PlayerIntro;
        assert!(s.take(10.0));
        assert_eq!(s.update(10.25), Curtain::Fade(0.5));
        assert_eq!(s.program, SceneKind::Game);
        assert_eq!(s.update(10.5), Curtain::Fade(1.0));
        assert_eq!(s.program, SceneKind::PlayerIntro);
        assert!(matches!(s.update(10.75), Curtain::Fade(a) if (a - 0.5).abs() < 1e-6));
        assert_eq!(s.update(11.0), Curtain::None);
        assert!(!s.in_transition());
    }

    #[test]
    fn wipe_covers_then_uncovers() {
        let mut s = SceneSwitcher {
            transition: TransitionKind::Wipe,
            duration: 2.0,
            ..Default::default()
        };
        s.preview = SceneKind::FinalResult;
        s.take(0.0);
        assert_eq!(
            s.update(0.5),
            Curtain::Wipe {
                left: 0.0,
                right: 0.5
            }
        );
        assert_eq!(
            s.update(1.5),
            Curtain::Wipe {
                left: 0.5,
                right: 1.0
            }
        );
        assert_eq!(s.program, SceneKind::FinalResult);
    }

    #[test]
    fn take_ignored_during_transition_or_same_scene() {
        let mut s = SceneSwitcher::default();
        assert!(!s.take(0.0), "同じシーンは切り替えない");
        s.preview = SceneKind::Waiting;
        assert!(s.take(0.0));
        s.preview = SceneKind::PlayerIntro;
        assert!(!s.take(0.1), "トランジション中は受け付けない");
    }
}

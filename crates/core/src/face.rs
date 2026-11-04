//! 選手画面の切替（設計書 4.7）。描画とは独立した純粋なロジック（時刻は秒の f64 で受け取る）。
//!
//! - モード: 自動 ／ 一時手動（設定秒数後に自動へ） ／ 手動固定
//! - 自動のきっかけ（上ほど優先）: アガリ → 放銃 → リーチ → 槓 → それ以外は巡回（または親を表示）
//! - 最短表示時間: 切替直後は、より優先度の高いきっかけ以外では切り替えない
//! - 顔カメラが信号なしの席は自動切替の対象から外す

use serde::{Deserialize, Serialize};

use crate::tile::SeatId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerKind {
    Win,
    DealIn,
    Riichi,
    Kan,
}

impl TriggerKind {
    pub fn name_ja(self) -> &'static str {
        match self {
            TriggerKind::Win => "アガリ",
            TriggerKind::DealIn => "放銃",
            TriggerKind::Riichi => "リーチ",
            TriggerKind::Kan => "槓",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerSetting {
    pub kind: TriggerKind,
    pub enabled: bool,
    /// 表示秒数（アガリはアガった人1人あたり）
    pub secs: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManualBehavior {
    /// 一時手動（temp_manual_secs 後に自動へ戻る）
    Temporary,
    /// 手動固定（「自動」を押すまで）
    Locked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdleMode {
    /// 席順に一定間隔で巡回
    Rotate,
    /// 親を表示
    Dealer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaceSwitchConfig {
    pub manual_behavior: ManualBehavior,
    pub temp_manual_secs: f64,
    /// 優先順位の順（先頭が最優先）
    pub triggers: Vec<TriggerSetting>,
    pub idle: IdleMode,
    pub rotate_secs: f64,
    pub min_display_secs: f64,
    /// 切替効果: true ならフェード
    pub fade: bool,
}

impl Default for FaceSwitchConfig {
    fn default() -> Self {
        FaceSwitchConfig {
            manual_behavior: ManualBehavior::Temporary,
            temp_manual_secs: 30.0,
            triggers: vec![
                TriggerSetting {
                    kind: TriggerKind::Win,
                    enabled: true,
                    secs: 10.0,
                },
                TriggerSetting {
                    kind: TriggerKind::DealIn,
                    enabled: false,
                    secs: 5.0,
                },
                TriggerSetting {
                    kind: TriggerKind::Riichi,
                    enabled: true,
                    secs: 10.0,
                },
                TriggerSetting {
                    kind: TriggerKind::Kan,
                    enabled: false,
                    secs: 5.0,
                },
            ],
            idle: IdleMode::Rotate,
            rotate_secs: 15.0,
            min_display_secs: 5.0,
            fade: false,
        }
    }
}

impl FaceSwitchConfig {
    /// 優先度（小さいほど優先）。無効なきっかけは None。
    fn priority(&self, kind: TriggerKind) -> Option<(usize, f64)> {
        self.triggers
            .iter()
            .position(|t| t.kind == kind && t.enabled)
            .map(|i| (i, self.triggers[i].secs))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FaceMode {
    Auto,
    TempManual { seat: SeatId, until: f64 },
    LockedManual { seat: SeatId },
}

/// いま映している理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaceReason {
    Trigger(TriggerKind),
    Rotate,
    Dealer,
    Manual,
}

#[derive(Clone, Debug, PartialEq)]
struct Active {
    kind: TriggerKind,
    prio: usize,
    /// 順に映す席（ダブロンは演出の順）
    seats: Vec<SeatId>,
    secs: f64,
    started: f64,
}

impl Active {
    fn seat_at(&self, now: f64) -> Option<SeatId> {
        let i = ((now - self.started) / self.secs).floor();
        if i < 0.0 {
            return self.seats.first().copied();
        }
        self.seats.get(i as usize).copied()
    }
    fn until(&self) -> f64 {
        self.started + self.secs * self.seats.len() as f64
    }
}

/// 卓の状況（毎回の判定に渡す）。
#[derive(Clone, Copy, Debug)]
pub struct FaceContext {
    pub dealer: SeatId,
    /// 顔カメラの信号があるか（黒枠モードでは全席 true）
    pub has_signal: [bool; 4],
}

#[derive(Clone, Debug)]
pub struct FaceSwitcher {
    pub config: FaceSwitchConfig,
    mode: FaceMode,
    current: SeatId,
    reason: FaceReason,
    shown_since: f64,
    active: Option<Active>,
    /// 巡回の次の切替時刻
    next_rotate: f64,
}

impl FaceSwitcher {
    pub fn new(config: FaceSwitchConfig, first: SeatId) -> Self {
        FaceSwitcher {
            config,
            mode: FaceMode::Auto,
            current: first,
            reason: FaceReason::Dealer,
            shown_since: f64::NEG_INFINITY,
            active: None,
            next_rotate: 0.0,
        }
    }

    pub fn mode(&self) -> FaceMode {
        self.mode
    }
    pub fn current(&self) -> SeatId {
        self.current
    }
    pub fn reason(&self) -> FaceReason {
        self.reason
    }

    /// 次に自動で切り替わるまでの秒数（表示用）。手動固定なら None。
    pub fn remaining(&self, now: f64) -> Option<f64> {
        match self.mode {
            FaceMode::LockedManual { .. } => None,
            FaceMode::TempManual { until, .. } => Some((until - now).max(0.0)),
            FaceMode::Auto => match &self.active {
                Some(a) => Some((a.until() - now).max(0.0)),
                None if self.config.idle == IdleMode::Rotate => {
                    Some((self.next_rotate - now).max(0.0))
                }
                None => None,
            },
        }
    }

    /// 手動で選ぶ（F1〜F4・クリック）。
    pub fn manual(&mut self, seat: SeatId, now: f64) {
        self.mode = match self.config.manual_behavior {
            ManualBehavior::Temporary => FaceMode::TempManual {
                seat,
                until: now + self.config.temp_manual_secs,
            },
            ManualBehavior::Locked => FaceMode::LockedManual { seat },
        };
        self.show(seat, FaceReason::Manual, now);
    }

    /// 自動に戻す。
    pub fn auto(&mut self, now: f64) {
        self.mode = FaceMode::Auto;
        self.next_rotate = now;
    }

    /// きっかけ（アガリ・放銃・リーチ・槓）。seats は順に映す席。
    pub fn trigger(&mut self, kind: TriggerKind, seats: Vec<SeatId>, now: f64) {
        let Some((prio, secs)) = self.config.priority(kind) else {
            return;
        };
        if seats.is_empty() {
            return;
        }
        let replace = match &self.active {
            Some(a) if now < a.until() => prio <= a.prio,
            _ => true,
        };
        if replace {
            self.active = Some(Active {
                kind,
                prio,
                seats,
                secs,
                started: now,
            });
        }
    }

    /// 局の切れ目などで、進行中のきっかけを消す。
    pub fn clear_triggers(&mut self) {
        self.active = None;
    }

    fn show(&mut self, seat: SeatId, reason: FaceReason, now: f64) {
        if seat != self.current {
            self.shown_since = now;
        }
        self.current = seat;
        self.reason = reason;
    }

    fn reason_prio(&self, r: FaceReason) -> usize {
        match r {
            FaceReason::Trigger(k) => self
                .config
                .priority(k)
                .map(|p| p.0)
                .unwrap_or(usize::MAX - 1),
            FaceReason::Manual => usize::MAX - 1,
            FaceReason::Rotate | FaceReason::Dealer => usize::MAX,
        }
    }

    /// 毎フレーム呼ぶ。映す席を返す。
    pub fn update(&mut self, now: f64, ctx: &FaceContext) -> SeatId {
        // 手動
        match self.mode {
            FaceMode::LockedManual { seat } => {
                self.show(seat, FaceReason::Manual, now);
                return seat;
            }
            FaceMode::TempManual { seat, until } if now < until => {
                self.show(seat, FaceReason::Manual, now);
                return seat;
            }
            FaceMode::TempManual { .. } => {
                self.mode = FaceMode::Auto;
                self.next_rotate = now + self.config.rotate_secs;
            }
            FaceMode::Auto => {}
        }
        let can_switch = |me: &Self, reason: FaceReason| {
            now - me.shown_since >= me.config.min_display_secs
                || me.reason_prio(reason) < me.reason_prio(me.reason)
        };
        // きっかけ
        if let Some(a) = self.active.clone() {
            if now >= a.until() {
                // きっかけの表示が終わったら、映している席のまま通常（巡回・親）に戻る
                self.active = None;
                self.next_rotate = now + self.config.rotate_secs;
                self.reason = match self.config.idle {
                    IdleMode::Rotate => FaceReason::Rotate,
                    IdleMode::Dealer => FaceReason::Dealer,
                };
            } else if let Some(seat) = a.seat_at(now).filter(|s| ctx.has_signal[s.index()]) {
                let reason = FaceReason::Trigger(a.kind);
                if seat == self.current || can_switch(self, reason) {
                    self.show(seat, reason, now);
                }
                return self.current;
            }
        }
        // それ以外: 巡回 または 親
        match self.config.idle {
            IdleMode::Dealer => {
                let seat = ctx.dealer;
                if ctx.has_signal[seat.index()]
                    && (seat == self.current || can_switch(self, FaceReason::Dealer))
                {
                    self.show(seat, FaceReason::Dealer, now);
                } else if self.reason != FaceReason::Dealer {
                    self.reason = FaceReason::Dealer;
                }
            }
            IdleMode::Rotate => {
                if now >= self.next_rotate && can_switch(self, FaceReason::Rotate) {
                    let next = if matches!(self.reason, FaceReason::Rotate) {
                        self.current.next()
                    } else {
                        self.current
                    };
                    if let Some(seat) = next_with_signal(next, ctx) {
                        self.show(seat, FaceReason::Rotate, now);
                    }
                    self.reason = FaceReason::Rotate;
                    self.next_rotate = now + self.config.rotate_secs;
                }
            }
        }
        // 映している席の信号が切れたら、信号のある席へ
        if !ctx.has_signal[self.current.index()] {
            if let Some(seat) = next_with_signal(self.current.next(), ctx) {
                self.show(seat, self.reason, now);
            }
        }
        self.current
    }
}

fn next_with_signal(from: SeatId, ctx: &FaceContext) -> Option<SeatId> {
    (0..4)
        .map(|k| SeatId((from.0 + k) % 4))
        .find(|s| ctx.has_signal[s.index()])
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: SeatId = SeatId(0);
    const B: SeatId = SeatId(1);
    const C: SeatId = SeatId(2);
    const D: SeatId = SeatId(3);

    fn ctx() -> FaceContext {
        FaceContext {
            dealer: A,
            has_signal: [true; 4],
        }
    }

    fn run(f: &mut FaceSwitcher, from: f64, to: f64) -> Vec<(f64, SeatId)> {
        let mut out = vec![];
        let mut t = from;
        while t <= to + 1e-9 {
            out.push((t, f.update(t, &ctx())));
            t += 0.5;
        }
        out
    }

    #[test]
    fn rotation_in_seat_order() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        assert_eq!(f.current(), A);
        assert_eq!(f.update(15.0, &ctx()), B);
        assert_eq!(f.update(29.9, &ctx()), B);
        assert_eq!(f.update(30.0, &ctx()), C);
    }

    #[test]
    fn riichi_then_back_to_rotation() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        f.trigger(TriggerKind::Riichi, vec![C], 6.0);
        assert_eq!(f.update(6.0, &ctx()), C);
        assert_eq!(f.reason(), FaceReason::Trigger(TriggerKind::Riichi));
        assert_eq!(f.update(15.9, &ctx()), C);
        // 10秒後に巡回へ戻る
        f.update(16.0, &ctx());
        assert_eq!(f.reason(), FaceReason::Rotate);
    }

    #[test]
    fn min_display_blocks_lower_priority() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        f.trigger(TriggerKind::Riichi, vec![B], 1.0);
        assert_eq!(f.update(1.0, &ctx()), B);
        // 同じ優先度のリーチが2秒後に来ても、最短表示時間（5秒）の間は切り替えない
        f.trigger(TriggerKind::Riichi, vec![C], 3.0);
        assert_eq!(f.update(3.0, &ctx()), B);
        assert_eq!(f.update(6.0, &ctx()), C);
        // より優先度の高いアガリはすぐ切り替える
        f.trigger(TriggerKind::Win, vec![D], 7.0);
        assert_eq!(f.update(7.0, &ctx()), D);
    }

    #[test]
    fn double_ron_in_order() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        f.trigger(TriggerKind::Win, vec![C, B], 10.0);
        assert_eq!(f.update(10.0, &ctx()), C);
        assert_eq!(f.update(19.0, &ctx()), C);
        assert_eq!(f.update(20.0, &ctx()), B);
        assert_eq!(f.update(29.5, &ctx()), B);
    }

    #[test]
    fn lower_trigger_does_not_replace_higher() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.trigger(TriggerKind::Win, vec![B], 0.0);
        f.trigger(TriggerKind::Riichi, vec![C], 1.0);
        assert_eq!(f.update(1.0, &ctx()), B);
    }

    #[test]
    fn disabled_trigger_is_ignored() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        f.trigger(TriggerKind::Kan, vec![D], 8.0); // 既定OFF
        assert_eq!(f.update(8.0, &ctx()), A);
    }

    #[test]
    fn temp_manual_returns_to_auto() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        f.update(0.0, &ctx());
        f.manual(D, 1.0);
        assert!(matches!(f.mode(), FaceMode::TempManual { seat: D, .. }));
        // 手動中はきっかけが来ても手動のまま
        f.trigger(TriggerKind::Riichi, vec![B], 2.0);
        assert_eq!(f.update(2.0, &ctx()), D);
        assert_eq!(f.update(30.9, &ctx()), D);
        f.update(31.0, &ctx());
        assert_eq!(f.mode(), FaceMode::Auto);
    }

    #[test]
    fn locked_manual_until_auto() {
        let cfg = FaceSwitchConfig {
            manual_behavior: ManualBehavior::Locked,
            ..Default::default()
        };
        let mut f = FaceSwitcher::new(cfg, A);
        f.manual(C, 0.0);
        let seen = run(&mut f, 0.0, 120.0);
        assert!(seen.iter().all(|(_, s)| *s == C));
        f.auto(121.0);
        assert_eq!(f.mode(), FaceMode::Auto);
    }

    #[test]
    fn no_signal_seats_are_skipped() {
        let mut f = FaceSwitcher::new(FaceSwitchConfig::default(), A);
        let c = FaceContext {
            dealer: A,
            has_signal: [true, false, true, true],
        };
        f.update(0.0, &c);
        assert_eq!(f.update(15.0, &c), C, "B は信号なしなので飛ばす");
        // 信号なしの席のリーチは映さない
        f.trigger(TriggerKind::Riichi, vec![B], 20.0);
        assert_ne!(f.update(21.0, &c), B);
    }

    #[test]
    fn dealer_mode() {
        let cfg = FaceSwitchConfig {
            idle: IdleMode::Dealer,
            ..Default::default()
        };
        let mut f = FaceSwitcher::new(cfg, B);
        let c = FaceContext {
            dealer: C,
            has_signal: [true; 4],
        };
        assert_eq!(f.update(0.0, &c), C);
        assert_eq!(f.update(100.0, &c), C);
    }
}

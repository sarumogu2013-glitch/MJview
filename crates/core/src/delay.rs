//! 配信ディレイ（設計書 4.11）。届いたものに時刻を付けて溜め、「今 − ディレイ秒」を過ぎたものから出す。
//! データ（卓状態・演出の出来事）と映像（顔カメラの JPEG）の両方で使う。時刻は手元の単調時計（Instant）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 時刻付きの待ち行列。
#[derive(Debug, Clone)]
pub struct DelayLine<T> {
    q: VecDeque<(Instant, T)>,
}

impl<T> Default for DelayLine<T> {
    fn default() -> Self {
        DelayLine { q: VecDeque::new() }
    }
}

impl<T> DelayLine<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 届いた時刻 `at` で積む（時刻は増える順に積むこと）。
    pub fn push(&mut self, at: Instant, item: T) {
        self.q.push_back((at, item));
    }

    /// `now − delay` までに届いたものを、届いた順にすべて取り出す。
    pub fn drain_due(&mut self, now: Instant, delay: Duration) -> Vec<T> {
        let cutoff = now.checked_sub(delay);
        let mut out = Vec::new();
        while let Some((at, _)) = self.q.front() {
            match cutoff {
                Some(c) if *at <= c => out.push(self.q.pop_front().unwrap().1),
                _ => break,
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// まだ出していないものの中で、いちばん古いものが届いてからの時間。
    pub fn oldest_age(&self, now: Instant) -> Option<Duration> {
        self.q
            .front()
            .map(|(at, _)| now.saturating_duration_since(*at))
    }
}

/// 映像用: 最新から一定時間ぶんだけ残し、「ある時刻にいちばん近い過去の1枚」を取り出せる。
#[derive(Debug, Clone)]
pub struct FrameRing<T> {
    q: VecDeque<(Instant, T)>,
    /// これより古いものは捨てる
    pub retain: Duration,
}

impl<T: Clone> FrameRing<T> {
    pub fn new(retain: Duration) -> Self {
        FrameRing {
            q: VecDeque::new(),
            retain,
        }
    }

    pub fn push(&mut self, at: Instant, item: T) {
        self.q.push_back((at, item));
        while let Some((t, _)) = self.q.front() {
            if at.saturating_duration_since(*t) > self.retain && self.q.len() > 1 {
                self.q.pop_front();
            } else {
                break;
            }
        }
    }

    /// `target` 以前でいちばん新しい1枚と、その時刻。`target` より前のものがなければ None。
    pub fn at_or_before(&self, target: Instant) -> Option<(Instant, T)> {
        // 新しい方から探す（ディレイ0なら最初の1つで見つかる）
        self.q
            .iter()
            .rev()
            .find(|(t, _)| *t <= target)
            .map(|(t, v)| (*t, v.clone()))
    }

    pub fn latest(&self) -> Option<(Instant, T)> {
        self.q.back().map(|(t, v)| (*t, v.clone()))
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_line_releases_in_order() {
        let t0 = Instant::now();
        let s = |x: u64| t0 + Duration::from_millis(x);
        let mut d = DelayLine::new();
        d.push(s(0), "a");
        d.push(s(100), "b");
        d.push(s(200), "c");
        let delay = Duration::from_millis(1000);
        assert!(d.drain_due(s(900), delay).is_empty());
        assert_eq!(d.drain_due(s(1100), delay), vec!["a", "b"]);
        assert_eq!(d.drain_due(s(1100), delay), Vec::<&str>::new());
        // ディレイを0にすると残りはすぐ出る
        assert_eq!(d.drain_due(s(1100), Duration::ZERO), vec!["c"]);
        assert!(d.is_empty());
    }

    #[test]
    fn zero_delay_is_immediate() {
        let t0 = Instant::now();
        let mut d = DelayLine::new();
        d.push(t0, 1);
        assert_eq!(d.drain_due(t0, Duration::ZERO), vec![1]);
    }

    #[test]
    fn frame_ring_picks_past_frame_and_trims() {
        let t0 = Instant::now();
        let s = |x: u64| t0 + Duration::from_millis(x);
        let mut r = FrameRing::new(Duration::from_millis(1000));
        for i in 0..30u64 {
            r.push(s(i * 100), i);
        }
        // 古いもの（1秒より前）は捨てる
        assert!(r.len() <= 11);
        assert_eq!(r.latest().unwrap().1, 29);
        // 0.5秒前の映像
        assert_eq!(r.at_or_before(s(2900 - 500)).unwrap().1, 24);
        assert_eq!(r.at_or_before(s(2450)).unwrap().1, 24);
        // 残っている範囲より前は None
        assert!(r.at_or_before(s(100)).is_none());
    }
}

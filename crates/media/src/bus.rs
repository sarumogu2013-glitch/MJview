//! 映像の通り道: いちばん新しい JPEG を1枚持ち、新しいのが来たら待っている人を起こす。

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Inner {
    seq: u64,
    frame: Option<Arc<Vec<u8>>>,
    at: Option<Instant>,
    window_start: Option<Instant>,
    window_frames: u32,
    fps: f32,
}

#[derive(Default)]
pub struct Bus {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Bus {
    pub fn push(&self, jpeg: Vec<u8>) {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        g.seq += 1;
        g.frame = Some(Arc::new(jpeg));
        g.at = Some(now);
        g.window_frames += 1;
        let ws = *g.window_start.get_or_insert(now);
        let el = now.duration_since(ws).as_secs_f32();
        if el >= 1.0 {
            g.fps = g.window_frames as f32 / el;
            g.window_frames = 0;
            g.window_start = Some(now);
        }
        drop(g);
        self.cv.notify_all();
    }

    pub fn latest(&self) -> Option<(u64, Arc<Vec<u8>>)> {
        let g = self.inner.lock().unwrap();
        g.frame.clone().map(|f| (g.seq, f))
    }

    /// seq より新しい1枚を待つ（timeout まで）
    pub fn wait_newer(&self, seq: u64, timeout: Duration) -> Option<(u64, Arc<Vec<u8>>)> {
        let g = self.inner.lock().unwrap();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |g| g.seq <= seq)
            .unwrap();
        if g.seq > seq {
            g.frame.clone().map(|f| (g.seq, f))
        } else {
            None
        }
    }

    /// 2秒以内に届いている
    pub fn live(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
    }

    pub fn fps(&self) -> f32 {
        let g = self.inner.lock().unwrap();
        if g.at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            g.fps
        } else {
            0.0
        }
    }
}

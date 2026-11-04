//! 顔カメラの取り込み（設計書 4.11）。FFmpeg を別プロセスとして起動し、RTMP を受けて MJPEG で受け取る。
//!
//! - FFmpeg は別のプログラムとして呼ぶだけ（リンクしない）。配布では LGPL ビルドの ffmpeg.exe を同梱する
//! - 受け方は2通り: 配信元のサーバーから引っ張る（Pull）／本ソフト側で待ち受ける（Listen）
//! - FFmpeg が出力ウィンドウの大きさに縮小し、MJPEG（圧縮データ）で渡す。配信ディレイ（段階B7）は
//!   この圧縮データを時刻付きで溜めて、遅らせてからデコードする
//! - 途切れたら自動で FFmpeg を起動し直す。一定時間フレームが来なければ「信号なし」

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RtmpMode {
    /// 配信元のサーバーから引っ張る（rtmp://server/app/key）
    #[default]
    Pull,
    /// 本ソフトが待ち受ける（配信元から rtmp://このPC:1935/live/<key> に送ってもらう）
    Listen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamConfig {
    pub url: String,
    pub mode: RtmpMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// ffmpeg の実行ファイル
    pub ffmpeg: String,
}

impl StreamConfig {
    pub fn args(&self) -> Vec<String> {
        // 注意: -fflags nobuffer は Listen で入力の判定前にフレームを捨ててしまうので付けない
        let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        match self.mode {
            RtmpMode::Listen => a.extend(["-listen".into(), "1".into()]),
            RtmpMode::Pull => {
                if self.url.starts_with("rtmp") {
                    a.extend(["-rw_timeout".into(), "5000000".into()]);
                }
            }
        }
        a.extend(["-i".into(), self.url.clone(), "-an".into()]);
        let (w, h) = (self.width, self.height);
        a.extend([
            "-vf".into(),
            format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps={}", self.fps),
            "-c:v".into(),
            "mjpeg".into(),
            "-q:v".into(),
            "4".into(),
            "-f".into(),
            "image2pipe".into(),
            "-".into(),
        ]);
        a
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamStatus {
    Starting,
    Live,
    NoSignal,
    /// ffmpeg が起動できない（見つからないなど）
    Failed(String),
}

/// 1フレーム（JPEG のまま）。
#[derive(Clone, Debug)]
pub struct JpegFrame {
    pub data: Arc<Vec<u8>>,
    /// 受け取った時刻
    pub at: Instant,
    pub seq: u64,
}

#[derive(Debug)]
struct Shared {
    latest: Option<JpegFrame>,
    status: StreamStatus,
    frames: u64,
    fps: f32,
    last_error: Option<String>,
}

/// 顔カメラ1本分の取り込み。drop すると ffmpeg を止める。
pub struct FaceStream {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// この時間フレームが来なければ「信号なし」。
pub const NO_SIGNAL_AFTER: Duration = Duration::from_secs(2);

impl FaceStream {
    pub fn start(cfg: StreamConfig) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            latest: None,
            status: StreamStatus::Starting,
            frames: 0,
            fps: 0.0,
            last_error: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (s2, st2, c2) = (shared.clone(), stop.clone(), child.clone());
        let thread = std::thread::Builder::new()
            .name("janscope-face".into())
            .spawn(move || run(cfg, s2, st2, c2))
            .expect("thread");
        FaceStream {
            shared,
            stop,
            child,
            thread: Some(thread),
        }
    }

    /// 最新のフレーム（JPEG）。
    pub fn latest(&self) -> Option<JpegFrame> {
        self.shared.lock().unwrap().latest.clone()
    }

    pub fn status(&self) -> StreamStatus {
        let sh = self.shared.lock().unwrap();
        match (&sh.status, &sh.latest) {
            (StreamStatus::Live, Some(f)) if f.at.elapsed() > NO_SIGNAL_AFTER => {
                StreamStatus::NoSignal
            }
            (s, _) => s.clone(),
        }
    }

    pub fn fps(&self) -> f32 {
        self.shared.lock().unwrap().fps
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.lock().unwrap().last_error.clone()
    }
}

impl Drop for FaceStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    cfg: StreamConfig,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    let mut backoff = Duration::from_millis(500);
    let mut seq = 0u64;
    while !stop.load(Ordering::SeqCst) {
        let mut cmd = Command::new(&cfg.ffmpeg);
        cmd.args(cfg.args())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("ffmpeg を起動できません（{}）: {e}", cfg.ffmpeg);
                let mut sh = shared.lock().unwrap();
                sh.status = StreamStatus::Failed(msg.clone());
                sh.last_error = Some(msg);
                drop(sh);
                sleep_unless_stopped(&stop, Duration::from_secs(5));
                continue;
            }
        };
        let mut stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        *child_slot.lock().unwrap() = Some(child);
        // エラー出力は最後の1行だけ覚えておく
        let sh_err = shared.clone();
        let err_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let mut r = stderr;
            let _ = r.read_to_string(&mut s);
            if let Some(l) = s.lines().rev().find(|l| !l.trim().is_empty()) {
                sh_err.lock().unwrap().last_error = Some(l.to_string());
            }
        });
        let mut splitter = MjpegSplitter::default();
        let mut buf = vec![0u8; 256 * 1024];
        let mut window_start = Instant::now();
        let mut window_frames = 0u32;
        let mut got_any = false;
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let n = match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for jpeg in splitter.push(&buf[..n]) {
                seq += 1;
                window_frames += 1;
                got_any = true;
                let mut sh = shared.lock().unwrap();
                sh.latest = Some(JpegFrame {
                    data: Arc::new(jpeg),
                    at: Instant::now(),
                    seq,
                });
                sh.status = StreamStatus::Live;
                sh.frames += 1;
                let el = window_start.elapsed().as_secs_f32();
                if el >= 1.0 {
                    sh.fps = window_frames as f32 / el;
                    window_start = Instant::now();
                    window_frames = 0;
                }
            }
        }
        if let Some(mut c) = child_slot.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = err_thread.join();
        {
            let mut sh = shared.lock().unwrap();
            if !matches!(sh.status, StreamStatus::Failed(_)) {
                sh.status = StreamStatus::NoSignal;
            }
            sh.fps = 0.0;
        }
        if got_any {
            backoff = Duration::from_millis(500);
        }
        sleep_unless_stopped(&stop, backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// MJPEG（JPEG の連結）を1枚ずつに分ける。SOI(FFD8)〜EOI(FFD9)。
/// JPEG の圧縮データ中の 0xFF は 0xFF00 に置き換えられているので、FFD9 は終端にしか現れない。
#[derive(Default)]
pub struct MjpegSplitter {
    buf: Vec<u8>,
}

impl MjpegSplitter {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(start) = find(&self.buf, &[0xFF, 0xD8]) else {
                // 先頭以外のゴミは捨てる（最後の1バイトは FF の可能性があるので残す）
                let keep = self.buf.len().min(1);
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            let Some(end) = find(&self.buf[start + 2..], &[0xFF, 0xD9]).map(|e| start + 2 + e + 2)
            else {
                if start > 0 {
                    self.buf.drain(..start);
                }
                // 異常に大きくなったら捨てる（壊れたデータ）
                if self.buf.len() > 32 * 1024 * 1024 {
                    self.buf.clear();
                }
                break;
            };
            out.push(self.buf[start..end].to_vec());
            self.buf.drain(..end);
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// JPEG → RGBA（幅, 高さ, 画素）。
pub fn decode_rgba(jpeg: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut d = JpegDecoder::new_with_options(ZCursor::new(jpeg), opts);
    let px = d.decode().map_err(|e| e.to_string())?;
    let (w, h) = d.dimensions().ok_or("大きさが分かりません")?;
    Ok((w as u32, h as u32, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_handles_chunks() {
        let a = vec![0xFF, 0xD8, 1, 2, 0xFF, 0x00, 3, 0xFF, 0xD9];
        let b = vec![0xFF, 0xD8, 9, 9, 0xFF, 0xD9];
        let mut all = vec![0u8, 7];
        all.extend(&a);
        all.extend(&b);
        let mut s = MjpegSplitter::default();
        let mut got = vec![];
        for chunk in all.chunks(3) {
            got.extend(s.push(chunk));
        }
        assert_eq!(got, vec![a, b]);
    }

    #[test]
    fn listen_args() {
        let c = StreamConfig {
            url: "rtmp://0.0.0.0:1935/live/a".into(),
            mode: RtmpMode::Listen,
            width: 1066,
            height: 600,
            fps: 30,
            ffmpeg: "ffmpeg".into(),
        };
        let a = c.args();
        assert!(a.windows(2).any(|w| w == ["-listen", "1"]));
        assert!(a.iter().any(|x| x.contains("scale=1066:600")));
    }
}

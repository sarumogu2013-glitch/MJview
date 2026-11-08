//! ソースごとの処理: ffmpeg を1つ起動して、確認用の縮小映像と、枠ごとの切り出し映像を同時に作る。
//!
//! - RTMP は受け口から届く FLV を ffmpeg の標準入力へ流す。設定が変わって ffmpeg を起動し直すときは、
//!   覚えておいた頭の部分（FLV の頭・メタデータ・映像の設定）を先に流してから続きを流す（カメラはつないだまま）
//! - ffmpeg の出力は、出力ごとに 127.0.0.1 のポートで受ける（tcp://）

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use janscope_video::MjpegSplitter;
use janscope_video::sandbox::{SourceDef, SourceKind};

use crate::{State, now};

/// 確認用の縮小映像のフィルター
pub const PREVIEW_FILTER: &str =
    "fps=5,scale=960:540:force_original_aspect_ratio=decrease:force_divisible_by=2";

/// RTMP の FLV を ffmpeg へ渡すところ
#[derive(Default)]
struct Feed {
    /// 頭の部分（FLV の頭・メタデータ・映像の設定）
    header: Option<Vec<u8>>,
    script: Option<Vec<u8>>,
    seq_header: Option<Vec<u8>>,
    stdin: Option<ChildStdin>,
    /// 映像が届いている
    active: bool,
}

impl Feed {
    fn remember(&mut self, chunk: &[u8]) {
        if chunk.starts_with(b"FLV") {
            self.header = Some(chunk.to_vec());
            self.script = None;
            self.seq_header = None;
        } else if chunk.first() == Some(&18) {
            self.script = Some(chunk.to_vec());
        } else if chunk.first() == Some(&9)
            && chunk.len() > 12
            && chunk[11] & 0x0f == 7
            && chunk[12] == 0
        {
            self.seq_header = Some(chunk.to_vec());
        }
    }
    fn init(&self) -> Vec<&Vec<u8>> {
        [&self.header, &self.script, &self.seq_header]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// ffmpeg の引数（入力 → 分ける → 出力ごとに tcp://127.0.0.1:port）
pub fn ffmpeg_args(kind: &SourceKind, outputs: &[(String, u16)]) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.extend(kind.input_args());
    a.push("-an".into());
    let n = outputs.len();
    let mut graph = String::new();
    if n == 1 {
        graph.push_str(&format!("[0:v]{}[o0]", outputs[0].0));
    } else {
        graph.push_str(&format!("[0:v]split={n}"));
        for i in 0..n {
            graph.push_str(&format!("[s{i}]"));
        }
        for (i, (f, _)) in outputs.iter().enumerate() {
            graph.push_str(&format!(";[s{i}]{f}[o{i}]"));
        }
    }
    a.extend(["-filter_complex".into(), graph]);
    for (i, (_, port)) in outputs.iter().enumerate() {
        a.extend([
            "-map".into(),
            format!("[o{i}]"),
            "-c:v".into(),
            "mjpeg".into(),
            "-q:v".into(),
            "5".into(),
            "-f".into(),
            "mjpeg".into(),
            format!("tcp://127.0.0.1:{port}"),
        ]);
    }
    a
}

/// ffmpeg がつないでくるのを待ち、届いた MJPEG を通り道へ流す（done が立つか、ffmpeg が切るまで）
fn receive(l: TcpListener, bus: Arc<crate::Bus>, done: Arc<AtomicBool>) {
    let _ = l.set_nonblocking(true);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut s = loop {
        match l.accept() {
            Ok((s, _)) => break s,
            Err(_) if !done.load(Ordering::SeqCst) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return,
        }
    };
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let mut sp = MjpegSplitter::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for j in sp.push(&buf[..n]) {
                    bus.push(j);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if done.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// 1つのソースの処理（stop が立つまで）
pub fn run(src: SourceDef, st: Arc<State>, stop: Arc<AtomicBool>) {
    let feed = Arc::new(Mutex::new(Feed::default()));
    // RTMP: 受け口から届く FLV を覚えつつ、今の ffmpeg へ流す
    if let SourceKind::Rtmp { app, key } = &src.kind {
        let sub = st.rtmp.subscribe(app, key);
        let (f2, stop2, name) = (feed.clone(), stop.clone(), src.name.clone());
        std::thread::spawn(move || {
            // sub はこのスレッドが持つ（止まると受け口の登録も消える）
            while !stop2.load(Ordering::SeqCst) {
                let session = match sub.sessions.recv_timeout(Duration::from_millis(300)) {
                    Ok(s) => s,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                println!("{} 映像が届きました: {name}", now());
                f2.lock().unwrap().active = true;
                for chunk in session.flv.iter() {
                    let mut f = f2.lock().unwrap();
                    f.remember(&chunk);
                    if let Some(w) = f.stdin.as_mut() {
                        if w.write_all(&chunk).is_err() {
                            f.stdin = None;
                        }
                    }
                }
                let mut f = f2.lock().unwrap();
                f.active = false;
                f.stdin = None;
                println!("{} 映像が切れました: {name}", now());
            }
        });
    }
    let rtmp = matches!(src.kind, SourceKind::Rtmp { .. });
    let mut backoff = Duration::from_millis(500);
    while !stop.load(Ordering::SeqCst) {
        if rtmp && !feed.lock().unwrap().active {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let ver = st.version.load(Ordering::SeqCst);
        let regions: Vec<_> = st
            .config
            .lock()
            .unwrap()
            .regions
            .iter()
            .filter(|r| r.source == src.id)
            .cloned()
            .collect();
        let mut outs: Vec<(String, String)> =
            vec![(format!("preview/{}", src.id), PREVIEW_FILTER.to_string())];
        for r in &regions {
            outs.push((format!("region/{}", r.role.id()), r.filter()));
        }
        // 出力ごとに受け口を開く
        let done = Arc::new(AtomicBool::new(false));
        let mut ports = vec![];
        let mut receivers = vec![];
        for (bus_name, filter) in &outs {
            let Ok(l) = TcpListener::bind(("127.0.0.1", 0)) else {
                continue;
            };
            let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
            ports.push((filter.clone(), port));
            let (bus, d) = (st.bus(bus_name), done.clone());
            receivers.push(std::thread::spawn(move || receive(l, bus, d)));
        }
        let args = ffmpeg_args(&src.kind, &ports);
        let mut cmd = Command::new(&st.ffmpeg);
        cmd.args(&args)
            .stdin(if rtmp { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child: Child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("ffmpeg を起動できません（{}）: {e}", st.ffmpeg);
                if st
                    .errors
                    .lock()
                    .unwrap()
                    .insert(src.id.clone(), msg.clone())
                    != Some(msg.clone())
                {
                    println!("{} {}: {msg}", now(), src.name);
                }
                done.store(true, Ordering::SeqCst);
                sleep_unless(&stop, Duration::from_secs(5));
                continue;
            }
        };
        // エラー出力は最後の1行を覚えて、ログにも出す
        if let Some(mut err) = child.stderr.take() {
            let (st2, id, name) = (st.clone(), src.id.clone(), src.name.clone());
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = err.read_to_string(&mut s);
                if let Some(l) = s.lines().rev().find(|l| !l.trim().is_empty()) {
                    println!("{} {name}: ffmpeg: {l}", now());
                    st2.errors.lock().unwrap().insert(id, l.to_string());
                }
            });
        }
        if rtmp {
            if let Some(mut w) = child.stdin.take() {
                let mut f = feed.lock().unwrap();
                let ok = f.init().into_iter().all(|c| w.write_all(c).is_ok());
                if ok {
                    f.stdin = Some(w);
                }
            }
        }
        let started = Instant::now();
        let mut restart = false;
        loop {
            if stop.load(Ordering::SeqCst)
                || st.version.load(Ordering::SeqCst) != ver
                || (rtmp && !feed.lock().unwrap().active)
            {
                restart = true;
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            // 10秒動いていれば、前のエラーは消す（止めた前の ffmpeg が最後に出したものなど）
            if started.elapsed() > Duration::from_secs(10) {
                st.errors.lock().unwrap().remove(&src.id);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = child.kill();
        let _ = child.wait();
        feed.lock().unwrap().stdin = None;
        done.store(true, Ordering::SeqCst);
        for r in receivers {
            let _ = r.join();
        }
        if started.elapsed() > Duration::from_secs(10) {
            backoff = Duration::from_millis(500);
            st.errors.lock().unwrap().remove(&src.id);
        }
        if !restart {
            sleep_unless(&stop, backoff);
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// ソースの増減に合わせて処理を起動・停止する（0.5秒ごと）
pub fn manage(st: Arc<State>) {
    let mut running: HashMap<String, (SourceDef, Arc<AtomicBool>)> = HashMap::new();
    loop {
        let want = st.sources();
        let ids: HashSet<String> = want.iter().map(|s| s.id.clone()).collect();
        // なくなった・中身が変わったソースは止める
        running.retain(|id, (def, stop)| {
            let keep = ids.contains(id) && want.iter().any(|w| w == def);
            if !keep {
                stop.store(true, Ordering::SeqCst);
            }
            keep
        });
        for s in want {
            if !running.contains_key(&s.id) {
                let stop = Arc::new(AtomicBool::new(false));
                let (st2, s2, stop2) = (st.clone(), s.clone(), stop.clone());
                println!("{} ソース: {}（{}）", now(), s.name, s.kind.label());
                std::thread::spawn(move || run(s2, st2, stop2));
                running.insert(s.id.clone(), (s, stop));
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_split_outputs() {
        let a = ffmpeg_args(
            &SourceKind::Image {
                path: "a.png".into(),
            },
            &[("fps=5".into(), 1000), ("crop=1:1".into(), 1001)],
        );
        let g = a
            .iter()
            .position(|x| x == "-filter_complex")
            .map(|i| a[i + 1].clone())
            .unwrap();
        assert_eq!(g, "[0:v]split=2[s0][s1];[s0]fps=5[o0];[s1]crop=1:1[o1]");
        assert!(a.iter().any(|x| x == "tcp://127.0.0.1:1001"));
        let one = ffmpeg_args(
            &SourceKind::Image {
                path: "a.png".into(),
            },
            &[("fps=5".into(), 1000)],
        );
        assert!(one.iter().any(|x| x == "[0:v]fps=5[o0]"));
    }
}

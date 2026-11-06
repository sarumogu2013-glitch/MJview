//! 実際に RTMP で受け取るテスト。ffmpeg が PATH にない環境では何もしない。
//! 本ソフト側が待ち受け（Listen）、別の ffmpeg がテスト映像を送る。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use janscope_video::{FaceStream, RtmpMode, StreamConfig, StreamStatus, decode_rgba};

/// テスト映像を作るエンコーダー。LGPL ビルドの ffmpeg には libx264 がないので、ある物を使う
/// （JanScope が受けて ffmpeg でデコードするのはどれも同じ。H.264 のデコードは LGPL ビルドにもある）
fn encoder() -> &'static str {
    static E: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    E.get_or_init(|| {
        if let Ok(e) = std::env::var("JANSCOPE_TEST_ENCODER") {
            return Box::leak(e.into_boxed_str());
        }
        let out = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        ["libx264", "libopenh264", "h264_mf"]
            .into_iter()
            .find(|e| out.contains(&format!(" {e} ")))
            // FLV の昔の映像形式（Sorenson H.263）。ffmpeg に必ず入っている
            .unwrap_or("flv")
    })
}

/// エンコーダーごとの引数（キーフレームは1秒ごと。実際のカメラも1〜2秒ごと）
fn encode_args() -> Vec<&'static str> {
    let mut a = vec!["-c:v", encoder(), "-g", "30", "-pix_fmt", "yuv420p"];
    if encoder() == "libx264" {
        a.extend(["-preset", "ultrafast", "-tune", "zerolatency"]);
    }
    a
}

fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[test]
fn receive_rtmp_listen_and_recover() {
    if !has_ffmpeg() {
        eprintln!("ffmpeg がないのでスキップ");
        return;
    }
    let port = 19350 + (std::process::id() % 500) as u16;
    let url = format!("rtmp://127.0.0.1:{port}/live/seatA");
    let stream = FaceStream::start(StreamConfig {
        url: url.clone(),
        mode: RtmpMode::Listen,
        width: 320,
        height: 180,
        fps: 15,
        ffmpeg: "ffmpeg".into(),
    });
    let push = || {
        Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x360:rate=30",
            ])
            .args(["-t", "4"])
            .args(encode_args())
            .args(["-f", "flv", &url])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("push")
    };
    // 待ち受けが始まるまで少し待ってから送る
    std::thread::sleep(Duration::from_millis(800));
    let mut sender = push();
    let deadline = Instant::now() + Duration::from_secs(10);
    while stream.status() != StreamStatus::Live && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        stream.status(),
        StreamStatus::Live,
        "映像が届かない: {:?}",
        stream.last_error()
    );
    let f = stream.latest().unwrap();
    let (w, h, px) = decode_rgba(&f.data).unwrap();
    assert_eq!((w, h), (320, 180));
    assert_eq!(px.len(), (320 * 180 * 4) as usize);
    let _ = sender.wait();
    // 送信が止まると「信号なし」になる
    let deadline = Instant::now() + Duration::from_secs(6);
    while stream.status() == StreamStatus::Live && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(stream.status(), StreamStatus::NoSignal);
    // 送信を再開すると自動で戻る（ffmpeg を起動し直して待ち受ける）
    std::thread::sleep(Duration::from_millis(1500));
    let mut sender = push();
    let seq_before = stream.latest().map(|f| f.seq).unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(12);
    while !(stream.status() == StreamStatus::Live && stream.latest().unwrap().seq > seq_before)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        stream.status(),
        StreamStatus::Live,
        "復帰しない: {:?}",
        stream.last_error()
    );
    let _ = sender.kill();
}

#[test]
fn missing_ffmpeg_is_reported() {
    let stream = FaceStream::start(StreamConfig {
        url: "rtmp://127.0.0.1:1/x".into(),
        mode: RtmpMode::Pull,
        width: 320,
        height: 180,
        fps: 15,
        ffmpeg: "/nonexistent/ffmpeg".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !matches!(stream.status(), StreamStatus::Failed(_)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(matches!(stream.status(), StreamStatus::Failed(m) if m.contains("ffmpeg")));
}

/// テスト映像を送る ffmpeg（`secs` 秒）
fn push_to(url: &str, secs: u32) -> std::process::Child {
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=30",
        ])
        .args(["-t", &secs.to_string()])
        .args(encode_args())
        .args(["-f", "flv", url])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("push")
}

/// 4台を同じポートで同時に受ける（以前は ffmpeg が1台ずつ待ち受けていたので2台目以降を受けられなかった）
#[test]
fn four_cameras_on_one_port() {
    use janscope_video::rtmp_server::server_status;
    if !has_ffmpeg() {
        eprintln!("ffmpeg がないのでスキップ");
        return;
    }
    let port = 20350 + (std::process::id() % 500) as u16;
    let seats = ["seatA", "seatB", "seatC", "seatD"];
    let streams: Vec<FaceStream> = seats
        .iter()
        .map(|k| {
            FaceStream::start(StreamConfig {
                url: format!("rtmp://0.0.0.0:{port}/live/{k}"),
                mode: RtmpMode::Listen,
                width: 320,
                height: 180,
                fps: 15,
                ffmpeg: "ffmpeg".into(),
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(500));
    let mut senders: Vec<_> = seats
        .iter()
        .map(|k| push_to(&format!("rtmp://127.0.0.1:{port}/live/{k}"), 6))
        .collect();
    // どの席にも割り当てていないキー
    senders.push(push_to(&format!("rtmp://127.0.0.1:{port}/live/stray"), 6));
    let deadline = Instant::now() + Duration::from_secs(10);
    while streams.iter().any(|s| s.status() != StreamStatus::Live) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    for (k, s) in seats.iter().zip(&streams) {
        assert_eq!(s.status(), StreamStatus::Live, "{k}: {:?}", s.last_error());
    }
    // 同じキーで2台目は断られる（すぐ終わる）
    let mut dup = push_to(&format!("rtmp://127.0.0.1:{port}/live/seatA"), 6);
    let t = Instant::now();
    let st = loop {
        if let Some(s) = dup.try_wait().unwrap() {
            break Some(s);
        }
        if t.elapsed() > Duration::from_secs(4) {
            let _ = dup.kill();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        st.is_some_and(|s| !s.success()),
        "同じキーの2台目が断られない"
    );

    let status = server_status(port).expect("受け口が開いている");
    assert!(status.bind_error.is_none());
    let keys: Vec<_> = status
        .publishers
        .iter()
        .map(|p| (p.key.as_str(), p.registered))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("seatA", true),
            ("seatB", true),
            ("seatC", true),
            ("seatD", true),
            ("stray", false)
        ]
    );
    let a = &status.publishers[0];
    assert_eq!(
        (a.width, a.height),
        (Some(640), Some(360)),
        "カメラが知らせた解像度"
    );
    assert!(a.video_bytes > 0);
    for mut s in senders {
        let _ = s.kill();
        let _ = s.wait();
    }
}

/// カメラが先に送り始めていても、あとから受け手を作れば映る。受け手を作り直しても（設定の適用し直し）映り続ける
#[test]
fn late_subscriber_and_reapply() {
    if !has_ffmpeg() {
        eprintln!("ffmpeg がないのでスキップ");
        return;
    }
    let port = 21350 + (std::process::id() % 500) as u16;
    let cfg = StreamConfig {
        url: format!("rtmp://0.0.0.0:{port}/live/seatA"),
        mode: RtmpMode::Listen,
        width: 320,
        height: 180,
        fps: 15,
        ffmpeg: "ffmpeg".into(),
    };
    // 受け口だけ先に開けておき（別の席が使っている状態）、カメラに送り始めてもらう
    let other = FaceStream::start(StreamConfig {
        url: format!("rtmp://0.0.0.0:{port}/live/seatB"),
        ..cfg.clone()
    });
    std::thread::sleep(Duration::from_millis(500));
    let mut sender = push_to(&format!("rtmp://127.0.0.1:{port}/live/seatA"), 20);
    std::thread::sleep(Duration::from_secs(2));
    let wait_live = |s: &FaceStream| {
        let deadline = Instant::now() + Duration::from_secs(8);
        while s.status() != StreamStatus::Live && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        s.status()
    };
    // あとから受け手
    let first = FaceStream::start(cfg.clone());
    assert_eq!(
        wait_live(&first),
        StreamStatus::Live,
        "あとから付けた受け手に映らない: {:?}",
        first.last_error()
    );
    // 作り直し: 新しいほうを先に作ってから古いほうを捨てる（FaceRuntime::apply と同じ順）
    let second = FaceStream::start(cfg.clone());
    std::thread::sleep(Duration::from_millis(300));
    drop(first);
    assert_eq!(
        wait_live(&second),
        StreamStatus::Live,
        "作り直した受け手に映らない: {:?}",
        second.last_error()
    );
    let seq = second.latest().unwrap().seq;
    std::thread::sleep(Duration::from_secs(1));
    assert!(second.latest().unwrap().seq > seq, "映像が進んでいない");
    drop(other);
    let _ = sender.kill();
    let _ = sender.wait();
}

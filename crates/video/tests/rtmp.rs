//! 実際に RTMP で受け取るテスト。ffmpeg が PATH にない環境では何もしない。
//! 本ソフト側が待ち受け（Listen）、別の ffmpeg がテスト映像を送る。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use janscope_video::{FaceStream, RtmpMode, StreamConfig, StreamStatus, decode_rgba};

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
            .args([
                "-t",
                "4",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-f",
                "flv",
                &url,
            ])
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

//! 動いている実行ファイルを入れ替えられるか（Windows では動いている exe は消せないが、改名はできる）。
//! 害のない常駐しないプログラム（ping / sleep）を「配布フォルダの exe」として動かしたまま入れ替える。

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use janscope_update::*;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("janscope-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 数十秒動き続けるプログラムと、その引数
fn long_running() -> Option<(PathBuf, Vec<&'static str>)> {
    if cfg!(windows) {
        let p = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
            .join(r"System32\PING.EXE");
        p.is_file().then(|| (p, vec!["-n", "60", "127.0.0.1"]))
    } else {
        ["/bin/sleep", "/usr/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .map(|p| (p, vec!["60"]))
    }
}

#[test]
fn replaces_running_executable() {
    let Some((prog, args)) = long_running() else {
        eprintln!("ping / sleep が見つからないので飛ばします");
        return;
    };
    let exe = if cfg!(windows) {
        "janscope-server.exe"
    } else {
        "janscope-server"
    };
    let inst = tmpdir("inst");
    std::fs::write(inst.join(DIST_MARKER), "0.1.0\n").unwrap();
    std::fs::copy(&prog, inst.join(exe)).unwrap();
    let mut child = Command::new(inst.join(exe))
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "入れ替える前に止まってしまった"
    );

    // 新しい版
    let staged_root = tmpdir("staged");
    std::fs::write(staged_root.join(DIST_MARKER), "0.2.0\n").unwrap();
    std::fs::write(staged_root.join(exe), b"new executable").unwrap();
    let staged = Staged {
        root: staged_root.clone(),
        files: vec![PathBuf::from(DIST_MARKER), PathBuf::from(exe)],
    };
    let r = apply(&staged, &inst).expect("動いている exe を入れ替えられない");
    assert_eq!(
        r,
        Applied {
            replaced: 2,
            added: 0
        }
    );
    assert_eq!(std::fs::read(inst.join(exe)).unwrap(), b"new executable");
    assert!(inst.join(format!("{exe}.old")).is_file());
    // 古い exe はまだ動いている
    assert!(child.try_wait().unwrap().is_none());

    // 動いている間は、Windows では .old の exe を消せないことがある（消せなくても失敗ではない）
    let n_running = cleanup_old(&inst);
    eprintln!("動いている間に消せた .old: {n_running}");

    child.kill().unwrap();
    child.wait().unwrap();
    // 止めたあとは全部消える（Windows はプロセスが終わってから少し待つことがある）
    let mut left = 1;
    for _ in 0..20 {
        cleanup_old(&inst);
        left = std::fs::read_dir(&inst)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "old"))
            .count();
        if left == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(left, 0, ".old が残った");
    let _ = std::fs::remove_dir_all(&inst);
    let _ = std::fs::remove_dir_all(&staged_root);
}

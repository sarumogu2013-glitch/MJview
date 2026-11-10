//! ファイル・フォルダを選ぶダイアログ（「参照…」ボタン）。ダイアログは別のスレッドで開き、選んだ結果を次のフレームで欄に入れる。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bevy_egui::egui;

/// 何を選ぶか
#[derive(Clone, Copy)]
pub enum Pick {
    /// ファイル（名前, 拡張子の一覧）
    File(&'static str, &'static [&'static str]),
    Folder,
}

/// 選び終わった結果（欄の id → パス）と、開いている最中の id
fn state() -> &'static Mutex<(HashMap<String, String>, Vec<String>)> {
    static S: OnceLock<Mutex<(HashMap<String, String>, Vec<String>)>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "gif"];
pub const VIDEO_EXT: &[&str] = &[
    "mp4", "mov", "mkv", "avi", "webm", "flv", "ts", "m4v", "wmv",
];

/// ダイアログを開く（すでに開いていれば何もしない）
pub fn open(id: &str, pick: Pick, start: &str) {
    {
        let mut st = state().lock().unwrap();
        if st.1.iter().any(|x| x == id) {
            return;
        }
        st.1.push(id.to_string());
    }
    let id = id.to_string();
    let start = std::path::PathBuf::from(start.trim());
    std::thread::spawn(move || {
        let mut d = rfd::FileDialog::new();
        let dir = if start.is_dir() {
            Some(start.clone())
        } else {
            start
                .parent()
                .filter(|p| p.is_dir())
                .map(|p| p.to_path_buf())
        };
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        let r = match pick {
            Pick::File(name, exts) => d
                .add_filter(name, exts)
                .add_filter("すべてのファイル", &["*"])
                .pick_file(),
            Pick::Folder => d.pick_folder(),
        };
        let mut st = state().lock().unwrap();
        st.1.retain(|x| *x != id);
        if let Some(p) = r {
            st.0.insert(id, p.to_string_lossy().into_owned());
        }
    });
}

/// 「参照…」ボタン。選び終わったら target に入れて true を返す
pub fn button(ui: &mut egui::Ui, id: &str, pick: Pick, target: &mut String) -> bool {
    let busy = state().lock().unwrap().1.iter().any(|x| x == id);
    if ui
        .add_enabled(!busy, egui::Button::new("参照…"))
        .on_hover_text(match pick {
            Pick::Folder => "フォルダを選ぶ",
            _ => "ファイルを選ぶ",
        })
        .clicked()
    {
        open(id, pick, target);
    }
    take(id, target)
}

/// 選び終わった結果があれば target に入れて true
pub fn take(id: &str, target: &mut String) -> bool {
    match state().lock().unwrap().0.remove(id) {
        Some(p) => {
            *target = p;
            true
        }
        None => false,
    }
}

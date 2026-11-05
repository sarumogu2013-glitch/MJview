//! 設定ウィンドウの「更新」「情報」タブと、起動時の新しい版の確認（設計書 9章）。
//!
//! - 起動時は確かめるだけ。入れ替えは「更新」タブのボタンを人が押したときだけ（本番配信中に勝手に変えない）
//! - 通信は別のスレッドで行い、画面は止めない

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_egui::egui;
use janscope_update::{Check, Release, Updater};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

#[derive(Clone, Debug, Default)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate(String),
    Available(Box<Release>),
    Downloading {
        version: String,
        done: u64,
        total: u64,
    },
    /// 入れ替え済み（起動し直すと新しい版になる）
    Applied(String),
    Failed(String),
}

#[derive(Default)]
pub struct UpdateState {
    status: Arc<Mutex<Status>>,
    /// 適用の確認を出しているか
    confirm: bool,
}

impl UpdateState {
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, s: Status) {
        if let Ok(mut g) = self.status.lock() {
            *g = s;
        }
    }

    fn busy(&self) -> bool {
        matches!(self.status(), Status::Checking | Status::Downloading { .. })
    }

    /// 新しい版があれば、その版
    pub fn available(&self) -> Option<String> {
        match self.status() {
            Status::Available(r) => Some(r.version.to_string()),
            _ => None,
        }
    }

    pub fn start_check(&self) {
        if self.busy() {
            return;
        }
        self.set(Status::Checking);
        let st = self.status.clone();
        std::thread::spawn(move || {
            let s = match Updater::github().check(VERSION) {
                Ok(Check::Available(r)) => Status::Available(Box::new(r)),
                Ok(Check::UpToDate { latest }) => Status::UpToDate(latest.to_string()),
                Err(e) => Status::Failed(e.to_string()),
            };
            if let Ok(mut g) = st.lock() {
                *g = s;
            }
        });
    }

    fn start_apply(&self, rel: Release) {
        let Some(dir) = janscope_update::install_dir() else {
            self.set(Status::Failed("実行ファイルの場所が分かりません".into()));
            return;
        };
        let version = rel.version.to_string();
        self.set(Status::Downloading {
            version: version.clone(),
            done: 0,
            total: rel.zip.as_ref().map_or(0, |z| z.size),
        });
        let st = self.status.clone();
        std::thread::spawn(move || {
            let up = Updater::github();
            let st2 = st.clone();
            let v2 = version.clone();
            let r = janscope_update::download_and_apply(&up, &rel, &dir, move |done, total| {
                if let Ok(mut g) = st2.lock() {
                    *g = Status::Downloading {
                        version: v2.clone(),
                        done,
                        total,
                    };
                }
            });
            let s = match r {
                Ok(_) => Status::Applied(version),
                Err(e) => Status::Failed(format!("入れ替えられませんでした（元のままです）: {e}")),
            };
            if let Ok(mut g) = st.lock() {
                *g = s;
            }
        });
    }

    /// 確認用: 新しい版がある状態にする
    pub fn demo_available(&self) {
        self.set(Status::Available(Box::new(Release {
            tag: "v9.9.9".into(),
            version: janscope_update::semver::Version::new(9, 9, 9),
            notes: "- 点数表示の不具合を直しました\n- 牌譜再生を速くしました".into(),
            page_url: format!("{REPO_URL}/releases/tag/v9.9.9"),
            zip: None,
            checksum: None,
        })));
    }
}

/// 起動時: 前回の更新の残りを消し、設定が ON なら新しい版を確かめる（配布フォルダから動いているときだけ）
pub fn startup_check(
    settings: Res<crate::settings::SettingsUi>,
    cfg: Res<crate::config::ClientConfig>,
) {
    if let Some(dir) = janscope_update::install_dir() {
        janscope_update::cleanup_old(&dir);
    }
    if std::env::var("JANSCOPE_UPDATE_DEMO").is_ok_and(|v| v == "available") {
        settings.update.demo_available();
        return;
    }
    if cfg.prefs.update.check_on_start && janscope_update::is_dist_install() {
        settings.update.start_check();
    }
}

/// 起動し直す（同じ引数で新しいプロセスを立ててから終わる）
fn restart() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|e| format!("起動し直せません: {e}"))?;
    std::process::exit(0);
}

fn mb(b: u64) -> String {
    format!("{:.1}MB", b as f64 / 1_048_576.0)
}

pub fn update_tab(
    ui: &mut egui::Ui,
    st: &mut UpdateState,
    cfg: &mut crate::config::ClientConfig,
    msg: &mut Option<String>,
) {
    ui.heading("更新");
    ui.label(format!("今の版: {VERSION}"));
    let dist = janscope_update::is_dist_install();
    if !dist {
        ui.weak(
            "開発中の実行（配布フォルダではない）なので、確認はできますが入れ替えはできません。",
        );
    }
    let mut on = cfg.prefs.update.check_on_start;
    if ui
        .checkbox(
            &mut on,
            "起動時に新しい版があるか確かめる（確かめるだけで、入れ替えはしません）",
        )
        .changed()
    {
        cfg.prefs.update.check_on_start = on;
        *msg = Some(match cfg.save() {
            Ok(()) => "保存しました".into(),
            Err(e) => e,
        });
    }
    ui.add_space(6.0);
    let status = st.status();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!st.busy(), egui::Button::new("新しい版を確かめる"))
            .clicked()
        {
            st.confirm = false;
            st.start_check();
        }
        if ui.link("リリースの一覧を開く").clicked() {
            ui.ctx()
                .open_url(egui::OpenUrl::new_tab(format!("{REPO_URL}/releases")));
        }
    });
    ui.add_space(6.0);
    match status {
        Status::Idle => {
            ui.weak("まだ確かめていません");
        }
        Status::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("確かめています…");
            });
        }
        Status::UpToDate(latest) => {
            ui.colored_label(
                egui::Color32::LIGHT_GREEN,
                format!("最新の版です（公開されている最新: {latest}）"),
            );
        }
        Status::Failed(e) => {
            ui.colored_label(egui::Color32::from_rgb(255, 140, 100), e);
        }
        Status::Downloading {
            version,
            done,
            total,
        } => {
            ui.label(format!("{version} を取ってきています…"));
            let f = if total > 0 {
                done as f32 / total as f32
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(f).text(format!("{} / {}", mb(done), mb(total))));
        }
        Status::Applied(v) => {
            ui.colored_label(
                egui::Color32::LIGHT_GREEN,
                format!("{v} に入れ替えました。起動し直すと新しい版になります。"),
            );
            ui.label(
                "サーバーや選手用画面など、同じフォルダの他のプログラムも起動し直してください。",
            );
            if ui.button("今すぐ起動し直す").clicked() {
                if let Err(e) = restart() {
                    *msg = Some(e);
                }
            }
        }
        Status::Available(rel) => {
            ui.label(
                egui::RichText::new(format!("新しい版 {} があります", rel.version))
                    .strong()
                    .color(egui::Color32::from_rgb(255, 210, 90)),
            );
            if !rel.notes.trim().is_empty() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label("変更点:");
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| ui.label(rel.notes.clone()));
                });
            }
            if ui.link("リリースのページを開く").clicked() {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(rel.page_url.clone()));
            }
            ui.add_space(6.0);
            let can = dist && rel.zip.is_some();
            if rel.zip.is_none() {
                ui.weak("このPC向けのファイルがこのリリースにはありません");
            }
            if !st.confirm {
                if ui
                    .add_enabled(can, egui::Button::new("入れ替える…"))
                    .clicked()
                {
                    st.confirm = true;
                }
            } else {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "配信中・対局中は入れ替えないでください。入れ替えたあと起動し直すまでは今の版のまま動きます。",
                    );
                    ui.label("設定・選手・対局ログ・ffmpeg はそのまま残ります。");
                    ui.horizontal(|ui| {
                        if ui.button("入れ替える").clicked() {
                            st.confirm = false;
                            st.start_apply(*rel.clone());
                        }
                        if ui.button("やめる").clicked() {
                            st.confirm = false;
                        }
                    });
                });
            }
        }
    }
}

pub fn about_tab(ui: &mut egui::Ui) {
    ui.heading("情報");
    ui.label(format!("JanScope {VERSION}"));
    ui.add_space(4.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(
            egui::RichText::new("このソフトウェアは AI（Anthropic の Claude）で作成しました")
                .strong()
                .color(egui::Color32::from_rgb(255, 210, 90)),
        );
        ui.label("人による網羅的なコードレビューは行っていません。大会など本番で使う前に、必ず実際の環境で動作と計算結果を確かめてください。");
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("ソースコード:");
        ui.hyperlink(REPO_URL);
    });
    ui.label(
        "ライセンス: MIT OR Apache-2.0（LICENSE-MIT / LICENSE-APACHE。どちらかを選んで使えます）",
    );
    ui.label(
        "同梱物と依存ライブラリのライセンス: THIRD_PARTY_LICENSES.md（配布フォルダにあります）",
    );
    ui.add_space(4.0);
    egui::Grid::new("about-third")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (what, lic) in [
                ("牌画像（FluffyStuff/riichi-mahjong-tiles）", "CC0-1.0"),
                ("フォント（Noto Sans JP）", "SIL Open Font License 1.1"),
                ("点数計算（mahc）", "BSD-3-Clause"),
                ("Bevy / egui ほか Rust の依存", "MIT / Apache-2.0 など"),
                (
                    "FFmpeg（別プログラムとして呼ぶ）",
                    "LGPL-2.1 以降（ffmpeg フォルダ）",
                ),
                ("牌認識の検出モデル（別配布）", "AGPL-3.0"),
            ] {
                ui.label(what);
                ui.label(lic);
                ui.end_row();
            }
        });
}

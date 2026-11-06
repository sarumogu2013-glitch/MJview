//! 操作ウィンドウのメニューバーとツールバーから、ウィンドウの全画面・ウィンドウ表示を切り替える。
//!
//! - 起動時はどのウィンドウも普通のウィンドウ（枠あり）。全画面にするのはボタン・メニュー・F11 から
//! - 出力ウィンドウ（OBS で取り込む）は、選んだモニターに枠なし全画面で出せる
//! - 画面（egui）側は要求を [`WindowCtl`] に積むだけ。実際の切替は [`apply_window_ctl`] が行う

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PrimaryWindow, WindowMode};
use bevy_egui::egui;

/// 操作ウィンドウがランチャー（JanScope.exe）に埋め込まれている（--embed）。
/// そのときはメニューバー・操作ウィンドウの全画面はランチャー側にあるので出さない
pub static EMBEDDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn embedded() -> bool {
    EMBEDDED.load(std::sync::atomic::Ordering::Relaxed)
}

/// 出力ウィンドウ（配信に載せる画面）の印
#[derive(Component)]
pub struct OutputWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinReq {
    /// 出力ウィンドウを、このモニター（0から）に全画面で出す
    OutputFullscreen(usize),
    OutputWindowed,
    OperatorFullscreen,
    OperatorWindowed,
    Quit,
}

/// ウィンドウの様子（毎フレーム [`apply_window_ctl`] が書く）と、画面からの要求
#[derive(Default)]
pub struct WindowCtl {
    pub requests: Vec<WinReq>,
    pub has_output: bool,
    pub output_fullscreen: bool,
    pub operator_fullscreen: bool,
    /// つながっているモニターの名前（0から）
    pub monitors: Vec<String>,
    /// 出力ウィンドウを出すモニター（ツールバーで選ぶ）
    pub output_monitor: usize,
}

impl WindowCtl {
    pub fn request(&mut self, r: WinReq) {
        self.requests.push(r);
    }

    fn monitor_label(&self, i: usize) -> String {
        match self.monitors.get(i) {
            Some(n) if !n.is_empty() => format!("モニター{}（{n}）", i + 1),
            _ => format!("モニター{}", i + 1),
        }
    }

    /// ツールバーの全画面ボタン（出力ウィンドウのモニター選択つき）と、操作ウィンドウの全画面ボタン
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        if self.has_output {
            ui.label("出力");
            if self.output_fullscreen {
                if ui
                    .button("ウィンドウに戻す")
                    .on_hover_text("出力ウィンドウを普通のウィンドウ（1920×1080）に戻します")
                    .clicked()
                {
                    self.request(WinReq::OutputWindowed);
                }
            } else {
                if self.monitors.len() > 1 {
                    let sel = self.output_monitor.min(self.monitors.len() - 1);
                    egui::ComboBox::from_id_salt("out_monitor")
                        .selected_text(format!("モニター{}", sel + 1))
                        .show_ui(ui, |ui| {
                            for i in 0..self.monitors.len() {
                                let label = self.monitor_label(i);
                                ui.selectable_value(&mut self.output_monitor, i, label);
                            }
                        });
                }
                if ui
                    .button("全画面")
                    .on_hover_text("出力ウィンドウを、選んだモニターに枠なしの全画面で出します")
                    .clicked()
                {
                    let m = self.output_monitor;
                    self.request(WinReq::OutputFullscreen(m));
                }
            }
            ui.separator();
        }
        if embedded() {
            return;
        }
        let (label, req, tip) = if self.operator_fullscreen {
            (
                "操作: ウィンドウに戻す",
                WinReq::OperatorWindowed,
                "操作ウィンドウを普通のウィンドウに戻します（F11）",
            )
        } else {
            (
                "操作: 全画面",
                WinReq::OperatorFullscreen,
                "この操作ウィンドウを全画面にします（F11）",
            )
        };
        if ui.button(label).on_hover_text(tip).clicked() {
            self.request(req);
        }
    }

    /// メニューバーの「表示」の中身
    pub fn view_menu(&mut self, ui: &mut egui::Ui) {
        if self.has_output {
            if self.output_fullscreen {
                if ui.button("出力ウィンドウをウィンドウに戻す").clicked() {
                    self.request(WinReq::OutputWindowed);
                    ui.close();
                }
            } else {
                ui.menu_button("出力ウィンドウを全画面", |ui| {
                    for i in 0..self.monitors.len().max(1) {
                        if ui.button(self.monitor_label(i)).clicked() {
                            self.output_monitor = i;
                            self.request(WinReq::OutputFullscreen(i));
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
        }
        let (label, req) = if self.operator_fullscreen {
            (
                "操作ウィンドウをウィンドウに戻す（F11）",
                WinReq::OperatorWindowed,
            )
        } else {
            ("操作ウィンドウを全画面（F11）", WinReq::OperatorFullscreen)
        };
        if ui.button(label).clicked() {
            self.request(req);
            ui.close();
        }
    }
}

/// 要求どおりにウィンドウを切り替え、様子を書き戻す。F11 で操作ウィンドウ（選手用画面ではその画面）の全画面を切り替える
pub fn apply_window_ctl(
    ui: Option<ResMut<crate::operator::OperatorUi>>,
    mut primary: Query<&mut Window, (With<PrimaryWindow>, Without<OutputWindow>)>,
    mut output: Query<&mut Window, (With<OutputWindow>, Without<PrimaryWindow>)>,
    monitors: Query<&Monitor>,
    keys: Res<ButtonInput<KeyCode>>,
    mut exit: MessageWriter<AppExit>,
) {
    let f11 = keys.just_pressed(KeyCode::F11);
    let Some(mut ui) = ui else {
        // 選手用画面（操作ウィンドウがない）: F11 だけ
        if f11 {
            if let Ok(mut w) = primary.single_mut() {
                w.mode = if matches!(w.mode, WindowMode::Windowed) {
                    WindowMode::BorderlessFullscreen(MonitorSelection::Current)
                } else {
                    WindowMode::Windowed
                };
            }
        }
        return;
    };
    let ctl = &mut ui.windows;
    if f11 && !embedded() {
        let fs = primary
            .single()
            .is_ok_and(|w| !matches!(w.mode, WindowMode::Windowed));
        ctl.request(if fs {
            WinReq::OperatorWindowed
        } else {
            WinReq::OperatorFullscreen
        });
    }
    for r in std::mem::take(&mut ctl.requests) {
        match r {
            WinReq::OutputFullscreen(i) => {
                if let Ok(mut w) = output.single_mut() {
                    w.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Index(i));
                }
            }
            WinReq::OutputWindowed => {
                if let Ok(mut w) = output.single_mut() {
                    w.mode = WindowMode::Windowed;
                }
            }
            WinReq::OperatorFullscreen => {
                if let Ok(mut w) = primary.single_mut() {
                    w.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Current);
                }
            }
            WinReq::OperatorWindowed => {
                if let Ok(mut w) = primary.single_mut() {
                    w.mode = WindowMode::Windowed;
                }
            }
            WinReq::Quit => {
                exit.write(AppExit::Success);
            }
        }
    }
    ctl.has_output = !output.is_empty();
    ctl.output_fullscreen = output
        .single()
        .is_ok_and(|w| !matches!(w.mode, WindowMode::Windowed));
    ctl.operator_fullscreen = primary
        .single()
        .is_ok_and(|w| !matches!(w.mode, WindowMode::Windowed));
    ctl.monitors = monitors
        .iter()
        .map(|m| m.name.clone().unwrap_or_default())
        .collect();
}

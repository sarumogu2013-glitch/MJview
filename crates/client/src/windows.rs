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

/// 1920×1080 で配置した画面（出力・選手用画面）を映すカメラの印。窓の大きさに合わせて縮め、真ん中に置く
#[derive(Component)]
pub struct Fit1080;

/// 窓の大きさが変わったら、画面の倍率（scale factor）を「窓 ÷ 1920×1080」にして、配置はそのままで窓に収める。
/// 縦横比が違うときは、カメラの表示範囲（viewport）を真ん中に寄せて、余りは黒にする
pub fn fit_1080(
    mut cams: Query<(&mut Camera, &bevy::camera::RenderTarget), With<Fit1080>>,
    mut windows: Query<&mut Window>,
    primary: Query<Entity, With<PrimaryWindow>>,
) {
    use bevy::camera::{RenderTarget, Viewport};
    use bevy::window::WindowRef;
    for (mut cam, target) in &mut cams {
        let e = match target {
            RenderTarget::Window(WindowRef::Entity(e)) => *e,
            RenderTarget::Window(WindowRef::Primary) => match primary.single() {
                Ok(e) => e,
                Err(_) => continue,
            },
            _ => continue,
        };
        let Ok(mut w) = windows.get_mut(e) else {
            continue;
        };
        let (pw, ph) = (
            w.resolution.physical_width(),
            w.resolution.physical_height(),
        );
        if pw == 0 || ph == 0 {
            continue;
        }
        let f = (pw as f32 / 1920.0).min(ph as f32 / 1080.0).max(0.05);
        if w.resolution
            .scale_factor_override()
            .is_none_or(|o| (o - f).abs() > 0.0005)
        {
            w.resolution.set_scale_factor_override(Some(f));
        }
        let size = UVec2::new(
            ((1920.0 * f).round() as u32).clamp(1, pw),
            ((1080.0 * f).round() as u32).clamp(1, ph),
        );
        let pos = UVec2::new((pw - size.x) / 2, (ph - size.y) / 2);
        let want = Some(Viewport {
            physical_position: pos,
            physical_size: size,
            ..default()
        });
        let same = cam
            .viewport
            .as_ref()
            .is_some_and(|v| v.physical_position == pos && v.physical_size == size);
        if !same {
            cam.viewport = want;
        }
    }
}

/// 出力ウィンドウ（配信に載せる画面）の印
#[derive(Component)]
pub struct OutputWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinReq {
    /// 出力ウィンドウを、このモニター（0から）に全画面で出す
    OutputFullscreen(usize),
    OutputWindowed,
    /// 出力ウィンドウを 1920×1080 に戻す（OBS でそのまま取り込める大きさ）
    OutputSize1080,
    OperatorFullscreen,
    OperatorWindowed,
    /// 配信画面（出力ウィンドウ）を開く（閉じたあとに開き直す）
    OpenOutput,
    Quit,
}

/// 出力ウィンドウを枠つきで出すか（`--output-borderless` なら枠なし）
#[derive(Resource, Clone, Copy)]
pub struct OutputDecor(pub bool);

/// 配信画面（出力ウィンドウ）とそのカメラを出す。閉じたら [`crate::output::cleanup_closed_output`] が片付ける
pub fn spawn_output(commands: &mut Commands, decorations: bool) {
    use bevy::camera::RenderTarget;
    use bevy::window::{PresentMode, WindowRef, WindowResolution};
    let out = commands
        .spawn(Window {
            title: "JanScope 出力".into(),
            resolution: WindowResolution::new(1920, 1080),
            // 起動時は普通のウィンドウ。大きさを変えられる（Windows のスナップ・最大化）。
            // 中身は 1920×1080 の配置を窓の大きさに合わせて縮める。全画面は操作ウィンドウのツールバー・メニューから
            decorations,
            resizable: true,
            position: WindowPosition::Automatic,
            present_mode: PresentMode::AutoVsync,
            ..default()
        })
        .insert(OutputWindow)
        .id();
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(out)),
        crate::output::OutputCamera,
        Fit1080,
    ));
}

/// ウィンドウの様子（毎フレーム [`apply_window_ctl`] が書く）と、画面からの要求
#[derive(Default)]
pub struct WindowCtl {
    pub requests: Vec<WinReq>,
    pub has_output: bool,
    pub output_fullscreen: bool,
    /// 出力ウィンドウの中身の大きさ（物理ピクセル）
    pub output_size: (u32, u32),
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
        if !self.has_output {
            if ui
                .button("配信画面を開く")
                .on_hover_text(
                    "配信画面（出力ウィンドウ）を開きます。閉じても、ここからまた開けます",
                )
                .clicked()
            {
                self.request(WinReq::OpenOutput);
            }
            ui.separator();
        }
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
                if self.output_size != (1920, 1080) && self.output_size != (0, 0) {
                    let (w, h) = self.output_size;
                    if ui
                        .button("1920×1080 に戻す")
                        .on_hover_text(format!(
                            "いまの出力ウィンドウは {w}×{h}。OBS でそのまま取り込むには 1920×1080 に戻します"
                        ))
                        .clicked()
                    {
                        self.request(WinReq::OutputSize1080);
                    }
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
        if !self.has_output && ui.button("配信画面を開く").clicked() {
            self.request(WinReq::OpenOutput);
            ui.close();
        }
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

/// すべてのウィンドウ（操作・出力・選手用画面）にアイコンを付ける。ウィンドウができたら一度だけ
pub fn set_window_icons(
    _main_thread: bevy::ecs::system::NonSendMarker,
    windows: Query<Entity, With<Window>>,
    mut done: Local<std::collections::HashSet<Entity>>,
    mut icon: Local<Option<winit::window::Icon>>,
) {
    if icon.is_none() {
        let png = include_bytes!("../../../assets/icon/janscope-256.png");
        let Ok(img) = image_from_png(png) else { return };
        *icon = winit::window::Icon::from_rgba(img.2, img.0, img.1).ok();
    }
    let Some(icon) = icon.as_ref() else { return };
    bevy::winit::WINIT_WINDOWS.with_borrow(|ww| {
        for e in &windows {
            if done.contains(&e) {
                continue;
            }
            if let Some(w) = ww.get_window(e) {
                w.set_window_icon(Some(icon.clone()));
                done.insert(e);
            }
        }
    });
}

/// PNG → (幅, 高さ, RGBA)
fn image_from_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>), ()> {
    let img = bevy::image::Image::from_buffer(
        png,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        true,
        bevy::image::ImageSampler::Default,
        bevy::asset::RenderAssetUsages::default(),
    )
    .map_err(|_| ())?;
    let (w, h) = (img.width(), img.height());
    let data = img.data.ok_or(())?;
    Ok((w, h, data))
}

/// 要求どおりにウィンドウを切り替え、様子を書き戻す。F11 で操作ウィンドウ（選手用画面ではその画面）の全画面を切り替える
#[allow(clippy::too_many_arguments)]
pub fn apply_window_ctl(
    mut commands: Commands,
    decor: Option<Res<OutputDecor>>,
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
            WinReq::OutputSize1080 => {
                if let Ok(mut w) = output.single_mut() {
                    w.mode = WindowMode::Windowed;
                    w.resolution.set_physical_resolution(1920, 1080);
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
            WinReq::OpenOutput => {
                if output.is_empty() {
                    spawn_output(&mut commands, decor.as_deref().is_none_or(|d| d.0));
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
    ctl.output_size = output
        .single()
        .map(|w| {
            (
                w.resolution.physical_width(),
                w.resolution.physical_height(),
            )
        })
        .unwrap_or((0, 0));
    ctl.operator_fullscreen = primary
        .single()
        .is_ok_and(|w| !matches!(w.mode, WindowMode::Windowed));
    ctl.monitors = monitors
        .iter()
        .map(|m| m.name.clone().unwrap_or_default())
        .collect();
}

//! janscope-client: 配信クライアント（配信用PCで起動）。
//!
//!     janscope-client --server 192.168.1.10:47800 --code 123456 [--role broadcast|input|player]
//!
//! - broadcast（既定）: 操作ウィンドウ ＋ 出力ウィンドウ（1920×1080 枠なし、OBS のゲームキャプチャで取り込む）
//! - input           : 入力担当ウィンドウ（操作ウィンドウと同じ。段階C5で手修正の画面を足す）
//! - player          : 選手用画面（会場の選手向けモニター。公開情報だけを受け取る）
//!
//! 開発用: `--demo` でサーバーなしの見本の状態を表示、`--screenshot <png>` で出力を保存して終了。

mod assets;
mod cameras;
mod config;
mod demo;
mod face;
mod net;
mod operator;
mod output;
mod player_screen;
mod prefs;
mod program;
mod replay;
mod scenes;
mod settings;
mod settings_prefs;
mod theme;
mod ui_kit;
mod update;
mod windows;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::window::{
    MonitorSelection, PresentMode, WindowMode, WindowPosition, WindowRef, WindowResolution,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use janscope_core::protocol::{ClientRole, DEFAULT_PORT};

#[derive(Resource, Clone, Copy)]
pub struct Role(pub ClientRole);

#[derive(Clone, Debug)]
struct Args {
    server: String,
    code: String,
    role: ClientRole,
    demo: bool,
    screenshot: Option<String>,
    fullscreen: bool,
    monitor: Option<usize>,
    output_borderless: bool,
}

fn usage() -> ! {
    eprintln!(
        "janscope-client {}\n\n使い方: janscope-client --server <IP:ポート> --code <接続コード> [オプション]\n\
         \x20 --role broadcast|input|player  ロール（既定 broadcast）\n\
         \x20 --fullscreen                   選手用画面を全画面にする\n\
         \x20 --monitor <番号>                全画面にするモニター（0から）\n\
         \x20 --output-borderless            出力ウィンドウを枠なしで開く（全画面はツールバーのボタンで）",
        janscope_core::VERSION
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args {
        server: format!("127.0.0.1:{DEFAULT_PORT}"),
        code: String::new(),
        role: ClientRole::Broadcast,
        demo: false,
        screenshot: None,
        fullscreen: false,
        monitor: None,
        output_borderless: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--server" => a.server = it.next().unwrap_or_else(|| usage()),
            "--code" => a.code = it.next().unwrap_or_else(|| usage()),
            "--role" => {
                a.role = match it.next().as_deref() {
                    Some("broadcast") => ClientRole::Broadcast,
                    Some("input") => ClientRole::Input,
                    Some("player") => ClientRole::PlayerScreen,
                    _ => usage(),
                }
            }
            "--demo" => a.demo = true,
            "--screenshot" => a.screenshot = Some(it.next().unwrap_or_else(|| usage())),
            "--fullscreen" => a.fullscreen = true,
            "--monitor" => a.monitor = it.next().and_then(|v| v.parse().ok()),
            // 以前の、枠を付けるオプション（今は既定で枠あり）
            "--output-decorations" => {}
            "--output-borderless" => a.output_borderless = true,
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }
    if a.port_missing() {
        a.server = format!("{}:{DEFAULT_PORT}", a.server);
    }
    a
}

impl Args {
    fn port_missing(&self) -> bool {
        !self.server.contains(':')
    }
}

fn main() {
    let args = parse_args();
    let role = args.role;
    let player = role == ClientRole::PlayerScreen;
    let primary = if player {
        let mode = if args.fullscreen {
            WindowMode::BorderlessFullscreen(
                args.monitor
                    .map(MonitorSelection::Index)
                    .unwrap_or(MonitorSelection::Current),
            )
        } else {
            WindowMode::Windowed
        };
        Window {
            title: "JanScope 選手用画面".into(),
            resolution: WindowResolution::new(1920, 1080).with_scale_factor_override(1.0),
            mode,
            present_mode: PresentMode::AutoVsync,
            ..default()
        }
    } else {
        Window {
            title: format!("JanScope 操作（{}）", role.name_ja()),
            resolution: WindowResolution::new(1500, 980),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(primary),
                ..default()
            })
            .set(AssetPlugin {
                file_path: assets::assets_dir().to_string_lossy().into_owned(),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(Role(role))
    .init_resource::<theme::Theme>()
    .init_resource::<net::Table>()
    .init_resource::<program::Program>()
    .init_resource::<program::Delayed>()
    .init_resource::<scenes::Scenes>()
    .init_resource::<replay::Replay>()
    .init_resource::<output::ScoreAnim>()
    .insert_resource(config::ClientConfig::load())
    .init_resource::<output::WinShow>()
    .init_resource::<output::OutputDirty>()
    .init_resource::<player_screen::PlayerDirty>()
    .add_systems(Startup, load_assets)
    .add_systems(PreUpdate, (theme::sync_theme, sync_tiles))
    .add_systems(Update, windows::apply_window_ctl);

    if args.demo {
        // 見本の状態を表示する。操作ウィンドウも描けるよう、つながらない宛先の接続を置いておく（状態は取り込まない）
        app.insert_resource(net::Net::connect("127.0.0.1:9", "", role));
        app.add_systems(Startup, demo::install);
    } else {
        app.insert_resource(net::Net::connect(&args.server, &args.code, role));
        app.add_systems(PreUpdate, net::poll_net);
    }

    if player {
        app.add_systems(Startup, setup_player_window).add_systems(
            Update,
            (reload_config_file, player_screen::rebuild_player_screen).chain(),
        );
    } else {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<operator::OperatorUi>()
            .add_systems(Startup, face::setup_face)
            .add_systems(
                Startup,
                move |c: Commands, s: ResMut<EguiGlobalSettings>| {
                    setup_operator_windows(c, s, role, !args.output_borderless)
                },
            )
            .add_systems(
                Update,
                (
                    program::release,
                    replay::tick_replay,
                    scenes::update_scenes,
                    output::handle_overlays,
                    face::face_update,
                    output::rebuild_output,
                )
                    .chain(),
            );
        app.insert_resource(settings::Library::load())
            .init_resource::<settings::SettingsUi>()
            .add_systems(Startup, update::startup_check)
            .add_systems(
                EguiPrimaryContextPass,
                (operator::operator_ui, settings::settings_ui).chain(),
            );
    }
    if let Some(path) = args.screenshot.clone() {
        app.insert_resource(demo::ScreenshotTask {
            path,
            frames: 0,
            done: false,
        });
        app.add_systems(Update, demo::take_screenshot);
    }
    app.run();
}

fn load_assets(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(assets::load_fonts(&server));
}

/// 牌画像を読み込む（テーマタブで通常版／黒版を切り替えたら読み直す）。
fn sync_tiles(
    mut commands: Commands,
    cfg: Res<config::ClientConfig>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Option<prefs::TileThemeChoice>>,
) {
    let want = cfg.prefs.theme.tiles;
    if *last == Some(want) {
        return;
    }
    *last = Some(want);
    let theme = match want {
        prefs::TileThemeChoice::Regular => assets::TileTheme::Regular,
        prefs::TileThemeChoice::Black => assets::TileTheme::Black,
    };
    match assets::load_tile_images(&mut images, theme) {
        Ok(t) => commands.insert_resource(t),
        Err(e) => error!("牌画像を読み込めません: {e}"),
    }
}

/// 選手用画面は別のプロセスなので、設定ファイルが変わったら読み直す（2秒ごとに確認）。
fn reload_config_file(
    time: Res<Time>,
    mut cfg: ResMut<config::ClientConfig>,
    mut next: Local<f64>,
    mut mtime: Local<Option<std::time::SystemTime>>,
) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 2.0;
    let m = std::fs::metadata(config::config_path())
        .and_then(|m| m.modified())
        .ok();
    if m.is_some() && m != *mtime {
        if mtime.is_some() {
            *cfg = config::ClientConfig::load();
        }
        *mtime = m;
    }
}

fn setup_player_window(mut commands: Commands) {
    commands.spawn((Camera2d, player_screen::PlayerCamera));
}

fn setup_operator_windows(
    mut commands: Commands,
    mut egui: ResMut<EguiGlobalSettings>,
    role: ClientRole,
    decorations: bool,
) {
    egui.auto_create_primary_context = false;
    commands.spawn((Camera2d, PrimaryEguiContext));
    if role == ClientRole::Broadcast {
        let out = commands
            .spawn(Window {
                title: "JanScope 出力".into(),
                resolution: WindowResolution::new(1920, 1080).with_scale_factor_override(1.0),
                // 起動時は普通のウィンドウ。全画面は操作ウィンドウのツールバー・メニューから
                decorations,
                resizable: false,
                position: WindowPosition::Automatic,
                present_mode: PresentMode::AutoVsync,
                ..default()
            })
            .insert(windows::OutputWindow)
            .id();
        commands.spawn((
            Camera2d,
            Camera {
                order: 1,
                ..default()
            },
            RenderTarget::Window(WindowRef::Entity(out)),
            output::OutputCamera,
        ));
    }
}

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
mod config;
mod demo;
mod face;
mod net;
mod operator;
mod output;
mod player_screen;
mod program;
mod theme;
mod ui_kit;

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
    output_decorations: bool,
}

fn usage() -> ! {
    eprintln!(
        "janscope-client {}\n\n使い方: janscope-client --server <IP:ポート> --code <接続コード> [オプション]\n\
         \x20 --role broadcast|input|player  ロール（既定 broadcast）\n\
         \x20 --fullscreen                   選手用画面を全画面にする\n\
         \x20 --monitor <番号>                全画面にするモニター（0から）\n\
         \x20 --output-decorations           出力ウィンドウに枠を付ける（確認用）",
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
        output_decorations: false,
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
            "--output-decorations" => a.output_decorations = true,
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
    .insert_resource(config::ClientConfig::load())
    .init_resource::<output::WinShow>()
    .init_resource::<output::OutputDirty>()
    .init_resource::<player_screen::PlayerScreenConfig>()
    .init_resource::<player_screen::PlayerDirty>()
    .add_systems(Startup, load_assets);

    if args.demo {
        // 見本の状態を表示する。操作ウィンドウも描けるよう、つながらない宛先の接続を置いておく（状態は取り込まない）
        app.insert_resource(net::Net::connect("127.0.0.1:9", "", role));
        app.add_systems(Startup, demo::install);
    } else {
        app.insert_resource(net::Net::connect(&args.server, &args.code, role));
        app.add_systems(PreUpdate, net::poll_net);
    }

    if player {
        app.add_systems(Startup, setup_player_window)
            .add_systems(Update, player_screen::rebuild_player_screen);
    } else {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<operator::OperatorUi>()
            .add_systems(Startup, face::setup_face)
            .add_systems(
                Startup,
                move |c: Commands, s: ResMut<EguiGlobalSettings>| {
                    setup_operator_windows(c, s, role, args.output_decorations)
                },
            )
            .add_systems(
                Update,
                (
                    program::release,
                    output::handle_overlays,
                    face::face_update,
                    output::rebuild_output,
                )
                    .chain(),
            );
        app.add_systems(EguiPrimaryContextPass, operator::operator_ui);
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

fn load_assets(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    match assets::load_tile_images(&mut images, assets::TileTheme::Regular) {
        Ok(t) => {
            commands.insert_resource(t);
        }
        Err(e) => error!("牌画像を読み込めません: {e}"),
    }
    commands.insert_resource(assets::load_fonts(&server));
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
                decorations,
                resizable: false,
                position: WindowPosition::At(IVec2::new(0, 0)),
                present_mode: PresentMode::AutoVsync,
                ..default()
            })
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

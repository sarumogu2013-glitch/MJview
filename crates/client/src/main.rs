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
mod tile_input;
mod ui_kit;
mod update;
mod windows;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode, WindowPosition, WindowResolution};
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
    /// 機能の確認用: ダミー対局データ入りのサーバーを中で動かしてつなぐ
    dummy_server: bool,
    /// --dummy-file: 取り込んだ対局（雀魂の牌譜など）をダミー対局データにする
    dummy_file: Option<String>,
    screenshot: Option<String>,
    fullscreen: bool,
    monitor: Option<usize>,
    output_borderless: bool,
    /// ランチャー（JanScope.exe）のウィンドウに操作ウィンドウを埋め込む
    embed: bool,
}

fn usage() -> ! {
    eprintln!(
        "janscope-client {}\n\n使い方: janscope-client --server <IP:ポート> --code <接続コード> [オプション]\n\
         \x20 --role broadcast|input|player  ロール（既定 broadcast）\n\
         \x20 --fullscreen                   選手用画面を全画面にする\n\
         \x20 --monitor <番号>                全画面にするモニター（0から）\n\
         \x20 --output-borderless            出力ウィンドウを枠なしで開く（全画面はツールバーのボタンで）\n\
         \x20 --embed                        操作ウィンドウをランチャー（JanScope.exe）に埋め込む（ランチャーが付ける）\n\
         \x20 --dummy-server                 機能の確認用: ダミー対局データ入りのサーバーを中で動かしてつなぐ（--server・--code は不要）\n\
         \x20 --dummy-file <ファイル>         機能の確認用: 取り込んだ対局（雀魂の牌譜など）で --dummy-server と同じことをする",
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
        dummy_server: false,
        dummy_file: None,
        screenshot: None,
        fullscreen: false,
        monitor: None,
        output_borderless: false,
        embed: false,
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
            "--dummy-server" => a.dummy_server = true,
            "--dummy-file" => {
                a.dummy_server = true;
                a.dummy_file = Some(it.next().unwrap_or_else(|| usage()));
            }
            "--screenshot" => a.screenshot = Some(it.next().unwrap_or_else(|| usage())),
            "--fullscreen" => a.fullscreen = true,
            "--monitor" => a.monitor = it.next().and_then(|v| v.parse().ok()),
            // 以前の、枠を付けるオプション（今は既定で枠あり）
            "--output-decorations" => {}
            "--output-borderless" => a.output_borderless = true,
            "--embed" => a.embed = true,
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
            // 起動時はウィンドウ（最大化ボタンで最大化、F11 で全画面）。中身は 1920×1080 の配置を窓の大きさに合わせて縮める
            resolution: WindowResolution::new(1280, 720),
            resizable: true,
            mode,
            position: args
                .monitor
                .map(|m| WindowPosition::Centered(MonitorSelection::Index(m)))
                .unwrap_or(WindowPosition::Automatic),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }
    } else {
        Window {
            // ランチャーはこの題名（「JanScope 操作」で始まる）で操作ウィンドウを見つけて埋め込む
            title: if args.dummy_server {
                format!("JanScope 操作（{}・ダミーサーバー）", role.name_ja())
            } else {
                format!("JanScope 操作（{}）", role.name_ja())
            },
            resolution: WindowResolution::new(1500, 980),
            present_mode: PresentMode::AutoVsync,
            // 埋め込むときは枠なしで画面の外に出し、ランチャーが中に入れる
            decorations: !args.embed,
            position: if args.embed {
                WindowPosition::At(IVec2::new(-30000, -30000))
            } else {
                WindowPosition::Automatic
            },
            ..default()
        }
    };
    windows::EMBEDDED.store(args.embed, std::sync::atomic::Ordering::Relaxed);
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
    .add_systems(
        Update,
        (
            windows::apply_window_ctl,
            windows::set_window_icons,
            windows::fit_1080,
        ),
    );

    if args.dummy_server {
        // ダミーサーバー: 中で本物のサーバー（ダミー対局データ入り）を動かすので、操作・取り消しなど全部試せる
        match start_dummy_server(args.dummy_file.clone()) {
            Ok((addr, code)) => {
                app.insert_resource(net::Net::connect(&addr, &code, role));
            }
            Err(e) => {
                eprintln!("ダミーサーバーを起動できません: {e}");
                app.insert_resource(net::Net::connect("127.0.0.1:9", "", role));
            }
        }
        app.add_systems(PreUpdate, net::poll_net);
    } else if args.demo {
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
                    output::cleanup_closed_output,
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

/// --dummy-server: ダミー対局データ入りのサーバーを、このプロセスの中（別スレッド）で動かす。宛先と接続コードを返す
fn start_dummy_server(file: Option<String>) -> Result<(String, String), String> {
    let actions = match &file {
        Some(p) => {
            let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            janscope_core::dummy::load(&t)?.actions
        }
        None => janscope_core::dummy::actions(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("janscope-dummy-server".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            rt.block_on(async move {
                let code = janscope_server::new_code();
                let cfg = janscope_server::ServerConfig {
                    bind: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
                    code: code.clone(),
                    // 本番のログ・大会通算と混ざらないよう、一時フォルダに置く
                    data_dir: std::env::temp_dir().join("janscope-dummy"),
                    resume: None,
                    dummy: Some(actions),
                };
                match janscope_server::start(cfg).await {
                    Ok(running) => {
                        let _ = tx.send(Ok((running.addr.to_string(), code)));
                        // アプリが終わるまで動かし続ける
                        std::future::pending::<()>().await;
                        drop(running);
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            });
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|e| e.to_string())?
}

fn setup_player_window(mut commands: Commands) {
    commands.spawn((Camera2d, player_screen::PlayerCamera, windows::Fit1080));
}

fn setup_operator_windows(
    mut commands: Commands,
    mut egui: ResMut<EguiGlobalSettings>,
    role: ClientRole,
    decorations: bool,
) {
    egui.auto_create_primary_context = false;
    commands.spawn((Camera2d, PrimaryEguiContext));
    commands.insert_resource(windows::OutputDecor(decorations));
    if role == ClientRole::Broadcast {
        windows::spawn_output(&mut commands, decorations);
    }
}

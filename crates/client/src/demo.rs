//! 開発用: サーバーなしで見本の状態を表示する（`--demo`）と、出力の画面を保存する（`--screenshot`）。
//! 見本の内容は環境変数 JANSCOPE_DEMO で選ぶ: `hand`（既定、局の途中）／ `win`（アガリ演出）／ `result`（終局）

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;
use janscope_core::engine::apply;
use janscope_core::rules::GameConfig;
use janscope_core::scoring::{Meld, MeldKind, WinInput};
use janscope_core::state::{Action, Player, TableState};
use janscope_core::tile::{SeatId, parse_tiles};
use janscope_core::view::FullView;

use crate::net::{Link, Table};
use crate::output::OutputCamera;

pub fn demo_state(kind: &str) -> (TableState, Vec<janscope_core::engine::Overlay>) {
    let mut s = TableState::default();
    let players = [
        Player {
            name: "山田 太郎".into(),
            affiliation: Some("東京支部".into()),
        },
        Player {
            name: "佐藤 花子".into(),
            affiliation: Some("大阪支部".into()),
        },
        Player {
            name: "鈴木 一郎".into(),
            affiliation: None,
        },
        Player {
            name: "高橋 美咲".into(),
            affiliation: Some("名古屋支部".into()),
        },
    ];
    let mut ov = vec![];
    let mut act = |s: &mut TableState, a: Action| {
        ov = apply(s, &a).expect("demo");
    };
    act(
        &mut s,
        Action::StartGame {
            config: GameConfig::preset_standard(),
            players,
            chiicha: SeatId(0),
        },
    );
    act(&mut s, Action::StartHand { dice: vec![(3, 5)] });
    act(
        &mut s,
        Action::Win {
            wins: vec![win(
                SeatId(2),
                Some(SeatId(0)),
                "234567m345p678s99p",
                "2m",
                vec![],
                false,
            )],
        },
    );
    act(&mut s, Action::StartHand { dice: vec![(2, 6)] });
    act(
        &mut s,
        Action::SetDora {
            tiles: parse_tiles("3p").unwrap(),
        },
    );
    for _ in 0..28 {
        act(&mut s, Action::WallDecrement);
    }
    act(&mut s, Action::Riichi { seat: SeatId(3) });
    if kind == "hand" {
        s.seats[1].melds = vec![Meld {
            kind: MeldKind::Pon,
            tiles: parse_tiles("777z").unwrap(),
            from: Some(SeatId(0)),
        }];
        return (s, ov);
    }
    let mut w = win(SeatId(3), None, "234m340p678s11p", "8s", vec![], true);
    w.melds = vec![Meld {
        kind: MeldKind::Ankan,
        tiles: parse_tiles("5555z").unwrap(),
        from: None,
    }];
    w.ura_indicators = parse_tiles("9p").unwrap();
    act(&mut s, Action::Win { wins: vec![w] });
    if kind == "result" {
        act(&mut s, Action::EndGame);
    }
    (s, ov)
}

fn win(
    winner: SeatId,
    loser: Option<SeatId>,
    closed: &str,
    wt: &str,
    melds: Vec<Meld>,
    riichi: bool,
) -> WinInput {
    WinInput {
        winner,
        loser,
        closed: parse_tiles(closed).unwrap(),
        melds,
        win_tile: wt.parse().unwrap(),
        riichi,
        double_riichi: false,
        ippatsu: false,
        haitei: false,
        rinshan: false,
        chankan: false,
        tenhou: false,
        ura_indicators: vec![],
        pao: None,
    }
}

pub fn install(
    mut table: ResMut<Table>,
    mut program: ResMut<crate::program::Program>,
    mut scenes: ResMut<crate::scenes::Scenes>,
    settings: Option<ResMut<crate::settings::SettingsUi>>,
) {
    // JANSCOPE_SETTINGS=rules で設定ウィンドウのルールタブを開く（確認用）
    if let (Some(mut st), Ok("rules")) = (settings, std::env::var("JANSCOPE_SETTINGS").as_deref()) {
        st.open_rules(janscope_core::rules::GameConfig::preset_standard());
    }
    // JANSCOPE_SCENE で最初に放送するシーンを選ぶ（確認用）
    use janscope_core::scene::SceneKind;
    scenes.sw.program = match std::env::var("JANSCOPE_SCENE").as_deref() {
        Ok("intro") => SceneKind::PlayerIntro,
        Ok("waiting") => SceneKind::Waiting,
        Ok("final") => SceneKind::FinalResult,
        Ok("standings") => SceneKind::Standings,
        Ok("fullscreen") => SceneKind::Fullscreen(janscope_core::tile::SeatId(3)),
        _ => SceneKind::Game,
    };
    if let Ok(t) = std::env::var("JANSCOPE_TELOP") {
        scenes.telop = Some(t);
    }
    let kind = std::env::var("JANSCOPE_DEMO").unwrap_or_else(|_| "hand".into());
    let (s, ov) = demo_state(&kind);
    table.link = Link::Connected;
    table.seq = 1;
    table.full = Some(FullView {
        state: s,
        can_undo: true,
        can_redo: false,
        tournament: demo_tournament(&kind),
    });
    program.full = table.full.clone();
    program.seq = 1;
    program.overlays = ov.into_iter().map(|o| (0, o)).collect();
    table.changed = true;
}

#[derive(Resource)]
pub struct ScreenshotTask {
    pub path: String,
    pub frames: u32,
    pub done: bool,
}

/// 何フレームか待って（フォント・画像の読み込み待ち）出力ウィンドウを保存し、保存できたら終了する。
pub fn take_screenshot(
    mut commands: Commands,
    mut task: ResMut<ScreenshotTask>,
    cams: Query<&bevy::camera::RenderTarget, With<OutputCamera>>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    task.frames += 1;
    // JANSCOPE_SHOT_FRAMES で保存までのフレーム数を変えられる（映像の入力待ちなど）
    let at: u32 = std::env::var("JANSCOPE_SHOT_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    if !task.done && task.frames == at {
        let target = cams.iter().next().and_then(|t| match t {
            bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(e)) => Some(*e),
            _ => None,
        });
        // JANSCOPE_SHOT=primary なら操作ウィンドウ（主ウィンドウ）を保存する
        let want_primary = std::env::var("JANSCOPE_SHOT").is_ok_and(|v| v == "primary");
        let win = if want_primary {
            primary.single().ok()
        } else {
            target.or_else(|| primary.single().ok())
        };
        if let Some(w) = win {
            commands
                .spawn(Screenshot::window(w))
                .observe(save_to_disk(task.path.clone()));
            task.done = true;
        }
    }
    if task.done && task.frames > at && std::path::Path::new(&task.path).exists() {
        exit.write(AppExit::Success);
    }
    if task.frames > at + 600 {
        error!("画面を保存できませんでした");
        exit.write(AppExit::error());
    }
}

/// 見本の大会通算（result の見本では、終局した半荘と架空の2半荘を入れる）。
fn demo_tournament(kind: &str) -> janscope_core::tournament::Tournament {
    let mut t = janscope_core::tournament::Tournament {
        name: "見本大会".into(),
        games: vec![],
    };
    if kind != "result" {
        return t;
    }
    let (s, _) = demo_state("result");
    t.sync_game("demo#1", &s, 1);
    for (k, d) in [
        ("demo#2", [12_000, -3_000, 1_000, -10_000]),
        ("demo#3", [-8_000, 20_000, -4_000, -8_000]),
    ] {
        let mut g = TableState::default();
        let players = std::array::from_fn(|i| s.seats[i].player.clone());
        apply(
            &mut g,
            &Action::StartGame {
                config: GameConfig::preset_standard(),
                players,
                chiicha: SeatId(0),
            },
        )
        .unwrap();
        apply(
            &mut g,
            &Action::ManualAdjust {
                deltas: d,
                reason: "見本".into(),
            },
        )
        .unwrap();
        apply(&mut g, &Action::EndGame).unwrap();
        t.sync_game(k, &g, 2);
    }
    t
}

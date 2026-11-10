//! 操作ウィンドウ（設計書 4.4・4.5）。オペレーター・入力担当が使う。egui で描く。
//!
//! 操作ウィンドウは常にライブ（遅延なし）。アガリは「プレビュー → 放送に出す」の2段階で、
//! 計算結果を確かめてから初めて出力に出る。

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiTextureHandle, egui};
use janscope_core::rules::GameConfig;
use janscope_core::scoring::{Meld, MeldKind, WinInput, fmt_pts};
use janscope_core::state::{AbortiveKind, Action, DrawKind, KanKind, Phase, Player, TableState};
use janscope_core::tile::{SeatId, Tile, Wind};

use crate::assets::TileImages;
use crate::config::{ClientConfig, FaceOutput};
use crate::face::FaceRuntime;
use crate::net::{Link, Net, Table};
use crate::output::WinShow;
use janscope_core::face::{FaceMode, IdleMode, ManualBehavior};
use janscope_video::RtmpMode;
use janscope_video::StreamStatus;
use janscope_video::obs::{ObsStatus, ObsSwitchMode};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PaletteTarget {
    #[default]
    Closed,
    Meld,
    Ura,
    Dora,
    KanDora,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DrawChoice {
    Exhaustive,
    Nagashi,
    Abortive(AbortiveKind),
}

#[derive(Resource)]
pub struct OperatorUi {
    fonts_set: bool,
    tex: HashMap<String, egui::TextureId>,
    // 半荘開始
    presets: Vec<GameConfig>,
    /// 選手の登録（設定の選手タブ）
    registry: Vec<Player>,
    preset: usize,
    names: [String; 4],
    affiliations: [String; 4],
    chiicha: u8,
    // 局開始
    dice: [[u8; 2]; 2],
    dice_second: bool,
    wall_direct: u32,
    // 槓
    kan_seat: u8,
    kan_kind: KanKind,
    kan_dora: Option<Tile>,
    // ドラの入力
    dora_edit: Vec<Tile>,
    /// 最後に見た卓のドラ表示牌（変わったら入力欄を合わせる）
    dora_seen: Vec<Tile>,
    // 流局
    tenpai: [bool; 4],
    nagashi: [bool; 4],
    draw_choice: DrawChoice,
    // 手動修正
    adjust: [i32; 4],
    adjust_reason: String,
    round_fix: (Wind, u8, u32, u32),
    // アガリ入力
    win: WinInput,
    target: PaletteTarget,
    meld_kind: MeldKind,
    meld_buf: Vec<Tile>,
    pending: Vec<WinInput>,
    preview_id: Option<u64>,
    confirm_end: bool,
    // 顔カメラ・OBS・切替の設定（編集中の下書き）
    cfg_draft: Option<ClientConfig>,
    cfg_msg: Option<String>,
    cfg_confirm_delay: bool,
    telop_edit: String,
    fullscreen_seat: u8,
    tour_name: Option<String>,
    csv_dir: String,
    csv_msg: Option<String>,
    remove_key: Option<String>,
    cam: crate::cameras::CameraUi,
    /// ウィンドウの全画面切替（メニューバー・ツールバー）
    pub windows: crate::windows::WindowCtl,
    /// 手牌・河の手入力（テンキー）
    tiles: crate::tile_input::TileInput,
}

impl Default for OperatorUi {
    fn default() -> Self {
        OperatorUi {
            fonts_set: false,
            tex: HashMap::new(),
            presets: GameConfig::presets(),
            registry: vec![],
            preset: 0,
            names: ["選手A", "選手B", "選手C", "選手D"].map(String::from),
            affiliations: Default::default(),
            chiicha: 0,
            dice: [[1, 1], [1, 1]],
            dice_second: false,
            wall_direct: 0,
            kan_seat: 0,
            kan_kind: KanKind::Ankan,
            kan_dora: None,
            dora_edit: vec![],
            dora_seen: vec![],
            tenpai: [false; 4],
            nagashi: [false; 4],
            draw_choice: DrawChoice::Exhaustive,
            adjust: [0; 4],
            adjust_reason: String::new(),
            round_fix: (Wind::East, 1, 0, 0),
            win: empty_win(),
            target: PaletteTarget::Closed,
            meld_kind: MeldKind::Pon,
            meld_buf: vec![],
            pending: vec![],
            preview_id: None,
            confirm_end: false,
            cfg_draft: None,
            cfg_msg: None,
            cfg_confirm_delay: false,
            telop_edit: String::new(),
            fullscreen_seat: 0,
            tour_name: None,
            csv_dir: ".".into(),
            csv_msg: None,
            remove_key: None,
            cam: Default::default(),
            windows: Default::default(),
            tiles: Default::default(),
        }
    }
}

fn empty_win() -> WinInput {
    WinInput {
        winner: SeatId(0),
        loser: Some(SeatId(1)),
        closed: vec![],
        melds: vec![],
        win_tile: Tile::new(0),
        riichi: false,
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

/// 日本語フォント（Noto Sans JP）を egui に入れる。
fn set_fonts(ctx: &egui::Context) {
    let path = crate::assets::assets_dir().join("fonts/NotoSansJP-Regular.otf");
    let Ok(bytes) = std::fs::read(&path) else {
        error!(
            "フォントが読めません: {}（assets フォルダを実行ファイルの隣に置いてください）",
            path.display()
        );
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "noto_jp".into(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(fam)
            .or_default()
            .insert(0, "noto_jp".into());
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|s| {
        s.text_styles.iter_mut().for_each(|(_, f)| f.size *= 1.15);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}

fn seat_name(s: &TableState, seat: SeatId) -> String {
    format!("{} {}", seat.label(), s.seats[seat.index()].player.name)
}

#[allow(clippy::too_many_arguments)]
pub fn operator_ui(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<OperatorUi>,
    mut table: ResMut<Table>,
    net: Res<Net>,
    tiles: Option<Res<TileImages>>,
    mut face: ResMut<FaceRuntime>,
    mut cfg: ResMut<ClientConfig>,
    time: Res<Time>,
    mut win_show: ResMut<WinShow>,
    keys: Res<ButtonInput<KeyCode>>,
    role: Res<crate::Role>,
    mut scenes: ResMut<crate::scenes::Scenes>,
    mut settings: ResMut<crate::settings::SettingsUi>,
    lib: Res<crate::settings::Library>,
    mut replay: ResMut<crate::replay::Replay>,
    mut sandbox: ResMut<crate::sandbox::SandboxUi>,
) -> Result {
    let Some(tiles) = tiles else { return Ok(()) };
    if tiles.is_changed() {
        ui_state.tex.clear();
    }
    if ui_state.tex.is_empty() {
        for t in palette_tiles() {
            let id = contexts.add_image(EguiTextureHandle::Strong(tiles.get(t)));
            ui_state.tex.insert(t.to_string(), id);
        }
        let id = contexts.add_image(EguiTextureHandle::Strong(tiles.back.clone()));
        ui_state.tex.insert("back".into(), id);
        // 顔カメラのプレビュー用
        for (i, h) in face.images.iter().enumerate() {
            let id = contexts.add_image(EguiTextureHandle::Strong(h.clone()));
            ui_state.tex.insert(format!("face{i}"), id);
        }
    }
    let ctx = contexts.ctx_mut()?;
    if !ui_state.fonts_set {
        set_fonts(ctx);
        ui_state.fonts_set = true;
    }
    let u = &mut *ui_state;
    u.presets = lib.all_presets();
    if u.preset >= u.presets.len() {
        u.preset = 0;
    }
    u.registry = lib.players.clone();

    // 専用キー（文字入力中は無効）
    // 専用キー（設定のショートカットタブで変更できる。文字入力中は無効）
    if !ctx.egui_wants_keyboard_input() {
        use crate::prefs::pressed;
        let sc = cfg.prefs.shortcuts.clone();
        let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        // 手牌・河の手入力中は、テンキーはそちらで使う（残り牌数はツモで自動に減る）
        let tile_mode = cfg.prefs.tile_mode;
        let tiles_on = tile_mode != crate::tile_input::TileMode::Off
            && table
                .full
                .as_ref()
                .is_some_and(|f| f.state.phase == Phase::InHand);
        let numpad = |k: &str| tiles_on && k.starts_with("Numpad");
        if tiles_on {
            let st = table.full.as_ref().map(|f| f.state.clone()).unwrap();
            for k in crate::tile_input::NumKey::from_keys(&keys) {
                if let Some(o) = u.tiles.key(k, &st, tile_mode, &*net) {
                    tile_outcome(u, o, &net, &mut table);
                }
            }
        }
        if pressed(&keys, &sc.wall_down) && !numpad(&sc.wall_down) {
            net.act(Action::WallDecrement);
        }
        if pressed(&keys, &sc.wall_up) && !numpad(&sc.wall_up) {
            net.act(Action::WallIncrement);
        }
        if ctrl && pressed(&keys, &sc.undo) {
            net.undo();
        }
        if ctrl && pressed(&keys, &sc.redo) {
            net.redo();
        }
        for (i, k) in sc.face.iter().enumerate() {
            if pressed(&keys, k) {
                face.sw.manual(SeatId(i as u8), time.elapsed_secs_f64());
            }
        }
        if scenes.sw.program == janscope_core::scene::SceneKind::Replay && replay.paifu.is_some() {
            if pressed(&keys, &sc.replay_forward) {
                replay.step_by(1);
            }
            if pressed(&keys, &sc.replay_back) {
                replay.step_by(-1);
            }
            if pressed(&keys, &sc.replay_play) {
                replay.playing = !replay.playing;
            }
        }
        if pressed(&keys, &sc.face_auto) {
            face.sw.auto(time.elapsed_secs_f64());
        }
    }

    // egui 0.36: パネルは画面全体の Ui の中に置く
    let mut root = egui::Ui::new(
        ctx.clone(),
        "operator_root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    // いちばん上: メニューバー（ランチャーに埋め込まれているときは、ランチャーのメニューを使うので出さない）
    if !crate::windows::embedded() {
        egui::Panel::top("menu").show(&mut root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("ファイル", |ui| {
                    if ui.button("終了").clicked() {
                        u.windows.request(crate::windows::WinReq::Quit);
                        ui.close();
                    }
                });
                ui.menu_button("表示", |ui| u.windows.view_menu(ui));
                ui.menu_button("設定", |ui| {
                    if ui.button("設定ウィンドウ…").clicked() {
                        settings.open = true;
                        ui.close();
                    }
                    if ui.button("ルール…").clicked() {
                        settings.open_tab("ルール");
                        ui.close();
                    }
                    if ui.button("選手…").clicked() {
                        settings.open_tab("選手");
                        ui.close();
                    }
                });
                ui.menu_button("ヘルプ", |ui| {
                    if ui.button("更新の確認…").clicked() {
                        settings.open_tab("更新");
                        ui.close();
                    }
                    if ui.button("情報…").clicked() {
                        settings.open_tab("情報");
                        ui.close();
                    }
                });
            });
        });
    }
    // その下: ツールバー（接続状態・Undo/Redo・設定・全画面・バージョン）
    egui::Panel::top("top").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            match &table.link {
                Link::Connected => ui.colored_label(egui::Color32::LIGHT_GREEN, "● 接続中"),
                Link::Connecting => ui.colored_label(egui::Color32::YELLOW, "● 接続しています…"),
                Link::Disconnected(r) => ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new(format!("■ 切断中（再接続しています）: {r}"))
                        .size(22.0)
                        .strong(),
                ),
                Link::Rejected(r) => ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new(format!("■ 接続を拒否されました: {r}"))
                        .size(22.0)
                        .strong(),
                ),
            };
            ui.separator();
            ui.label(format!("ロール: {}", role.0.name_ja()));
            if table.full.as_ref().is_some_and(|f| f.demo) {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 80),
                    egui::RichText::new("■ デモモード（ダミー履歴）").strong(),
                )
                .on_hover_text("サーバーがダミー履歴で動いています。ランチャーの「デモモード」をもう一度押すと実戦モードに戻ります");
            }
            if let Some(by) = table.last_by.filter(|b| *b != role.0) {
                ui.colored_label(
                    egui::Color32::LIGHT_BLUE,
                    format!("（最後の操作: {}）", by.name_ja()),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!(
                    "JanScope {}  遅延 {}ms",
                    janscope_core::VERSION,
                    table.rtt_ms / 2
                ));
                if let Some(v) = settings.update.available() {
                    if ui
                        .small_button(
                            egui::RichText::new(format!("新しい版 {v} あり"))
                                .color(egui::Color32::from_rgb(255, 210, 90)),
                        )
                        .on_hover_text(
                            "設定 →「更新」で入れ替えられます（配信中は入れ替えないこと）",
                        )
                        .clicked()
                    {
                        settings.open_tab("更新");
                    }
                }
            });
        });
        // 2段目: 操作（接続状態の文字が長くても押し出されないよう、1段にまとめる）
        ui.horizontal_wrapped(|ui| {
            let (cu, cr) = table
                .full
                .as_ref()
                .map(|f| (f.can_undo, f.can_redo))
                .unwrap_or((false, false));
            if ui
                .add_enabled(cu, egui::Button::new("取り消し (Ctrl+Z)"))
                .clicked()
            {
                net.undo();
            }
            if ui
                .add_enabled(cr, egui::Button::new("やり直し (Ctrl+Y)"))
                .clicked()
            {
                net.redo();
            }
            if ui.button("設定").clicked() {
                settings.open = !settings.open;
            }
            if ui
                .button("映像サンドボックス")
                .on_hover_text("メディアサーバーに届いた映像を並べて、選手顔・手牌確認・河/鳴き確認の枠を置きます")
                .clicked()
            {
                sandbox.open = !sandbox.open;
            }
            ui.separator();
            u.windows.toolbar(ui);
            if cfg.delay_secs > 0.0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 80),
                    format!(
                        "配信ディレイ {:.1}秒（出力ウィンドウは遅れて表示）",
                        cfg.delay_secs
                    ),
                );
            }
        });
        if let Some(e) = table.last_error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 140, 100), format!("注意: {e}"));
                if ui.small_button("閉じる").clicked() {
                    table.last_error = None;
                }
            });
        }
        for n in table.notices.clone() {
            ui.colored_label(egui::Color32::LIGHT_BLUE, n);
        }
    });

    let Some(state) = table.full.as_ref().map(|f| f.state.clone()) else {
        egui::CentralPanel::default().show(&mut root, |ui| {
            ui.heading("サーバーからの状態を待っています");
        });
        return Ok(());
    };

    // 左: 卓の状態
    egui::Panel::left("status")
        .default_size(360.0)
        .show(&mut root, |ui| {
            // 隠している欄を開くと下が見えなくなるので、左側はスクロールできるようにする
            egui::ScrollArea::vertical()
                .id_salt("status_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading(match state.phase {
                        Phase::Setup => "対局前".into(),
                        Phase::GameOver => "終局".into(),
                        _ => format!(
                            "{} {}本場 供託{}",
                            state.round_label(),
                            state.honba,
                            state.kyotaku
                        ),
                    });
                    egui::Grid::new("scores").striped(true).show(ui, |ui| {
                        for seat in SeatId::ALL {
                            let st = &state.seats[seat.index()];
                            ui.label(format!(
                                "{}{}",
                                state.seat_wind(seat).name_ja(),
                                if state.dealer() == seat { "親" } else { "" }
                            ));
                            ui.label(&st.player.name);
                            ui.label(egui::RichText::new(fmt_pts(st.score)).strong());
                            let d = state.last_deltas[seat.index()];
                            ui.label(if d != 0 {
                                crate::ui_kit::delta_text(d)
                            } else {
                                String::new()
                            });
                            ui.label(if st.riichi { "リーチ" } else { "" });
                            ui.end_row();
                        }
                    });
                    if state.phase == Phase::InHand {
                        ui.label(
                            egui::RichText::new(format!("残り {}", state.wall_remaining))
                                .size(28.0)
                                .strong(),
                        );
                    }
                    if let Some(fr) = &state.final_results {
                        ui.separator();
                        ui.heading("最終結果");
                        for r in fr {
                            ui.label(format!(
                                "{}位 {} {} ({:+.1})",
                                r.rank,
                                state.seats[r.seat.index()].player.name,
                                fmt_pts(r.score),
                                r.points
                            ));
                        }
                    }
                    ui.separator();
                    ui.label("選手画面（F1〜F4 で手動、F5 で自動）");
                    ui.horizontal(|ui| {
                        for seat in SeatId::ALL {
                            let sel =
                                face.current == seat && !matches!(face.sw.mode(), FaceMode::Auto);
                            if ui
                                .selectable_label(
                                    sel,
                                    format!(
                                        "F{} {}",
                                        seat.0 + 1,
                                        state.seats[seat.index()].player.name
                                    ),
                                )
                                .clicked()
                            {
                                face.sw.manual(seat, time.elapsed_secs_f64());
                            }
                        }
                        if ui
                            .selectable_label(matches!(face.sw.mode(), FaceMode::Auto), "自動")
                            .clicked()
                        {
                            face.sw.auto(time.elapsed_secs_f64());
                        }
                    });
                    let now = time.elapsed_secs_f64();
                    let mode = match face.sw.mode() {
                        FaceMode::Auto => "自動".to_string(),
                        FaceMode::TempManual { .. } => "一時手動".to_string(),
                        FaceMode::LockedManual { .. } => "手動固定".to_string(),
                    };
                    let rest = face
                        .sw
                        .remaining(now)
                        .map(|r| format!("、次の切替まで {:.0}秒", r))
                        .unwrap_or_default();
                    ui.label(format!(
                        "いま: {} {}（{mode}{rest}）",
                        face.current.label(),
                        state.seats[face.current.index()].player.name
                    ));
                    face_settings_panel(ui, u, &mut cfg, &mut face);
                    if !win_show.records.is_empty() && ui.button("アガリ演出を閉じる").clicked()
                    {
                        win_show.records.clear();
                    }
                    ui.separator();
                    if let Some(t) = table.full.as_ref().map(|f| f.tournament.clone()) {
                        tournament_panel(ui, u, &net, &t);
                    }
                    ui.separator();
                    ui.collapsing("局の履歴", |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                for h in state.history.iter().rev() {
                                    let what = match &h.outcome {
                                        janscope_core::state::HandOutcome::Win(ws) => ws
                                            .iter()
                                            .map(|w| {
                                                format!(
                                                    "{} {} {}",
                                                    state.seats[w.input.winner.index()].player.name,
                                                    w.score.han_fu_text(),
                                                    w.score.points_text()
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join(" / "),
                                        janscope_core::state::HandOutcome::Draw { .. } => {
                                            "流局".into()
                                        }
                                        janscope_core::state::HandOutcome::Adjust { reason } => {
                                            format!("調整: {reason}")
                                        }
                                    };
                                    ui.label(format!(
                                        "{}{}局{}本場 {}",
                                        h.wind.name_ja(),
                                        h.number,
                                        h.honba,
                                        what
                                    ));
                                }
                            });
                    });
                });
        });

    let now = time.elapsed_secs_f64();
    egui::CentralPanel::default().show(&mut root, |ui| {
        // 配信のメインは入力側（配信画面は、この画面のデータを写す）。シーン・牌譜再生も入力担当で操作する
        if role.0 != janscope_core::protocol::ClientRole::PlayerScreen {
            scene_panel(ui, u, &mut scenes, &state, now);
            replay_panel(ui, &mut replay, &cfg.prefs.replay);
            ui.separator();
        }
        egui::ScrollArea::vertical().show(ui, |ui| match state.phase {
            Phase::Setup | Phase::GameOver => setup_panel(ui, u, &net, &state),
            Phase::BetweenHands => {
                hand_start_panel(ui, u, &net, &state);
                ui.separator();
                fix_panel(ui, u, &net, &state);
            }
            Phase::InHand => {
                let mut mode = cfg.prefs.tile_mode;
                let (key, picked, changed) = {
                    let tex = &u.tex;
                    let tb = |ui: &mut egui::Ui, t: Tile, w: f32| tile_button_tex(ui, tex, t, w);
                    let tx = |t: Tile| tile_tex(tex, t);
                    u.tiles.panel(ui, &state, &mut mode, &tb, &tx)
                };
                if changed {
                    cfg.prefs.tile_mode = mode;
                    if let Err(e) = cfg.save() {
                        u.tiles.msg = Some(e);
                    }
                }
                let o = match (key, picked) {
                    (Some(k), _) => u.tiles.key(k, &state, mode, &*net),
                    (_, Some(t)) => u.tiles.tile(t, &state, mode, &*net),
                    _ => None,
                };
                if let Some(o) = o {
                    tile_outcome(u, o, &net, &mut table);
                }
                ui.separator();
                if mode == crate::tile_input::TileMode::Off {
                    in_hand_panel(ui, u, &net, &state);
                } else {
                    // 手牌・河を使うときは、残り牌数・リーチ・槓は自動。代わりに進行の履歴
                    let log = table
                        .full
                        .as_ref()
                        .map(|f| f.hand_log.clone())
                        .unwrap_or_default();
                    u.tiles.history(ui, &state, &log, &*net);
                    dora_panel(ui, u, &net, &state);
                }
                ui.separator();
                win_panel(ui, u, &net, &mut table, &state);
                ui.separator();
                draw_panel(ui, u, &net, &state);
                ui.separator();
                fix_panel(ui, u, &net, &state);
            }
        });
    });
    Ok(())
}

fn palette_tiles() -> Vec<Tile> {
    let mut v: Vec<Tile> = Tile::all_kinds().collect();
    v.extend(
        ["0m", "0p", "0s"]
            .iter()
            .map(|s| s.parse::<Tile>().unwrap()),
    );
    v
}

/// 牌パレット。クリックした牌を返す。
fn palette(ui: &mut egui::Ui, u: &OperatorUi, w: f32, red: bool) -> Option<Tile> {
    let mut clicked = None;
    let rows: [Vec<Tile>; 4] = [
        (0..9).map(Tile::new).collect(),
        (9..18).map(Tile::new).collect(),
        (18..27).map(Tile::new).collect(),
        (27..34).map(Tile::new).collect(),
    ];
    for (ri, row) in rows.iter().enumerate() {
        ui.horizontal(|ui| {
            for t in row {
                if tile_button(ui, u, *t, w).clicked() {
                    clicked = Some(*t);
                }
            }
            if red && ri < 3 {
                let r: Tile = ["0m", "0p", "0s"][ri].parse().unwrap();
                ui.add_space(8.0);
                if tile_button(ui, u, r, w).on_hover_text("赤5").clicked() {
                    clicked = Some(r);
                }
            }
        });
    }
    clicked
}

/// 手牌・河の手入力の結果（アガリ入力の欄を埋める・放送に出す）。
fn tile_outcome(u: &mut OperatorUi, o: crate::tile_input::Outcome, net: &Net, table: &mut Table) {
    use crate::tile_input::Outcome;
    match o {
        Outcome::Win(w) => {
            u.win = w;
            u.target = PaletteTarget::Closed;
            table.preview = None;
            u.preview_id = Some(net.preview(u.win.clone()));
        }
        Outcome::ConfirmWin => {
            let ok = matches!((u.preview_id, &table.preview), (Some(id), Some((rid, Ok(_)))) if id == *rid);
            if ok {
                let mut wins = std::mem::take(&mut u.pending);
                wins.push(u.win.clone());
                net.act(Action::Win { wins });
                u.win = empty_win();
                u.preview_id = None;
                u.tiles.sub = crate::tile_input::Sub::None;
            } else {
                u.tiles.msg = Some(
                    "まだ計算できていません（アガリ入力の欄で手牌・役を確かめてプレビューしてください）".into(),
                );
            }
        }
    }
}

fn tile_button(ui: &mut egui::Ui, u: &OperatorUi, t: Tile, w: f32) -> egui::Response {
    tile_button_tex(ui, &u.tex, t, w)
}

/// 牌の絵（不明の牌は裏）
fn tile_tex(tex: &HashMap<String, egui::TextureId>, t: Tile) -> Option<egui::TextureId> {
    if t.is_unknown() {
        tex.get("back").copied()
    } else {
        tex.get(&t.to_string()).copied()
    }
}

fn tile_button_tex(
    ui: &mut egui::Ui,
    tex: &HashMap<String, egui::TextureId>,
    t: Tile,
    w: f32,
) -> egui::Response {
    match tile_tex(tex, t).as_ref() {
        Some(id) => ui
            .add(egui::Button::image(egui::Image::new(
                egui::load::SizedTexture::new(*id, [w, w * 4.0 / 3.0]),
            )))
            .on_hover_text(t.name_ja()),
        None => ui.button(t.to_string()),
    }
}

/// 牌の列（クリックで取り除く）。
fn tile_row(ui: &mut egui::Ui, u: &OperatorUi, tiles: &mut Vec<Tile>, w: f32) {
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        if tiles.is_empty() {
            ui.weak("（なし）");
        }
        for (i, t) in tiles.iter().enumerate() {
            if tile_button(ui, u, *t, w)
                .on_hover_text("クリックで取り除く")
                .clicked()
            {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        tiles.remove(i);
    }
}

fn seat_combo(ui: &mut egui::Ui, id: &str, s: &TableState, seat: &mut SeatId) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(seat_name(s, *seat))
        .show_ui(ui, |ui| {
            for x in SeatId::ALL {
                ui.selectable_value(seat, x, seat_name(s, x));
            }
        });
}

fn setup_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    ui.heading("半荘を始める");
    egui::ComboBox::from_label("ルール")
        .selected_text(u.presets[u.preset].name.clone())
        .show_ui(ui, |ui| {
            for (i, p) in u.presets.iter().enumerate() {
                ui.selectable_value(&mut u.preset, i, p.name.clone());
            }
        });
    let c = &u.presets[u.preset];
    ui.weak(format!(
        "持ち点 {} / 返し {} / ウマ {}・{} / 赤{} / {}",
        fmt_pts(c.start_points),
        fmt_pts(c.return_points),
        c.uma[1] / 1000,
        c.uma[0] / 1000,
        c.red_fives,
        if c.length == janscope_core::rules::GameLength::Hanchan {
            "東南戦"
        } else {
            "東風戦"
        }
    ));
    ui.label("ルールの編集・プリセットの保存は上部の「設定」→ ルール");
    egui::Grid::new("players").show(ui, |ui| {
        ui.label("席");
        ui.label("選手名");
        ui.label("所属（任意）");
        ui.label("起家");
        ui.end_row();
        for i in 0..4 {
            ui.label(format!("{}", SeatId(i as u8).label()));
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut u.names[i]);
                if !u.registry.is_empty() {
                    let reg = u.registry.clone();
                    egui::ComboBox::from_id_salt(format!("reg{i}"))
                        .selected_text("登録から")
                        .show_ui(ui, |ui| {
                            for p in &reg {
                                if ui.selectable_label(false, &p.name).clicked() {
                                    u.names[i] = p.name.clone();
                                    u.affiliations[i] = p.affiliation.clone().unwrap_or_default();
                                }
                            }
                        });
                }
            });
            ui.text_edit_singleline(&mut u.affiliations[i]);
            ui.radio_value(&mut u.chiicha, i as u8, "");
            ui.end_row();
        }
    });
    let label = if state.phase == Phase::GameOver {
        "次の半荘を始める"
    } else {
        "半荘開始"
    };
    if ui.button(egui::RichText::new(label).size(22.0)).clicked() {
        let players: [Player; 4] = std::array::from_fn(|i| Player {
            name: u.names[i].trim().to_string(),
            affiliation: Some(u.affiliations[i].trim().to_string()).filter(|s| !s.is_empty()),
        });
        net.act(Action::StartGame {
            config: u.presets[u.preset].clone(),
            players,
            chiicha: SeatId(u.chiicha),
        });
    }
}

fn dice_input(ui: &mut egui::Ui, d: &mut [u8; 2]) {
    for v in d.iter_mut() {
        ui.add(egui::DragValue::new(v).range(1..=6).speed(0.1));
    }
}

fn hand_start_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    ui.heading(format!(
        "{} {}本場 を始める",
        state.round_label(),
        state.honba
    ));
    ui.horizontal(|ui| {
        ui.label("サイコロ");
        dice_input(ui, &mut u.dice[0]);
        if state.config.dice_twice {
            ui.checkbox(&mut u.dice_second, "2度振り");
            if u.dice_second {
                dice_input(ui, &mut u.dice[1]);
            }
        }
    });
    if ui
        .button(egui::RichText::new("局開始（残り牌数をリセット）").size(22.0))
        .clicked()
    {
        let mut dice = vec![(u.dice[0][0], u.dice[0][1])];
        if state.config.dice_twice && u.dice_second {
            dice.push((u.dice[1][0], u.dice[1][1]));
        }
        u.dora_edit.clear();
        u.tenpai = [false; 4];
        u.win = empty_win();
        u.pending.clear();
        net.act(Action::StartHand { dice });
    }
    ui.add_space(8.0);
    if !u.confirm_end {
        if ui.button("半荘をここで終了する…").clicked() {
            u.confirm_end = true;
        }
    } else {
        ui.horizontal(|ui| {
            ui.colored_label(egui::Color32::YELLOW, "本当に終了しますか？");
            if ui.button("終了する").clicked() {
                net.act(Action::EndGame);
                u.confirm_end = false;
            }
            if ui.button("やめる").clicked() {
                u.confirm_end = false;
            }
        });
    }
}

/// 手牌・河を使うときのドラ表示牌の直し（テンキーで入れなかったとき・直すとき）
fn dora_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    if u.dora_seen != state.dora_indicators {
        u.dora_seen = state.dora_indicators.clone();
        u.dora_edit = state.dora_indicators.clone();
    }
    ui.collapsing("ドラ表示牌を直す", |ui| {
        ui.horizontal(|ui| {
            ui.label("ドラ表示牌:");
            let mut v = std::mem::take(&mut u.dora_edit);
            tile_row(ui, u, &mut v, 24.0);
            u.dora_edit = v;
        });
        if let Some(t) = palette(ui, u, 26.0, false) {
            if u.dora_edit.len() < 5 {
                u.dora_edit.push(t);
            }
        }
        ui.horizontal(|ui| {
            if u.dora_edit != state.dora_indicators && ui.button("ドラ表示牌を確定").clicked()
            {
                net.act(Action::SetDora {
                    tiles: u.dora_edit.clone(),
                });
            }
            if ui.button("入力し直す").clicked() {
                u.dora_edit.clear();
            }
        });
    });
    if u.dora_edit.is_empty() && !state.dora_indicators.is_empty() {
        u.dora_edit = state.dora_indicators.clone();
    }
}

fn in_hand_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("残り {}", state.wall_remaining))
                .size(26.0)
                .strong(),
        );
        if ui
            .button(egui::RichText::new("−1").size(20.0))
            .on_hover_text("テンキーの −（手牌・河を使わないとき。使うときはツモで自動に減る）")
            .clicked()
        {
            net.act(Action::WallDecrement);
        }
        if ui
            .button("+1")
            .on_hover_text("テンキーの +（手牌・河を使わないとき）")
            .clicked()
        {
            net.act(Action::WallIncrement);
        }
        ui.add(egui::DragValue::new(&mut u.wall_direct).range(0..=state.config.wall_start));
        if ui.button("直接入力").clicked() {
            net.act(Action::SetWallRemaining {
                value: u.wall_direct,
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("立直");
        for seat in SeatId::ALL {
            let r = state.seats[seat.index()].riichi;
            if ui
                .add_enabled(!r, egui::Button::new(seat_name(state, seat)))
                .clicked()
            {
                net.act(Action::Riichi { seat });
            }
        }
        ui.menu_button("取り消し", |ui| {
            for seat in SeatId::ALL
                .into_iter()
                .filter(|s| state.seats[s.index()].riichi)
            {
                if ui.button(seat_name(state, seat)).clicked() {
                    net.act(Action::CancelRiichi { seat });
                    ui.close();
                }
            }
        });
    });
    ui.collapsing("ドラ表示牌・槓", |ui| {
        ui.horizontal(|ui| {
            ui.label("ドラ表示牌:");
            {
                let mut v = std::mem::take(&mut u.dora_edit);
                tile_row(ui, u, &mut v, 24.0);
                u.dora_edit = v;
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut u.target,
                PaletteTarget::Dora,
                "パレットでドラ表示牌を入力",
            );
            ui.radio_value(&mut u.target, PaletteTarget::KanDora, "槓ドラを選ぶ");
        });
        if u.dora_edit != state.dora_indicators && ui.button("ドラ表示牌を確定").clicked() {
            net.act(Action::SetDora {
                tiles: u.dora_edit.clone(),
            });
        }
        if ui.button("入力し直す").clicked() {
            u.dora_edit.clear();
        }
        ui.horizontal(|ui| {
            ui.label("槓:");
            let mut seat = SeatId(u.kan_seat);
            seat_combo(ui, "kan_seat", state, &mut seat);
            u.kan_seat = seat.0;
            ui.radio_value(&mut u.kan_kind, KanKind::Ankan, "暗槓");
            ui.radio_value(&mut u.kan_kind, KanKind::Minkan, "大明槓");
            ui.radio_value(&mut u.kan_kind, KanKind::Kakan, "加槓");
            ui.label(format!(
                "新ドラ: {}",
                u.kan_dora.map(|t| t.name_ja()).unwrap_or("なし".into())
            ));
            if ui.button("槓を入力（残り −1）").clicked() {
                net.act(Action::Kan {
                    seat,
                    kind: u.kan_kind,
                    new_dora_indicator: u.kan_dora.take(),
                    tiles: vec![],
                });
            }
        });
        if matches!(u.target, PaletteTarget::Dora | PaletteTarget::KanDora) {
            if let Some(t) = palette(ui, u, 26.0, false) {
                if u.target == PaletteTarget::Dora {
                    if u.dora_edit.len() < 5 {
                        u.dora_edit.push(t);
                    }
                } else {
                    u.kan_dora = Some(t);
                }
            }
        }
    });
    if u.dora_edit.is_empty() && !state.dora_indicators.is_empty() {
        u.dora_edit = state.dora_indicators.clone();
    }
}

fn win_panel(
    ui: &mut egui::Ui,
    u: &mut OperatorUi,
    net: &Net,
    table: &mut Table,
    state: &TableState,
) {
    ui.heading("アガリ入力");
    ui.horizontal(|ui| {
        ui.label("アガリ");
        seat_combo(ui, "winner", state, &mut u.win.winner);
        let mut tsumo = u.win.loser.is_none();
        ui.radio_value(&mut tsumo, false, "ロン");
        ui.radio_value(&mut tsumo, true, "ツモ");
        if tsumo {
            u.win.loser = None;
        } else {
            let mut l = u.win.loser.unwrap_or(u.win.winner.next());
            ui.label("放銃");
            seat_combo(ui, "loser", state, &mut l);
            u.win.loser = Some(l);
        }
    });
    if state.seats[u.win.winner.index()].riichi && !u.win.double_riichi {
        u.win.riichi = true;
    }
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut u.win.riichi, "立直");
        ui.checkbox(&mut u.win.double_riichi, "ダブル立直");
        ui.checkbox(&mut u.win.ippatsu, "一発");
        ui.checkbox(
            &mut u.win.haitei,
            if u.win.loser.is_none() {
                "海底"
            } else {
                "河底"
            },
        );
        ui.checkbox(&mut u.win.rinshan, "嶺上開花");
        ui.checkbox(&mut u.win.chankan, "槍槓");
        ui.checkbox(&mut u.win.tenhou, "天和・地和");
        if state.wall_remaining == 0 {
            ui.weak("（残り0なので海底・河底は自動で付きます）");
        }
    });
    ui.horizontal(|ui| {
        ui.label("包（パオ）");
        let name = |x: Option<SeatId>| match x {
            None => "なし".to_string(),
            Some(p) => seat_name(state, p),
        };
        egui::ComboBox::from_id_salt("pao")
            .selected_text(name(u.win.pao))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut u.win.pao, None, "なし");
                for p in SeatId::ALL.into_iter().filter(|p| *p != u.win.winner) {
                    ui.selectable_value(&mut u.win.pao, Some(p), seat_name(state, p));
                }
            })
            .response
            .on_hover_text(
                "大三元・大四喜・四槓子を確定させる牌を鳴かせた人（役満のときだけ使われる）",
            );
        if u.win.pao == Some(u.win.winner) {
            u.win.pao = None;
        }
    });
    ui.horizontal(|ui| {
        ui.label("入力先:");
        ui.radio_value(
            &mut u.target,
            PaletteTarget::Closed,
            "手牌（門前、アガリ牌は最後に）",
        );
        ui.radio_value(&mut u.target, PaletteTarget::Meld, "副露");
        ui.radio_value(&mut u.target, PaletteTarget::Ura, "裏ドラ表示牌");
    });
    ui.horizontal(|ui| {
        ui.label(format!("手牌 {}枚:", u.win.closed.len()));
        {
            let mut v = std::mem::take(&mut u.win.closed);
            tile_row(ui, u, &mut v, 30.0);
            u.win.closed = v;
        }
    });
    if let Some(&last) = u.win.closed.last() {
        u.win.win_tile = last;
        ui.label(format!("アガリ牌: {}（最後に入れた牌）", last.name_ja()));
    }
    ui.horizontal(|ui| {
        ui.label("副露:");
        let mut remove = None;
        for (i, m) in u.win.melds.iter().enumerate() {
            let s: String = m
                .tiles
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("");
            if ui.button(format!("{:?} {s} ×", m.kind)).clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            u.win.melds.remove(i);
        }
    });
    if u.target == PaletteTarget::Meld {
        ui.horizontal(|ui| {
            ui.radio_value(&mut u.meld_kind, MeldKind::Chi, "チー");
            ui.radio_value(&mut u.meld_kind, MeldKind::Pon, "ポン");
            ui.radio_value(&mut u.meld_kind, MeldKind::Minkan, "明槓");
            ui.radio_value(&mut u.meld_kind, MeldKind::Ankan, "暗槓");
            ui.radio_value(&mut u.meld_kind, MeldKind::Kakan, "加槓");
            ui.label("入力中:");
            {
                let mut v = std::mem::take(&mut u.meld_buf);
                tile_row(ui, u, &mut v, 24.0);
                u.meld_buf = v;
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("裏ドラ表示牌:");
        {
            let mut v = std::mem::take(&mut u.win.ura_indicators);
            tile_row(ui, u, &mut v, 24.0);
            u.win.ura_indicators = v;
        }
    });
    if matches!(
        u.target,
        PaletteTarget::Closed | PaletteTarget::Meld | PaletteTarget::Ura
    ) {
        if let Some(t) = palette(ui, u, 34.0, state.config.red_fives > 0) {
            match u.target {
                PaletteTarget::Closed => {
                    if u.win.closed.len() < 14 {
                        u.win.closed.push(t);
                    }
                }
                PaletteTarget::Meld => {
                    u.meld_buf.push(t);
                    let need = if u.meld_kind.is_kan() { 4 } else { 3 };
                    if u.meld_kind == MeldKind::Pon || u.meld_kind.is_kan() {
                        u.meld_buf = vec![t; need];
                    }
                    if u.meld_kind == MeldKind::Chi
                        && u.meld_buf.len() == 1
                        && t.number() <= 7
                        && !t.is_honor()
                    {
                        u.meld_buf = vec![t, Tile::new(t.kind() + 1), Tile::new(t.kind() + 2)];
                    }
                    if u.meld_buf.len() >= need {
                        let from = if u.meld_kind == MeldKind::Ankan {
                            None
                        } else {
                            Some(u.win.winner.next())
                        };
                        u.win.melds.push(Meld {
                            kind: u.meld_kind,
                            tiles: u.meld_buf.drain(..).take(need).collect(),
                            from,
                        });
                    }
                }
                PaletteTarget::Ura => {
                    if u.win.ura_indicators.len() < 5 {
                        u.win.ura_indicators.push(t);
                    }
                }
                _ => {}
            }
        }
    }
    ui.horizontal(|ui| {
        if ui
            .button(egui::RichText::new("プレビュー（計算だけ）").size(20.0))
            .clicked()
        {
            u.preview_id = Some(net.preview(u.win.clone()));
            table.preview = None;
        }
        if ui.button("手牌をクリア").clicked() {
            u.win.closed.clear();
            u.win.melds.clear();
            u.win.ura_indicators.clear();
            table.preview = None;
        }
    });
    if let (Some(id), Some((rid, res))) = (u.preview_id, &table.preview) {
        if id == *rid {
            match res {
                Ok(sc) => {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  {}",
                                sc.han_fu_text(),
                                sc.points_text()
                            ))
                            .size(26.0)
                            .strong(),
                        );
                        ui.label(
                            sc.yaku
                                .iter()
                                .map(|y| {
                                    if y.yakuman > 0 {
                                        y.name.clone()
                                    } else {
                                        format!("{} {}翻", y.name, y.han)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("、"),
                        );
                        if sc.dora + sc.aka + sc.ura > 0 {
                            ui.label(format!("ドラ{} 赤{} 裏{}", sc.dora, sc.aka, sc.ura));
                        }
                        ui.weak(
                            sc.fu_items
                                .iter()
                                .map(|f| format!("{} {}", f.name, f.fu))
                                .collect::<Vec<_>>()
                                .join(" / "),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .button(
                                    egui::RichText::new("放送に出す（確定）")
                                        .size(22.0)
                                        .color(egui::Color32::YELLOW),
                                )
                                .clicked()
                            {
                                let mut wins = u.pending.clone();
                                wins.push(u.win.clone());
                                net.act(Action::Win { wins });
                                u.pending.clear();
                                u.win = empty_win();
                                u.preview_id = None;
                            }
                            if u.win.loser.is_some()
                                && ui
                                    .button("ダブロン: このアガリを保留して次を入力")
                                    .clicked()
                            {
                                u.pending.push(u.win.clone());
                                let loser = u.win.loser;
                                u.win = empty_win();
                                u.win.loser = loser;
                                u.preview_id = None;
                            }
                        });
                    });
                }
                Err(e) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 140, 100),
                        format!("計算できません: {e}"),
                    );
                }
            }
        }
    }
    if !u.pending.is_empty() {
        ui.label(format!("保留中のアガリ: {}件（ダブロン）", u.pending.len()));
        if ui.button("保留を取り消す").clicked() {
            u.pending.clear();
        }
    }
}

fn draw_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    ui.collapsing("流局", |ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut u.draw_choice, DrawChoice::Exhaustive, "荒牌流局");
            ui.radio_value(&mut u.draw_choice, DrawChoice::Nagashi, "流し満貫");
            for k in [
                AbortiveKind::KyuushuKyuuhai,
                AbortiveKind::SuufonRenda,
                AbortiveKind::SuukanSanra,
                AbortiveKind::SuuchaRiichi,
                AbortiveKind::SanchaHou,
            ] {
                ui.radio_value(&mut u.draw_choice, DrawChoice::Abortive(k), k.name_ja());
            }
        });
        if !matches!(u.draw_choice, DrawChoice::Abortive(_)) {
            ui.horizontal(|ui| {
                ui.label("テンパイ:");
                for seat in SeatId::ALL {
                    ui.checkbox(&mut u.tenpai[seat.index()], seat_name(state, seat));
                }
            });
        }
        if u.draw_choice == DrawChoice::Nagashi {
            ui.horizontal(|ui| {
                ui.label("流し満貫:");
                for seat in SeatId::ALL {
                    ui.checkbox(&mut u.nagashi[seat.index()], seat_name(state, seat));
                }
            });
        }
        if ui
            .button(egui::RichText::new("流局を確定").size(20.0))
            .clicked()
        {
            let tenpai: Vec<SeatId> = SeatId::ALL
                .into_iter()
                .filter(|s| u.tenpai[s.index()])
                .collect();
            let kind = match u.draw_choice {
                DrawChoice::Exhaustive => DrawKind::Exhaustive,
                DrawChoice::Nagashi => DrawKind::NagashiMangan(
                    SeatId::ALL
                        .into_iter()
                        .filter(|s| u.nagashi[s.index()])
                        .collect(),
                ),
                DrawChoice::Abortive(k) => DrawKind::Abortive(k),
            };
            net.act(Action::Draw { tenpai, kind });
            u.tenpai = [false; 4];
            u.nagashi = [false; 4];
        }
        if state.wall_remaining == 0 {
            ui.colored_label(
                egui::Color32::YELLOW,
                "残り牌数が0です。アガリがなければ流局を入力してください",
            );
        }
    });
}

fn fix_panel(ui: &mut egui::Ui, u: &mut OperatorUi, net: &Net, state: &TableState) {
    ui.collapsing("手動修正（点数・局・本場・供託）", |ui| {
        ui.horizontal(|ui| {
            for seat in SeatId::ALL {
                ui.label(seat.label().to_string());
                ui.add(
                    egui::DragValue::new(&mut u.adjust[seat.index()])
                        .speed(100.0)
                        .range(-200_000..=200_000),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("理由");
            ui.text_edit_singleline(&mut u.adjust_reason);
            if ui.button("点数を調整").clicked() {
                net.act(Action::ManualAdjust {
                    deltas: u.adjust,
                    reason: u.adjust_reason.clone(),
                });
                u.adjust = [0; 4];
                u.adjust_reason.clear();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("現在の値を読み込む").clicked() {
                u.round_fix = (
                    state.round_wind,
                    state.round_number,
                    state.honba,
                    state.kyotaku,
                );
            }
            egui::ComboBox::from_id_salt("fix_wind")
                .selected_text(u.round_fix.0.name_ja())
                .show_ui(ui, |ui| {
                    for w in Wind::ALL {
                        ui.selectable_value(&mut u.round_fix.0, w, w.name_ja());
                    }
                });
            ui.add(
                egui::DragValue::new(&mut u.round_fix.1)
                    .range(1..=4)
                    .suffix("局"),
            );
            ui.add(
                egui::DragValue::new(&mut u.round_fix.2)
                    .range(0..=99)
                    .suffix("本場"),
            );
            ui.add(
                egui::DragValue::new(&mut u.round_fix.3)
                    .range(0..=99)
                    .prefix("供託"),
            );
            if ui.button("局・本場・供託を修正").clicked() {
                let (wind, number, honba, kyotaku) = u.round_fix;
                net.act(Action::SetRound {
                    wind,
                    number,
                    honba,
                    kyotaku,
                });
            }
        });
    });
}

fn stream_status_text(s: Option<StreamStatus>) -> (String, egui::Color32) {
    match s {
        None => ("使わない".into(), egui::Color32::GRAY),
        Some(StreamStatus::Starting) => ("待機中".into(), egui::Color32::YELLOW),
        Some(StreamStatus::Live) => ("受信中".into(), egui::Color32::LIGHT_GREEN),
        Some(StreamStatus::NoSignal) => ("信号なし".into(), egui::Color32::from_rgb(255, 140, 100)),
        Some(StreamStatus::Failed(e)) => (format!("失敗: {e}"), egui::Color32::RED),
    }
}

/// 顔カメラ・OBS 連携・選手画面の切替の設定（設計書 4.6 の映像入力・役割割り当て・切替タブの一部。全タブは段階B8）。
fn face_settings_panel(
    ui: &mut egui::Ui,
    u: &mut OperatorUi,
    cfg: &mut ClientConfig,
    face: &mut FaceRuntime,
) {
    // JANSCOPE_OPEN_FACES で最初から開く（確認用）
    egui::CollapsingHeader::new("顔カメラ・OBS 連携・切替の設定")
        .default_open(std::env::var_os("JANSCOPE_OPEN_FACES").is_some())
        .show(ui, |ui| {
        let d = u.cfg_draft.get_or_insert_with(|| cfg.clone());
        ui.label("選手画面の映像");
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.face_output, FaceOutput::Video, "顔カメラを取り込む（RTMP）")
                .on_hover_text("別の配信元から RTMP で送られてくる顔カメラの映像を、このソフトで受けて表示します");
            ui.radio_value(&mut d.face_output, FaceOutput::Black, "黒枠（OBS で重ねる）");
            ui.radio_value(&mut d.face_output, FaceOutput::Chroma, "クロマキー（緑）");
        });
        if d.face_output == FaceOutput::Video {
            egui::Grid::new("faces").striped(true).show(ui, |ui| {
                ui.label("席");
                ui.label("使う");
                ui.label("受け方");
                ui.label("URL");
                ui.label("状態");
                ui.end_row();
                for i in 0..4 {
                    let f = &mut d.faces[i];
                    ui.label(SeatId(i as u8).label().to_string());
                    ui.checkbox(&mut f.enabled, "");
                    let label = match f.mode {
                        RtmpMode::Listen => "待ち受け",
                        RtmpMode::Pull => "引っ張る",
                        RtmpMode::Media => "メディアサーバー",
                    };
                    let before = f.mode;
                    egui::ComboBox::from_id_salt(format!("rtmp_mode{i}"))
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut f.mode, RtmpMode::Media, "メディアサーバー")
                                .on_hover_text("メディアサーバーが映像サンドボックスの「選手顔」の枠で切り出した映像を受ける（おすすめ。入力担当を起動し直してもカメラは切れない）");
                            ui.selectable_value(&mut f.mode, RtmpMode::Listen, "待ち受け")
                                .on_hover_text("配信元から rtmp://このPCのIP:1935/live/... に送ってもらう");
                            ui.selectable_value(&mut f.mode, RtmpMode::Pull, "引っ張る")
                                .on_hover_text("配信元のサーバーの rtmp://... から受け取る");
                        });
                    if f.mode == RtmpMode::Media && before != RtmpMode::Media {
                        f.url = crate::sandbox::face_url(&d.media_url, i as u8);
                    }
                    if f.mode == RtmpMode::Media {
                        ui.weak("映像サンドボックスの「選手顔」の枠");
                    } else if f.mode == RtmpMode::Listen {
                        ui.weak("下の「カメラの受信」で設定します");
                    } else {
                        ui.add(egui::TextEdit::singleline(&mut f.url).desired_width(320.0));
                    }
                    let (t, c) = stream_status_text(face.status(SeatId(i as u8)));
                    ui.colored_label(c, t);
                    ui.end_row();
                }
            });
            crate::cameras::camera_section(ui, &mut u.cam, d, cfg, face, &u.tex);
            ui.horizontal(|ui| {
                ui.label("ffmpeg");
                ui.add(egui::TextEdit::singleline(&mut d.ffmpeg).desired_width(360.0))
                    .on_hover_text("ffmpeg の実行ファイル。LGPL ビルドを使ってください");
                crate::filepick::button(ui, "ffmpeg", crate::filepick::Pick::File("ffmpeg", &["exe", ""]), &mut d.ffmpeg);
            });
        } else {
            ui.checkbox(&mut d.obs.enabled, "切替の結果を OBS に反映する（obs-websocket）");
            if d.obs.enabled {
                ui.horizontal(|ui| {
                    ui.label("接続先");
                    ui.text_edit_singleline(&mut d.obs.url);
                    ui.label("パスワード");
                    ui.add(egui::TextEdit::singleline(&mut d.obs.password).password(true).desired_width(140.0));
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.obs.mode, ObsSwitchMode::Scene, "シーンを切り替える");
                    ui.radio_value(&mut d.obs.mode, ObsSwitchMode::SourceVisibility, "ソースの表示を切り替える");
                });
                egui::Grid::new("obs_names").show(ui, |ui| {
                    if d.obs.mode == ObsSwitchMode::SourceVisibility {
                        ui.label("シーン");
                        ui.text_edit_singleline(&mut d.obs.source_scene);
                        ui.end_row();
                    }
                    for i in 0..4 {
                        ui.label(format!("席{}", SeatId(i as u8).label()));
                        if d.obs.mode == ObsSwitchMode::Scene {
                            ui.text_edit_singleline(&mut d.obs.scenes[i]);
                        } else {
                            ui.text_edit_singleline(&mut d.obs.sources[i]);
                        }
                        ui.end_row();
                    }
                });
                let (t, c) = match face.obs_status() {
                    ObsStatus::Disabled => ("未接続".to_string(), egui::Color32::GRAY),
                    ObsStatus::Connecting => ("接続しています".to_string(), egui::Color32::YELLOW),
                    ObsStatus::Connected { obs_version } => (format!("接続中（obs-websocket {obs_version}）"), egui::Color32::LIGHT_GREEN),
                    ObsStatus::Error(e) => (e, egui::Color32::from_rgb(255, 140, 100)),
                };
                ui.colored_label(c, t);
            }
        }
        ui.separator();
        ui.label("配信ディレイ");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut d.delay_secs).range(0.0..=600.0).speed(0.5).suffix("秒"))
                .on_hover_text("出力ウィンドウ全体（点数・手牌・演出・顔カメラ）を遅らせます。操作ウィンドウと選手用画面は遅れません");
            ui.weak("OBS の配信遅延を使う場合は 0 にしてください");
        });
        ui.horizontal(|ui| {
            ui.label("同期補正（ms）");
            for i in 0..4 {
                ui.label(SeatId(i as u8).label().to_string());
                ui.add(egui::DragValue::new(&mut d.sync_offset_ms[i]).range(-5000..=10000).speed(10.0))
                    .on_hover_text("この顔カメラの映像だけをさらに遅らせる（正）／早める（負、ディレイの範囲内）");
            }
        });
        let bytes: usize = face.streams.iter().flatten().map(|s| s.buffered_bytes()).sum();
        let per_sec: f64 = face.streams.iter().flatten().map(|s| s.buffered_bytes() as f64 / (face.delay_secs + 3.0)).sum();
        ui.weak(format!(
            "映像のバッファ: いま {:.0}MB（{:.0}秒にすると約 {:.0}MB）",
            bytes as f64 / 1e6,
            d.delay_secs,
            per_sec * (d.delay_secs + 3.0) / 1e6
        ));
        ui.separator();
        ui.label("選手画面の切替");
        let fs = &mut d.face_switch;
        ui.horizontal(|ui| {
            ui.label("手動で選んだとき");
            ui.radio_value(&mut fs.manual_behavior, ManualBehavior::Temporary, "一時手動");
            ui.radio_value(&mut fs.manual_behavior, ManualBehavior::Locked, "手動固定");
            if fs.manual_behavior == ManualBehavior::Temporary {
                ui.add(egui::DragValue::new(&mut fs.temp_manual_secs).range(3.0..=600.0).suffix("秒で自動へ"));
            }
        });
        ui.label("自動切替のきっかけ（上ほど優先）");
        let mut swap = None;
        let n = fs.triggers.len();
        for (i, t) in fs.triggers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut t.enabled, t.kind.name_ja());
                ui.add(egui::DragValue::new(&mut t.secs).range(1.0..=120.0).suffix("秒"));
                if i > 0 && ui.small_button("上へ").clicked() {
                    swap = Some(i);
                }
                if i + 1 < n && ui.small_button("下へ").clicked() {
                    swap = Some(i + 1);
                }
            });
        }
        if let Some(i) = swap {
            fs.triggers.swap(i - 1, i);
        }
        ui.horizontal(|ui| {
            ui.label("それ以外のとき");
            ui.radio_value(&mut fs.idle, IdleMode::Rotate, "巡回");
            ui.radio_value(&mut fs.idle, IdleMode::Dealer, "親を表示");
            if fs.idle == IdleMode::Rotate {
                ui.add(egui::DragValue::new(&mut fs.rotate_secs).range(3.0..=300.0).suffix("秒ごと"));
            }
        });
        ui.horizontal(|ui| {
            ui.label("最短表示時間");
            ui.add(egui::DragValue::new(&mut fs.min_display_secs).range(0.0..=60.0).suffix("秒"))
                .on_hover_text("切替直後は、より優先度の高いきっかけ以外では切り替えません（画面がチカチカしないため）");
        });
        // 0: なし 1: 適用 2: キャンセル 3: 既定値
        let mut act = 0u8;
        ui.horizontal(|ui| {
            if ui.button("適用（保存）").clicked() {
                act = 1;
            }
            if ui.button("キャンセル").clicked() {
                act = 2;
            }
            if ui.button("既定値に戻す").clicked() {
                act = 3;
            }
        });
        let draft = d.clone();
        match act {
            1 if draft.delay_secs != cfg.delay_secs && !u.cfg_confirm_delay => {
                u.cfg_confirm_delay = true;
                u.cfg_msg = Some("配信ディレイを変えると、出力の映像が一度飛びます。よろしければもう一度「適用」を押してください".into());
            }
            1 => match draft.save() {
                Ok(()) => {
                    face.apply(&draft);
                    *cfg = draft;
                    u.cfg_confirm_delay = false;
                    u.cfg_msg = Some(format!("保存しました（{}）", crate::config::config_path().display()));
                }
                Err(e) => u.cfg_msg = Some(e),
            },
            2 => {
                u.cfg_draft = None;
                u.cfg_msg = None;
            }
            3 => u.cfg_draft = Some(ClientConfig::default()),
            _ => {}
        }
        if let Some(m) = &u.cfg_msg {
            ui.label(m.clone());
        }
    });
}

/// シーン切替（設計書 4.4 映像エリア: PVW で選んで「切替」で PGM へ）とテロップ。
fn scene_panel(
    ui: &mut egui::Ui,
    u: &mut OperatorUi,
    sc: &mut crate::scenes::Scenes,
    state: &TableState,
    now: f64,
) {
    use janscope_core::scene::{SceneKind, TransitionKind};
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("放送中: {}", sc.sw.program.name_ja()))
                .strong()
                .color(egui::Color32::from_rgb(255, 120, 120)),
        );
        ui.separator();
        ui.label("プレビュー:");
        let fs = SceneKind::Fullscreen(SeatId(u.fullscreen_seat));
        for k in [
            SceneKind::Game,
            fs,
            SceneKind::PlayerIntro,
            SceneKind::Waiting,
            SceneKind::FinalResult,
            SceneKind::Standings,
            SceneKind::Replay,
        ] {
            let label = match k {
                SceneKind::Fullscreen(_) => "全画面映像".to_string(),
                _ => k.name_ja(),
            };
            let sel = std::mem::discriminant(&sc.sw.preview) == std::mem::discriminant(&k);
            if ui.selectable_label(sel, label).clicked() {
                sc.sw.preview = k;
            }
        }
        if matches!(sc.sw.preview, SceneKind::Fullscreen(_)) {
            egui::ComboBox::from_id_salt("fs_seat")
                .selected_text(seat_name(state, SeatId(u.fullscreen_seat)))
                .show_ui(ui, |ui| {
                    for s in SeatId::ALL {
                        if ui
                            .selectable_value(&mut u.fullscreen_seat, s.0, seat_name(state, s))
                            .clicked()
                        {
                            sc.sw.preview = SceneKind::Fullscreen(s);
                        }
                    }
                });
        }
    });
    ui.horizontal(|ui| {
        for t in [
            TransitionKind::Cut,
            TransitionKind::Fade,
            TransitionKind::Wipe,
        ] {
            ui.radio_value(&mut sc.sw.transition, t, t.name_ja());
        }
        if sc.sw.transition != TransitionKind::Cut {
            ui.add(
                egui::DragValue::new(&mut sc.sw.duration)
                    .range(0.1..=5.0)
                    .speed(0.05)
                    .suffix("秒"),
            );
        }
        let can = sc.sw.preview != sc.sw.program && !sc.sw.in_transition();
        if ui
            .add_enabled(
                can,
                egui::Button::new(egui::RichText::new("切替（TAKE）").size(20.0).strong()),
            )
            .clicked()
        {
            sc.take(now);
        }
    });
    ui.horizontal(|ui| {
        ui.label("テロップ");
        ui.add(egui::TextEdit::singleline(&mut u.telop_edit).desired_width(420.0));
        if ui.button("表示").clicked() && !u.telop_edit.trim().is_empty() {
            sc.telop = Some(u.telop_edit.trim().to_string());
            crate::scenes::video_log(&format!("テロップ表示 {}", u.telop_edit.trim()));
        }
        if ui
            .add_enabled(sc.telop.is_some(), egui::Button::new("消す"))
            .clicked()
        {
            sc.telop = None;
            crate::scenes::video_log("テロップ消去");
        }
        ui.separator();
        ui.label("待機画面の文字");
        ui.add(egui::TextEdit::singleline(&mut sc.waiting_text).desired_width(240.0));
    });
}

/// 大会通算（設計書 5.6）: 通算順位、半荘の記録の削除、CSV の書き出し。
fn tournament_panel(
    ui: &mut egui::Ui,
    u: &mut OperatorUi,
    net: &Net,
    t: &janscope_core::tournament::Tournament,
) {
    use janscope_core::protocol::TournamentOp;
    ui.collapsing(format!("大会通算（{}半荘）", t.games.len()), |ui| {
        ui.horizontal(|ui| {
            ui.label("大会名");
            let name = u.tour_name.get_or_insert_with(|| t.name.clone());
            ui.text_edit_singleline(name);
            if ui.button("変更").clicked() {
                net.tournament(TournamentOp::Rename {
                    name: name.trim().to_string(),
                });
                u.tour_name = None;
            }
        });
        egui::Grid::new("standings").striped(true).show(ui, |ui| {
            for h in [
                "順位",
                "選手",
                "半荘",
                "ポイント",
                "1着",
                "2着",
                "3着",
                "4着",
                "平均",
            ] {
                ui.label(h);
            }
            ui.end_row();
            for s in t.standings() {
                ui.label(s.rank.to_string());
                ui.label(&s.name);
                ui.label(s.games.to_string());
                ui.label(format!("{:+.1}", s.points));
                for n in s.placements {
                    ui.label(n.to_string());
                }
                ui.label(format!("{:.2}", s.average_rank));
                ui.end_row();
            }
        });
        ui.collapsing("半荘の記録", |ui| {
            for (i, g) in t.games.iter().enumerate() {
                ui.horizontal(|ui| {
                    let top = g
                        .results
                        .first()
                        .map(|r| g.players[r.seat.index()].name.clone())
                        .unwrap_or_default();
                    ui.label(format!("{}. トップ {top}", i + 1));
                    if u.remove_key.as_deref() == Some(g.key.as_str()) {
                        if ui.button("本当に消す").clicked() {
                            net.tournament(TournamentOp::RemoveGame { key: g.key.clone() });
                            u.remove_key = None;
                        }
                        if ui.button("やめる").clicked() {
                            u.remove_key = None;
                        }
                    } else if ui.small_button("記録から消す").clicked() {
                        u.remove_key = Some(g.key.clone());
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label("CSV の保存先フォルダ");
            ui.text_edit_singleline(&mut u.csv_dir);
            crate::filepick::button(ui, "csv_dir", crate::filepick::Pick::Folder, &mut u.csv_dir);
            if ui.button("CSV を書き出す").clicked() {
                let dir = std::path::PathBuf::from(u.csv_dir.trim());
                let a = dir.join("通算順位.csv");
                let b = dir.join("半荘結果.csv");
                u.csv_msg = Some(
                    match std::fs::write(&a, t.standings_csv())
                        .and_then(|_| std::fs::write(&b, t.games_csv()))
                    {
                        Ok(()) => format!("保存しました: {} / {}", a.display(), b.display()),
                        Err(e) => format!("保存できません: {e}"),
                    },
                );
            }
        });
        if let Some(m) = &u.csv_msg {
            ui.label(m.clone());
        }
    });
}

/// 牌譜再生の操作（設計書 4.9 操作）。
fn replay_panel(ui: &mut egui::Ui, r: &mut crate::replay::Replay, rp: &crate::prefs::ReplayPrefs) {
    if r.file.is_empty() && !rp.folder.is_empty() {
        r.file = format!(
            "{}{}",
            rp.folder.trim_end_matches(['/', '\\']),
            std::path::MAIN_SEPARATOR
        );
    }
    ui.collapsing("牌譜再生（天鳳形式 JSON）", |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut r.file)
                    .hint_text("牌譜ファイルのパス")
                    .desired_width(420.0),
            );
            if crate::filepick::button(
                ui,
                "paifu",
                crate::filepick::Pick::File("牌譜（天鳳形式 JSON）", &["json", "txt"]),
                &mut r.file,
            ) {
                let p = r.file.clone();
                r.open(&p);
            }
            if ui.button("開く").clicked() {
                let p = r.file.clone();
                r.open(&p);
                // 設定の牌譜再生タブの既定値
                r.viewpoint = SeatId(rp.viewpoint % 4);
                r.open_all = rp.open_all;
                r.speed = rp.speed;
            }
        });
        let Some(pf) = r.paifu.clone() else {
            if let Some(m) = &r.msg {
                ui.label(m.clone());
            }
            return;
        };
        ui.label(format!("{}  {}", pf.title, pf.players.join(" / ")));
        ui.horizontal(|ui| {
            let cur = pf.hands.get(r.hand).map(|h| h.label()).unwrap_or_default();
            let mut sel = r.hand;
            egui::ComboBox::from_id_salt("replay_hand")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for (i, h) in pf.hands.iter().enumerate() {
                        ui.selectable_value(&mut sel, i, h.label());
                    }
                });
            if sel != r.hand {
                r.set_hand(sel);
            }
            if ui.button("局の先頭").clicked() {
                r.step = 0;
            }
            if ui.button("前の局").clicked() && r.hand > 0 {
                let h = r.hand - 1;
                r.set_hand(h);
            }
            if ui.button("◀ 1手").clicked() {
                r.step_by(-1);
            }
            let play = if r.playing { "一時停止" } else { "再生" };
            if ui.button(play).clicked() {
                r.playing = !r.playing;
            }
            if ui.button("1手 ▶").clicked() {
                r.step_by(1);
            }
            if ui.button("次の局").clicked() {
                let h = r.hand + 1;
                r.set_hand(h);
            }
        });
        ui.horizontal(|ui| {
            let n = r.events_len();
            ui.add(egui::Slider::new(&mut r.step, 0..=n).text("手順"));
            ui.add(
                egui::Slider::new(&mut r.speed, 0.5..=4.0)
                    .text("速度")
                    .suffix("倍"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("視点（下に表示）");
            for s in SeatId::ALL {
                ui.radio_value(&mut r.viewpoint, s, pf.players[s.index()].clone());
            }
            ui.checkbox(&mut r.open_all, "全員の手牌を表示");
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("名場面に登録").clicked() {
                r.add_bookmark();
            }
            let bms = r.bookmarks.clone();
            for b in &bms {
                if ui.small_button(&b.label).on_hover_text(&b.file).clicked() {
                    r.jump(b);
                }
            }
        });
        ui.weak("放送中が「牌譜再生」のとき: ←→ で1手、Space で再生／停止");
        if let Some(m) = &r.msg {
            ui.label(m.clone());
        }
    });
}

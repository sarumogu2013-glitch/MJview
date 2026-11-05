//! 設定ウィンドウの見た目・演出・操作のタブ（設計書 4.6）:
//! 選手用画面 ／ レイアウト ／ テーマ ／ 演出 ／ ショートカット ／ 牌譜再生 ／ データ。

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::prefs::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrefTab {
    PlayerScreen,
    Layout,
    Theme,
    Presentation,
    Shortcuts,
    Replay,
    Data,
}

impl PrefTab {
    pub const ALL: [PrefTab; 7] = [
        PrefTab::PlayerScreen,
        PrefTab::Layout,
        PrefTab::Theme,
        PrefTab::Presentation,
        PrefTab::Shortcuts,
        PrefTab::Replay,
        PrefTab::Data,
    ];
    pub fn name(self) -> &'static str {
        match self {
            PrefTab::PlayerScreen => "選手用画面",
            PrefTab::Layout => "レイアウト",
            PrefTab::Theme => "テーマ",
            PrefTab::Presentation => "演出",
            PrefTab::Shortcuts => "ショートカット",
            PrefTab::Replay => "牌譜再生",
            PrefTab::Data => "データ",
        }
    }
}

/// タブの中身。キーの割り当て待ち（waiting）は押されたキーで埋める。
pub fn prefs_tab(
    ui: &mut egui::Ui,
    tab: PrefTab,
    d: &mut Prefs,
    waiting: &mut Option<String>,
    keys: &ButtonInput<KeyCode>,
    msg: &mut Option<String>,
) {
    match tab {
        PrefTab::PlayerScreen => {
            let p = &mut d.player_screen;
            ui.label("会場の選手向けモニター（--role player で起動したクライアント）の表示。設定ファイルが変わると自動で読み直します。");
            ui.horizontal(|ui| {
                ui.label("画面の下に来る席");
                for i in 0..4u8 {
                    ui.radio_value(&mut p.bottom, i, format!("席{}", (b'A' + i) as char));
                }
            });
            ui.checkbox(&mut p.table_layout, "卓配置（OFF なら一覧表示）");
            ui.checkbox(&mut p.show_others, "他家の点数を表示する")
                .on_hover_text("大会ルールで他家の点数を見せない場合は OFF");
            ui.checkbox(&mut p.show_diff, "トップとの点差を表示する");
            ui.weak("表示するモニター・全画面は起動時の --monitor / --fullscreen で指定します");
        }
        PrefTab::Layout => {
            let l = &mut d.layout;
            ui.label("出力ウィンドウ（対局画面）に出す部品");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut l.show_face, "選手画面（顔）");
                ui.checkbox(&mut l.show_round, "局情報");
                ui.checkbox(&mut l.show_scores, "スコア表");
                ui.checkbox(&mut l.show_dice, "サイコロ");
                ui.checkbox(&mut l.show_dora, "ドラ");
                ui.checkbox(&mut l.show_wall, "残り牌数");
                ui.checkbox(&mut l.show_hand_frames, "手元枠");
            });
            ui.horizontal(|ui| {
                ui.label("スコア表の並び");
                ui.radio_value(&mut l.score_order, ScoreOrder::Seat, "席順固定");
                ui.radio_value(&mut l.score_order, ScoreOrder::Rank, "順位順");
                ui.checkbox(&mut l.show_score_diff, "点差");
                ui.checkbox(&mut l.show_affiliation, "所属");
            });
            ui.horizontal(|ui| {
                ui.label("ドラの表示");
                ui.radio_value(&mut l.dora_mode, DoraMode::Indicator, "表示牌");
                ui.radio_value(&mut l.dora_mode, DoraMode::Dora, "実際のドラ");
                ui.radio_value(&mut l.dora_mode, DoraMode::Both, "両方");
            });
            ui.horizontal(|ui| {
                ui.label("局の開始前のサイコロ");
                ui.radio_value(&mut l.dice_prev, DicePrev::Faded, "前局の出目を薄く");
                ui.radio_value(&mut l.dice_prev, DicePrev::Hidden, "表示しない");
            });
            ui.weak(
                "部品の位置・大きさをドラッグで変える機能は、まだありません（既定の配置で固定）",
            );
        }
        PrefTab::Theme => {
            let t = &mut d.theme;
            ui.label("配色（仮デザイン。あとで差し替えられるようにテーマとして分けています）");
            egui::Grid::new("theme").num_columns(4).show(ui, |ui| {
                let items: [(&str, &mut [u8; 3]); 11] = [
                    ("背景", &mut t.bg),
                    ("枠", &mut t.panel),
                    ("枠（強調）", &mut t.panel_alt),
                    ("文字", &mut t.text),
                    ("文字（薄い）", &mut t.text_dim),
                    ("親マーク", &mut t.dealer),
                    ("リーチ", &mut t.riichi),
                    ("プラス", &mut t.plus),
                    ("マイナス", &mut t.minus),
                    ("警告（残りわずか）", &mut t.warn),
                    ("選手画面の強調", &mut t.highlight),
                ];
                for (i, (name, c)) in items.into_iter().enumerate() {
                    ui.label(name);
                    ui.color_edit_button_srgb(c);
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("牌画像");
                ui.radio_value(&mut t.tiles, TileThemeChoice::Regular, "通常版");
                ui.radio_value(&mut t.tiles, TileThemeChoice::Black, "黒版");
            });
        }
        PrefTab::Presentation => {
            let p = &mut d.presentation;
            egui::Grid::new("pres").num_columns(2).show(ui, |ui| {
                ui.label("アガリ演出（1人あたり）");
                ui.add(
                    egui::DragValue::new(&mut p.win_secs)
                        .range(3.0..=60.0)
                        .suffix("秒"),
                );
                ui.end_row();
                ui.label("点数移動のアニメーション");
                ui.add(
                    egui::DragValue::new(&mut p.score_anim_secs)
                        .range(0.0..=5.0)
                        .speed(0.1)
                        .suffix("秒"),
                )
                .on_hover_text("0 でアニメーションなし");
                ui.end_row();
                ui.label("残り牌数の強調");
                ui.add(
                    egui::DragValue::new(&mut p.wall_warn_at)
                        .range(0..=20)
                        .prefix("残り ")
                        .suffix(" 枚以下"),
                );
                ui.end_row();
                ui.label("局情報");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.show_all_last, "オーラスを表示");
                    ui.checkbox(&mut p.kyotaku_as_points, "供託を点数で（OFF なら本数）");
                });
                ui.end_row();
            });
            ui.weak("アガリ演出のスキップは操作ウィンドウの「アガリ演出を閉じる」");
        }
        PrefTab::Shortcuts => shortcuts_tab(ui, &mut d.shortcuts, waiting, keys),
        PrefTab::Replay => {
            let r = &mut d.replay;
            ui.horizontal(|ui| {
                ui.label("既定の視点（下に表示する席）");
                for i in 0..4u8 {
                    ui.radio_value(&mut r.viewpoint, i, format!("{}", i + 1));
                }
            });
            ui.checkbox(&mut r.open_all, "全員の手牌を表示する");
            ui.add(
                egui::Slider::new(&mut r.speed, 0.5..=4.0)
                    .text("再生速度")
                    .suffix("倍"),
            );
            ui.horizontal(|ui| {
                ui.label("牌譜ファイルの既定のフォルダ");
                ui.text_edit_singleline(&mut r.folder);
            });
            ui.weak("名場面のしおりは操作ウィンドウの「牌譜再生」欄で登録・呼び出し（janscope-bookmarks.json）");
        }
        PrefTab::Data => data_tab(ui, msg),
    }
}

fn shortcuts_tab(
    ui: &mut egui::Ui,
    s: &mut Shortcuts,
    waiting: &mut Option<String>,
    keys: &ButtonInput<KeyCode>,
) {
    // 割り当て待ちなら、押されたキーを入れる
    if let Some(id) = waiting.clone() {
        if keys.just_pressed(KeyCode::Escape) {
            *waiting = None;
        } else if let Some(name) = keys.get_just_pressed().find_map(|k| key_name(*k)) {
            if let Some(slot) = slot(s, &id) {
                *slot = name.to_string();
            }
            *waiting = None;
        }
    }
    ui.label("外付けのキーパッド・フットスイッチ（キーボードとして認識されるもの）のキーも割り当てられます。");
    let rows: [(&str, &str); 13] = [
        ("wall_down", "残り牌数 −1"),
        ("wall_up", "残り牌数 +1"),
        ("undo", "取り消し（Ctrl＋）"),
        ("redo", "やり直し（Ctrl＋）"),
        ("face0", "選手画面: 席A"),
        ("face1", "選手画面: 席B"),
        ("face2", "選手画面: 席C"),
        ("face3", "選手画面: 席D"),
        ("face_auto", "選手画面: 自動に戻す"),
        ("replay_back", "牌譜: 1手戻る"),
        ("replay_forward", "牌譜: 1手進む"),
        ("replay_play", "牌譜: 再生／停止"),
        ("", ""),
    ];
    egui::Grid::new("keys")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for (id, name) in rows.iter().filter(|(id, _)| !id.is_empty()) {
                ui.label(*name);
                let cur = slot(s, id).map(|v| key_label(v)).unwrap_or_default();
                if waiting.as_deref() == Some(*id) {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "キーを押してください（Esc で取り消し）",
                    );
                } else {
                    ui.label(egui::RichText::new(cur).monospace().strong());
                }
                ui.horizontal(|ui| {
                    if ui.small_button("変更").clicked() {
                        *waiting = Some(id.to_string());
                    }
                    if ui.small_button("なし").clicked() {
                        if let Some(v) = slot(s, id) {
                            v.clear();
                        }
                    }
                });
                ui.end_row();
            }
        });
    let dup = s.duplicates();
    if !dup.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(255, 120, 100),
            format!(
                "入力エラー: 同じキーが複数に割り当てられています: {}",
                dup.iter()
                    .map(|d| key_label(d))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        );
    }
}

fn slot<'a>(s: &'a mut Shortcuts, id: &str) -> Option<&'a mut String> {
    Some(match id {
        "wall_down" => &mut s.wall_down,
        "wall_up" => &mut s.wall_up,
        "undo" => &mut s.undo,
        "redo" => &mut s.redo,
        "face0" => &mut s.face[0],
        "face1" => &mut s.face[1],
        "face2" => &mut s.face[2],
        "face3" => &mut s.face[3],
        "face_auto" => &mut s.face_auto,
        "replay_back" => &mut s.replay_back,
        "replay_forward" => &mut s.replay_forward,
        "replay_play" => &mut s.replay_play,
        _ => return None,
    })
}

/// バックアップするファイル（設定フォルダからの相対パス）。
const DATA_FILES: [&str; 4] = [
    "janscope-client.json",
    "janscope-players.json",
    "janscope-bookmarks.json",
    "janscope-video-log.txt",
];

fn base_dir() -> PathBuf {
    crate::config::config_path()
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for f in DATA_FILES {
        if from.join(f).exists() {
            std::fs::copy(from.join(f), to.join(f))?;
            n += 1;
        }
    }
    let rules = from.join("janscope-rules");
    if rules.is_dir() {
        std::fs::create_dir_all(to.join("janscope-rules"))?;
        for e in std::fs::read_dir(&rules)? {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "json") {
                std::fs::copy(&p, to.join("janscope-rules").join(p.file_name().unwrap()))?;
                n += 1;
            }
        }
    }
    Ok(n)
}

fn data_tab(ui: &mut egui::Ui, msg: &mut Option<String>) {
    let dir = base_dir();
    ui.label(format!(
        "設定の保存先: {}",
        std::fs::canonicalize(&dir).unwrap_or(dir.clone()).display()
    ));
    ui.weak("クライアント設定・ルールのプリセット・選手の登録・名場面のしおり・映像の切替記録");
    ui.weak("対局の操作ログと大会通算はサーバー（ノートPC）の data フォルダにあります");
    if ui.button("設定をバックアップ").clicked() {
        let to = dir.join(format!(
            "janscope-backup-{}",
            janscope_core::protocol::now_ms()
        ));
        *msg = Some(match copy_all(&dir, &to) {
            Ok(n) => format!("{n}個のファイルを {} に保存しました", to.display()),
            Err(e) => format!("バックアップできません: {e}"),
        });
    }
    ui.horizontal(|ui| {
        let id = egui::Id::new("restore_path");
        let mut path: String = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut path).hint_text("バックアップのフォルダ").desired_width(360.0));
        ui.data_mut(|d| d.insert_temp(id, path.clone()));
        if ui.button("復元").clicked() {
            let from = PathBuf::from(path.trim());
            *msg = Some(if !from.join("janscope-client.json").exists() {
                "そのフォルダにバックアップ（janscope-client.json）がありません".into()
            } else {
                match copy_all(&from, &dir) {
                    Ok(n) => format!("{n}個のファイルを戻しました。反映するにはクライアントを起動し直してください"),
                    Err(e) => format!("復元できません: {e}"),
                }
            });
        }
    });
}

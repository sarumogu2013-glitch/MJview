//! 設定ウィンドウ（設計書 4.6）。操作ウィンドウの上部の「設定」から開く。
//! このファイル: ルールタブ（5.3 の全項目、プリセットの新規・複製・名前変更・削除・読込・書出、対局への適用）と選手タブ。
//! 顔カメラ・OBS 連携・切替・配信ディレイの設定は操作ウィンドウの左の欄（operator.rs）。
//!
//! 保存場所（設定ファイル janscope-client.json と同じフォルダ）:
//!   janscope-rules/<名前>.json   ルールのプリセット（同梱のプリセットは書き換えない）
//!   janscope-players.json        選手の登録

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use janscope_core::rules::{DoubleWindPairFu, GameConfig, GameLength, MultiRon, TieBreak};
use janscope_core::state::{Action, Phase, Player};

use crate::net::{Net, Table};

/// ルールのプリセットと選手の登録。
#[derive(Resource)]
pub struct Library {
    pub user_presets: Vec<GameConfig>,
    pub players: Vec<Player>,
    dir: PathBuf,
}

fn base_dir() -> PathBuf {
    crate::config::config_path()
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// ファイル名に使えない文字を置き換える。
fn file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if r#"\/:*?"<>|"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "ルール".into() } else { s }
}

impl Library {
    pub fn load() -> Self {
        let dir = base_dir();
        let mut user_presets = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir.join("janscope-rules")) {
            let mut files: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            files.sort();
            for f in files {
                match std::fs::read_to_string(&f)
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<GameConfig>(&s).map_err(|e| e.to_string()))
                {
                    Ok(c) => user_presets.push(c),
                    Err(e) => warn!("ルールのプリセット {} が読めません: {e}", f.display()),
                }
            }
        }
        let players = std::fs::read_to_string(dir.join("janscope-players.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Library {
            user_presets,
            players,
            dir,
        }
    }

    /// 同梱のプリセット＋保存したプリセット。
    pub fn all_presets(&self) -> Vec<GameConfig> {
        let mut v = GameConfig::presets();
        v.extend(self.user_presets.iter().cloned());
        v
    }

    pub fn is_builtin(name: &str) -> bool {
        GameConfig::presets().iter().any(|p| p.name == name)
    }

    fn preset_path(&self, name: &str) -> PathBuf {
        self.dir
            .join("janscope-rules")
            .join(format!("{}.json", file_name(name)))
    }

    pub fn save_preset(&mut self, c: &GameConfig) -> Result<(), String> {
        if Self::is_builtin(&c.name) {
            return Err(
                "同梱のプリセットと同じ名前では保存できません。名前を変えてください".into(),
            );
        }
        let p = self.preset_path(&c.name);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, serde_json::to_string_pretty(c).unwrap())
            .map_err(|e| format!("{} に保存できません: {e}", p.display()))?;
        match self.user_presets.iter_mut().find(|x| x.name == c.name) {
            Some(x) => *x = c.clone(),
            None => self.user_presets.push(c.clone()),
        }
        Ok(())
    }

    pub fn delete_preset(&mut self, name: &str) -> Result<(), String> {
        let p = self.preset_path(name);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
        self.user_presets.retain(|x| x.name != name);
        Ok(())
    }

    pub fn save_players(&self) -> Result<(), String> {
        let p = self.dir.join("janscope-players.json");
        std::fs::write(&p, serde_json::to_string_pretty(&self.players).unwrap())
            .map_err(|e| format!("{} に保存できません: {e}", p.display()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Rules,
    Players,
    Pref(crate::settings_prefs::PrefTab),
    Update,
    About,
}

#[derive(Resource, Default)]
pub struct SettingsUi {
    pub open: bool,
    tab: Tab,
    draft: Option<GameConfig>,
    /// 名前を変える前の名前（プリセットの名前変更で古いファイルを消すため）
    draft_origin: Option<String>,
    io_path: String,
    msg: Option<String>,
    confirm_apply: bool,
    confirm_delete: bool,
    players_draft: Option<Vec<Player>>,
    prefs_draft: Option<crate::prefs::Prefs>,
    key_wait: Option<String>,
    pub update: crate::update::UpdateState,
}

impl SettingsUi {
    /// ルールタブを開いて、そのルールを編集中にする（確認用）。
    /// 指定のタブを開く（確認用）。
    pub fn open_tab(&mut self, name: &str) {
        self.open = true;
        match name {
            "ルール" | "rules" => return self.tab = Tab::Rules,
            "選手" | "players" => return self.tab = Tab::Players,
            "更新" | "update" => return self.tab = Tab::Update,
            "情報" | "about" => return self.tab = Tab::About,
            _ => {}
        }
        if let Some(t) = crate::settings_prefs::PrefTab::ALL
            .into_iter()
            .find(|t| t.name() == name)
        {
            self.tab = Tab::Pref(t);
        }
    }

    pub fn open_rules(&mut self, c: GameConfig) {
        self.open = true;
        self.tab = Tab::Rules;
        self.draft = Some(c);
    }
}

pub fn settings_ui(
    mut contexts: EguiContexts,
    mut st: ResMut<SettingsUi>,
    mut lib: ResMut<Library>,
    table: Res<Table>,
    net: Res<Net>,
    mut cfg: ResMut<crate::config::ClientConfig>,
    keys: Res<ButtonInput<KeyCode>>,
) -> Result {
    if !st.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut open = true;
    let st = &mut *st;
    egui::Window::new("設定")
        .open(&mut open)
        .default_size([900.0, 700.0])
        .vscroll(true)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut st.tab, Tab::Rules, "ルール");
                ui.selectable_value(&mut st.tab, Tab::Players, "選手");
                for t in crate::settings_prefs::PrefTab::ALL {
                    ui.selectable_value(&mut st.tab, Tab::Pref(t), t.name());
                }
                ui.selectable_value(&mut st.tab, Tab::Update, "更新");
                ui.selectable_value(&mut st.tab, Tab::About, "情報");
                ui.weak(
                    "（顔カメラ・OBS・切替・ディレイは左の欄の「顔カメラ・OBS 連携・切替の設定」）",
                );
            });
            ui.separator();
            match st.tab {
                Tab::Rules => rules_tab(ui, st, &mut lib, &table, &net),
                Tab::Players => players_tab(ui, st, &mut lib),
                Tab::Pref(t) => prefs_page(ui, st, t, &mut cfg, &keys),
                Tab::Update => crate::update::update_tab(ui, &mut st.update, &mut cfg, &mut st.msg),
                Tab::About => crate::update::about_tab(ui),
            }
            if let Some(m) = &st.msg {
                ui.separator();
                ui.label(m.clone());
            }
        });
    if !open {
        st.open = false;
    }
    Ok(())
}

fn rules_tab(ui: &mut egui::Ui, st: &mut SettingsUi, lib: &mut Library, table: &Table, net: &Net) {
    let current = table.full.as_ref().map(|f| f.state.clone());
    if let Some(s) = &current {
        if s.phase != Phase::Setup {
            ui.label(
                egui::RichText::new(format!("現在の対局に適用中のルール: {}", s.config.name))
                    .strong(),
            );
        }
    }
    // プリセットの一覧
    ui.horizontal_wrapped(|ui| {
        ui.label("プリセット:");
        for p in lib.all_presets() {
            let builtin = Library::is_builtin(&p.name);
            let label = if builtin {
                format!("{}（同梱）", p.name)
            } else {
                p.name.clone()
            };
            let sel = st.draft.as_ref().is_some_and(|d| d.name == p.name);
            if ui.selectable_label(sel, label).clicked() {
                st.draft_origin = (!builtin).then(|| p.name.clone());
                st.draft = Some(p);
                st.confirm_apply = false;
                st.confirm_delete = false;
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("新規（標準から）").clicked() {
            let mut c = GameConfig::preset_standard();
            c.name = "新しいルール".into();
            st.draft = Some(c);
            st.draft_origin = None;
        }
        if let Some(s) = &current {
            if ui.button("現在の対局のルールを読み込む").clicked() {
                st.draft = Some(s.config.clone());
                st.draft_origin = None;
            }
        }
    });
    let Some(d) = st.draft.as_mut() else {
        ui.label("プリセットを選ぶか「新規」を押してください");
        return;
    };
    let errs = d.validate();
    rules_editor(ui, d);
    if !errs.is_empty() {
        for e in &errs {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 100),
                format!("入力エラー: {e}"),
            );
        }
    }
    ui.separator();
    let d = d.clone();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(errs.is_empty(), egui::Button::new("プリセットとして保存"))
            .clicked()
        {
            match lib.save_preset(&d) {
                Ok(()) => {
                    // 名前を変えて保存したら、古い名前のファイルは消す（名前変更）
                    if let Some(old) = st.draft_origin.clone().filter(|o| *o != d.name) {
                        let _ = lib.delete_preset(&old);
                    }
                    st.draft_origin = Some(d.name.clone());
                    st.msg = Some(format!("「{}」を保存しました", d.name));
                }
                Err(e) => st.msg = Some(e),
            }
        }
        if ui.button("複製").clicked() {
            let mut c = d.clone();
            c.name = format!("{} のコピー", d.name);
            st.draft = Some(c);
            st.draft_origin = None;
        }
        let deletable = st.draft_origin.is_some();
        if !st.confirm_delete {
            if ui
                .add_enabled(deletable, egui::Button::new("削除"))
                .clicked()
            {
                st.confirm_delete = true;
            }
        } else {
            if ui.button("本当に削除").clicked() {
                if let Some(o) = st.draft_origin.take() {
                    st.msg = Some(match lib.delete_preset(&o) {
                        Ok(()) => format!("「{o}」を削除しました"),
                        Err(e) => e,
                    });
                }
                st.draft = None;
                st.confirm_delete = false;
                return;
            }
            if ui.button("やめる").clicked() {
                st.confirm_delete = false;
            }
        }
        if ui.button("既定値に戻す").clicked() {
            let name = d.name.clone();
            st.draft = Some(GameConfig {
                name,
                ..GameConfig::preset_standard()
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("ファイル");
        ui.add(
            egui::TextEdit::singleline(&mut st.io_path)
                .hint_text("例: C:\\大会\\ルール.json")
                .desired_width(360.0),
        );
        if ui.button("読込").clicked() {
            match std::fs::read_to_string(st.io_path.trim())
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<GameConfig>(&s).map_err(|e| e.to_string()))
            {
                Ok(c) => {
                    st.msg = Some(format!(
                        "「{}」を読み込みました（保存するとプリセットに加わります）",
                        c.name
                    ));
                    st.draft = Some(c);
                    st.draft_origin = None;
                }
                Err(e) => st.msg = Some(format!("読み込めません: {e}")),
            }
        }
        if ui.button("書出").clicked() {
            st.msg = Some(
                match std::fs::write(st.io_path.trim(), serde_json::to_string_pretty(&d).unwrap()) {
                    Ok(()) => format!("{} に書き出しました", st.io_path.trim()),
                    Err(e) => format!("書き出せません: {e}"),
                },
            );
        }
    });
    // 対局への適用（点数に影響するので確認してから）
    let in_game = current.as_ref().is_some_and(|s| s.phase != Phase::Setup);
    if in_game {
        ui.horizontal(|ui| {
            if !st.confirm_apply {
                if ui.add_enabled(errs.is_empty(), egui::Button::new("現在の対局に適用…")).clicked() {
                    st.confirm_apply = true;
                }
            } else {
                ui.colored_label(egui::Color32::YELLOW, "対局中にルールを変えると、以後のアガリの点数や局の進行が変わります。適用しますか？");
                if ui.button("適用する").clicked() {
                    net.act(Action::SetConfig { config: d.clone() });
                    st.confirm_apply = false;
                    st.msg = Some(format!("「{}」を現在の対局に適用しました", d.name));
                }
                if ui.button("やめる").clicked() {
                    st.confirm_apply = false;
                }
            }
        });
    } else {
        ui.weak("対局前は、半荘開始の画面でこのプリセットを選んでください");
    }
}

/// 5.3 の全項目。
fn rules_editor(ui: &mut egui::Ui, d: &mut GameConfig) {
    egui::Grid::new("rules")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("名前");
            ui.text_edit_singleline(&mut d.name);
            ui.end_row();
            ui.label("持ち点 / 返し");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut d.start_points)
                        .range(1_000..=200_000)
                        .speed(100.0),
                );
                ui.add(
                    egui::DragValue::new(&mut d.return_points)
                        .range(1_000..=200_000)
                        .speed(100.0),
                );
            });
            ui.end_row();
            ui.label("ウマ（1〜4位、点）")
                .on_hover_text("合計が0になるように。例: 20000 / 10000 / −10000 / −20000");
            ui.horizontal(|ui| {
                for u in d.uma.iter_mut() {
                    ui.add(
                        egui::DragValue::new(u)
                            .range(-100_000..=100_000)
                            .speed(1000.0),
                    );
                }
            });
            ui.end_row();
            ui.label("オカ")
                .on_hover_text("（返し − 持ち点）× 4 をトップに");
            let oka = format!(
                "あり（トップに {}）",
                janscope_core::scoring::fmt_pts(d.oka_points())
            );
            ui.checkbox(&mut d.oka, oka);
            ui.end_row();
            ui.label("赤ドラ");
            ui.add(
                egui::DragValue::new(&mut d.red_fives)
                    .range(0..=3)
                    .suffix("枚"),
            );
            ui.end_row();
            ui.label("喰いタン / 後付け");
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.kuitan, "喰いタン");
                ui.checkbox(&mut d.atozuke, "後付け").on_hover_text(
                    "表示用（後付けの判定には打牌の記録が必要なため、点数計算では区別しません）",
                );
            });
            ui.end_row();
            ui.label("長さ");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.length, GameLength::Tonpuu, "東風戦");
                ui.radio_value(&mut d.length, GameLength::Hanchan, "東南戦");
                ui.checkbox(&mut d.west_extension, "延長（西入）")
                    .on_hover_text(
                        "規定の局が終わってトップが返し未満なら延長。誰かが返し以上で終局",
                    );
                ui.checkbox(&mut d.bust_ends, "箱下終了");
            });
            ui.end_row();
            ui.label("ダブロン・トリロン");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.multi_ron, MultiRon::All, "全員");
                ui.radio_value(&mut d.multi_ron, MultiRon::HeadBump, "頭ハネ");
            });
            ui.end_row();
            ui.label("満貫・役満");
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.kiriage_mangan, "切り上げ満貫");
                ui.checkbox(&mut d.kazoe_yakuman, "数え役満");
                ui.checkbox(&mut d.double_yakuman, "ダブル役満");
                ui.checkbox(&mut d.multiple_yakuman, "複合役満");
            });
            ui.end_row();
            ui.label("本場の点数");
            ui.add(
                egui::DragValue::new(&mut d.honba_points)
                    .range(0..=3000)
                    .speed(300.0)
                    .suffix("点"),
            );
            ui.end_row();
            ui.label("連風牌の雀頭");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.double_wind_pair_fu, DoubleWindPairFu::Two, "2符");
                ui.radio_value(&mut d.double_wind_pair_fu, DoubleWindPairFu::Four, "4符");
            });
            ui.end_row();
            ui.label("途中流局");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut d.abortive.kyuushu_kyuuhai, "九種九牌");
                ui.checkbox(&mut d.abortive.suufon_renda, "四風連打");
                ui.checkbox(&mut d.abortive.suukan_sanra, "四開槓");
                ui.checkbox(&mut d.abortive.suucha_riichi, "四家立直");
                ui.checkbox(&mut d.abortive.sancha_hou, "三家和");
            });
            ui.end_row();
            ui.label("同点の順位");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.tie_break, TieBreak::SeatOrder, "起家優先");
                ui.radio_value(&mut d.tie_break, TieBreak::Split, "同順位（順位点を等分）")
                    .on_hover_text("割り切れない分は起家に近い人が大きく取る");
            });
            ui.end_row();
            ui.label("局開始の残り牌数");
            ui.add(
                egui::DragValue::new(&mut d.wall_start)
                    .range(1..=122)
                    .suffix("枚"),
            );
            ui.end_row();
            ui.label("サイコロ");
            ui.checkbox(&mut d.dice_twice, "2度振り");
            ui.end_row();
            ui.label("ノーテン罰符");
            ui.add(
                egui::DragValue::new(&mut d.noten_penalty)
                    .range(0..=12_000)
                    .speed(3000.0)
                    .suffix("点"),
            );
            ui.end_row();
            ui.label("連荘・終局");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut d.tenpai_renchan, "テンパイ連荘")
                    .on_hover_text("OFF なら流局は親のテンパイに関わらず親流れ");
                ui.checkbox(&mut d.all_last_dealer_stop, "オーラスのアガリ止め");
                ui.checkbox(&mut d.leftover_kyotaku_to_top, "終局時の供託はトップへ")
                    .on_hover_text("同点のトップが複数で「同順位」なら等分（3人なら 400・300・300）");
            });
            ui.end_row();
            ui.label("包（パオ）");
            ui.checkbox(&mut d.pao_mleague, "Mリーグ方式")
                .on_hover_text("積み場は包の人が払う。ダブル役満以上は包の役満1つ分だけを包の人が払い、残りは普通の払い方。OFF なら役満のツモは包の人が全額、ロンは折半（積み場は放銃者）");
            ui.end_row();
        });
}

fn players_tab(ui: &mut egui::Ui, st: &mut SettingsUi, lib: &mut Library) {
    ui.label("大会に出る選手を登録しておくと、半荘開始の画面で席ごとに選べます。");
    let draft = st.players_draft.get_or_insert_with(|| lib.players.clone());
    let mut remove = None;
    egui::Grid::new("players_reg").striped(true).show(ui, |ui| {
        ui.label("選手名");
        ui.label("所属（任意）");
        ui.label("");
        ui.end_row();
        for (i, p) in draft.iter_mut().enumerate() {
            ui.text_edit_singleline(&mut p.name);
            let mut aff = p.affiliation.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut aff).changed() {
                p.affiliation = Some(aff.trim().to_string()).filter(|s| !s.is_empty());
            }
            if ui.small_button("削除").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        draft.remove(i);
    }
    let mut dup = None;
    for (i, p) in draft.iter().enumerate() {
        if draft[..i].iter().any(|q| q.name.trim() == p.name.trim()) {
            dup = Some(p.name.clone());
        }
    }
    // 0: なし 1: 保存 2: キャンセル
    let mut act = 0u8;
    ui.horizontal(|ui| {
        if ui.button("追加").clicked() {
            draft.push(Player::new(format!("選手{}", draft.len() + 1)));
        }
        if let Some(n) = &dup {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 100),
                format!("入力エラー: 「{n}」が重複しています（通算成績は名前で集計します）"),
            );
        }
        let empty = draft.iter().any(|p| p.name.trim().is_empty());
        if empty {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 100),
                "入力エラー: 名前が空の選手があります",
            );
        }
        if ui
            .add_enabled(dup.is_none() && !empty, egui::Button::new("保存"))
            .clicked()
        {
            act = 1;
        }
        if ui.button("キャンセル").clicked() {
            act = 2;
        }
    });
    let saved: Vec<Player> = draft
        .iter()
        .map(|p| Player {
            name: p.name.trim().to_string(),
            affiliation: p.affiliation.clone(),
        })
        .collect();
    match act {
        1 => {
            lib.players = saved;
            st.msg = Some(match lib.save_players() {
                Ok(()) => format!("{}人を保存しました", lib.players.len()),
                Err(e) => e,
            });
        }
        2 => st.players_draft = None,
        _ => {}
    }
}

/// 見た目・演出・操作のタブ（下書きを編集し、「適用」で保存）。
fn prefs_page(
    ui: &mut egui::Ui,
    st: &mut SettingsUi,
    tab: crate::settings_prefs::PrefTab,
    cfg: &mut crate::config::ClientConfig,
    keys: &ButtonInput<KeyCode>,
) {
    use crate::settings_prefs::PrefTab;
    let mut draft = st.prefs_draft.take().unwrap_or_else(|| cfg.prefs.clone());
    crate::settings_prefs::prefs_tab(ui, tab, &mut draft, &mut st.key_wait, keys, &mut st.msg);
    if tab == PrefTab::Data {
        st.prefs_draft = Some(draft);
        return;
    }
    ui.separator();
    let errors = !draft.shortcuts.duplicates().is_empty();
    let changed = draft != cfg.prefs;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(changed && !errors, egui::Button::new("適用"))
            .clicked()
        {
            cfg.prefs = draft.clone();
            st.msg = Some(match cfg.save() {
                Ok(()) => "保存しました".into(),
                Err(e) => e,
            });
        }
        if ui
            .add_enabled(changed, egui::Button::new("キャンセル"))
            .clicked()
        {
            draft = cfg.prefs.clone();
        }
        if ui.button("このタブを既定値に戻す").clicked() {
            let d = crate::prefs::Prefs::default();
            match tab {
                PrefTab::PlayerScreen => draft.player_screen = d.player_screen,
                PrefTab::Layout => draft.layout = d.layout,
                PrefTab::Theme => draft.theme = d.theme,
                PrefTab::Presentation => draft.presentation = d.presentation,
                PrefTab::Shortcuts => draft.shortcuts = d.shortcuts,
                PrefTab::Replay => draft.replay = d.replay,
                PrefTab::Data => {}
            }
        }
        if changed {
            ui.colored_label(egui::Color32::YELLOW, "未適用の変更があります");
        }
    });
    st.prefs_draft = Some(draft);
}

//! 見た目・演出・操作の設定（設計書 4.6 の レイアウト／テーマ／演出／ショートカット／選手用画面／牌譜再生 タブ）。
//! ClientConfig（janscope-client.json）の `prefs` に保存し、設定ウィンドウから変更する。

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScoreOrder {
    /// 席順固定
    #[default]
    Seat,
    /// 順位順
    Rank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DoraMode {
    /// ドラ表示牌
    #[default]
    Indicator,
    /// 実際のドラ牌
    Dora,
    /// 両方
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DicePrev {
    /// 局の開始前は前局の出目を薄く表示
    #[default]
    Faded,
    /// 局の開始前は表示しない
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutPrefs {
    pub show_face: bool,
    pub show_round: bool,
    pub show_scores: bool,
    pub show_dice: bool,
    pub show_dora: bool,
    pub show_wall: bool,
    pub show_hand_frames: bool,
    pub score_order: ScoreOrder,
    pub dora_mode: DoraMode,
    pub dice_prev: DicePrev,
    /// スコア表にトップとの点差を出す
    pub show_score_diff: bool,
    /// スコア表に所属を出す
    pub show_affiliation: bool,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        LayoutPrefs {
            show_face: true,
            show_round: true,
            show_scores: true,
            show_dice: true,
            show_dora: true,
            show_wall: true,
            show_hand_frames: true,
            score_order: ScoreOrder::Seat,
            dora_mode: DoraMode::Indicator,
            dice_prev: DicePrev::Faded,
            show_score_diff: true,
            show_affiliation: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TileThemeChoice {
    #[default]
    Regular,
    Black,
}

/// 配色（sRGB 0〜255）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemePrefs {
    pub bg: [u8; 3],
    pub panel: [u8; 3],
    pub panel_alt: [u8; 3],
    pub text: [u8; 3],
    pub text_dim: [u8; 3],
    pub dealer: [u8; 3],
    pub riichi: [u8; 3],
    pub plus: [u8; 3],
    pub minus: [u8; 3],
    pub warn: [u8; 3],
    pub highlight: [u8; 3],
    pub tiles: TileThemeChoice,
}

fn rgb8(c: Color) -> [u8; 3] {
    let s = c.to_srgba();
    [
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8,
    ]
}

impl Default for ThemePrefs {
    fn default() -> Self {
        let t = crate::theme::Theme::default();
        ThemePrefs {
            bg: rgb8(t.bg),
            panel: rgb8(t.panel),
            panel_alt: rgb8(t.panel_alt),
            text: rgb8(t.text),
            text_dim: rgb8(t.text_dim),
            dealer: rgb8(t.dealer),
            riichi: rgb8(t.riichi),
            plus: rgb8(t.plus),
            minus: rgb8(t.minus),
            warn: rgb8(t.warn),
            highlight: rgb8(t.highlight),
            tiles: TileThemeChoice::Regular,
        }
    }
}

impl ThemePrefs {
    /// 設定から描画用のテーマを作る（帯・顔の背景は既定のまま）。
    pub fn to_theme(&self) -> crate::theme::Theme {
        let c = |v: [u8; 3]| Color::srgb_u8(v[0], v[1], v[2]);
        crate::theme::Theme {
            bg: c(self.bg),
            panel: c(self.panel),
            panel_alt: c(self.panel_alt),
            text: c(self.text),
            text_dim: c(self.text_dim),
            dealer: c(self.dealer),
            riichi: c(self.riichi),
            plus: c(self.plus),
            minus: c(self.minus),
            warn: c(self.warn),
            highlight: c(self.highlight),
            ..crate::theme::Theme::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresentationPrefs {
    /// アガリ演出の1人あたりの秒数
    pub win_secs: f64,
    /// 点数移動を数えて見せる秒数（0 でなし）
    pub score_anim_secs: f64,
    pub show_all_last: bool,
    /// 供託を点数（1,000）で出す。false なら本数（1本）
    pub kyotaku_as_points: bool,
    /// 残り牌数を強調する枚数（以下）
    pub wall_warn_at: u32,
}

impl Default for PresentationPrefs {
    fn default() -> Self {
        PresentationPrefs {
            win_secs: 10.0,
            score_anim_secs: 1.2,
            show_all_last: true,
            kyotaku_as_points: true,
            wall_warn_at: 4,
        }
    }
}

/// キーの割り当て（キーの名前で保存）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub wall_down: String,
    pub wall_up: String,
    /// Ctrl と一緒に押す
    pub undo: String,
    /// Ctrl と一緒に押す
    pub redo: String,
    pub face: [String; 4],
    pub face_auto: String,
    pub replay_back: String,
    pub replay_forward: String,
    pub replay_play: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts {
            wall_down: "NumpadSubtract".into(),
            wall_up: "NumpadAdd".into(),
            undo: "KeyZ".into(),
            redo: "KeyY".into(),
            face: ["F1", "F2", "F3", "F4"].map(String::from),
            face_auto: "F5".into(),
            replay_back: "ArrowLeft".into(),
            replay_forward: "ArrowRight".into(),
            replay_play: "Space".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerScreenPrefs {
    /// 画面の下に来る席（0〜3）
    pub bottom: u8,
    pub show_others: bool,
    pub show_diff: bool,
    /// 卓配置（false なら一覧）
    pub table_layout: bool,
}

impl Default for PlayerScreenPrefs {
    fn default() -> Self {
        PlayerScreenPrefs {
            bottom: 0,
            show_others: true,
            show_diff: true,
            table_layout: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayPrefs {
    pub viewpoint: u8,
    pub open_all: bool,
    pub speed: f64,
    /// 牌譜ファイルの既定のフォルダ
    pub folder: String,
}

impl Default for ReplayPrefs {
    fn default() -> Self {
        ReplayPrefs {
            viewpoint: 0,
            open_all: true,
            speed: 1.0,
            folder: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prefs {
    pub layout: LayoutPrefs,
    pub theme: ThemePrefs,
    pub presentation: PresentationPrefs,
    pub shortcuts: Shortcuts,
    pub player_screen: PlayerScreenPrefs,
    pub replay: ReplayPrefs,
    pub update: UpdatePrefs,
}

/// 更新（設定ウィンドウの「更新」タブ）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePrefs {
    /// 起動時に新しい版を確かめる（確かめるだけ）
    pub check_on_start: bool,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self {
            check_on_start: true,
        }
    }
}

/// 割り当てられるキー（名前 ↔ KeyCode）。
pub const KEYS: &[(&str, KeyCode)] = &[
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("Numpad0", KeyCode::Numpad0),
    ("Numpad1", KeyCode::Numpad1),
    ("Numpad2", KeyCode::Numpad2),
    ("Numpad3", KeyCode::Numpad3),
    ("Numpad4", KeyCode::Numpad4),
    ("Numpad5", KeyCode::Numpad5),
    ("Numpad6", KeyCode::Numpad6),
    ("Numpad7", KeyCode::Numpad7),
    ("Numpad8", KeyCode::Numpad8),
    ("Numpad9", KeyCode::Numpad9),
    ("NumpadAdd", KeyCode::NumpadAdd),
    ("NumpadSubtract", KeyCode::NumpadSubtract),
    ("NumpadMultiply", KeyCode::NumpadMultiply),
    ("NumpadDivide", KeyCode::NumpadDivide),
    ("NumpadEnter", KeyCode::NumpadEnter),
    ("NumpadDecimal", KeyCode::NumpadDecimal),
    ("Digit0", KeyCode::Digit0),
    ("Digit1", KeyCode::Digit1),
    ("Digit2", KeyCode::Digit2),
    ("Digit3", KeyCode::Digit3),
    ("Digit4", KeyCode::Digit4),
    ("Digit5", KeyCode::Digit5),
    ("Digit6", KeyCode::Digit6),
    ("Digit7", KeyCode::Digit7),
    ("Digit8", KeyCode::Digit8),
    ("Digit9", KeyCode::Digit9),
    ("KeyA", KeyCode::KeyA),
    ("KeyB", KeyCode::KeyB),
    ("KeyC", KeyCode::KeyC),
    ("KeyD", KeyCode::KeyD),
    ("KeyE", KeyCode::KeyE),
    ("KeyF", KeyCode::KeyF),
    ("KeyG", KeyCode::KeyG),
    ("KeyH", KeyCode::KeyH),
    ("KeyI", KeyCode::KeyI),
    ("KeyJ", KeyCode::KeyJ),
    ("KeyK", KeyCode::KeyK),
    ("KeyL", KeyCode::KeyL),
    ("KeyM", KeyCode::KeyM),
    ("KeyN", KeyCode::KeyN),
    ("KeyO", KeyCode::KeyO),
    ("KeyP", KeyCode::KeyP),
    ("KeyQ", KeyCode::KeyQ),
    ("KeyR", KeyCode::KeyR),
    ("KeyS", KeyCode::KeyS),
    ("KeyT", KeyCode::KeyT),
    ("KeyU", KeyCode::KeyU),
    ("KeyV", KeyCode::KeyV),
    ("KeyW", KeyCode::KeyW),
    ("KeyX", KeyCode::KeyX),
    ("KeyY", KeyCode::KeyY),
    ("KeyZ", KeyCode::KeyZ),
    ("ArrowLeft", KeyCode::ArrowLeft),
    ("ArrowRight", KeyCode::ArrowRight),
    ("ArrowUp", KeyCode::ArrowUp),
    ("ArrowDown", KeyCode::ArrowDown),
    ("Space", KeyCode::Space),
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("Insert", KeyCode::Insert),
    ("Delete", KeyCode::Delete),
    ("Pause", KeyCode::Pause),
    ("ScrollLock", KeyCode::ScrollLock),
];

pub fn key_code(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

pub fn key_name(code: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(_, k)| *k == code).map(|(n, _)| *n)
}

/// 表示用のキー名。
pub fn key_label(name: &str) -> String {
    let s = name
        .replace("NumpadSubtract", "テンキー −")
        .replace("NumpadAdd", "テンキー +")
        .replace("NumpadMultiply", "テンキー ×")
        .replace("NumpadDivide", "テンキー ÷")
        .replace("NumpadEnter", "テンキー Enter")
        .replace("NumpadDecimal", "テンキー .")
        .replace("Numpad", "テンキー ")
        .replace("Digit", "")
        .replace("Key", "")
        .replace("ArrowLeft", "←")
        .replace("ArrowRight", "→")
        .replace("ArrowUp", "↑")
        .replace("ArrowDown", "↓");
    if s.is_empty() {
        "（なし）".into()
    } else {
        s
    }
}

/// 押されたか（割り当てのないキーは押されない扱い）。
pub fn pressed(keys: &ButtonInput<KeyCode>, name: &str) -> bool {
    key_code(name).is_some_and(|k| keys.just_pressed(k))
}

impl Shortcuts {
    /// 同じキーが2つ以上に割り当てられていないか。
    pub fn duplicates(&self) -> Vec<String> {
        let mut all: Vec<&str> = vec![
            &self.wall_down,
            &self.wall_up,
            &self.face_auto,
            &self.replay_back,
            &self.replay_forward,
            &self.replay_play,
        ];
        all.extend(self.face.iter().map(|s| s.as_str()));
        let mut dup = Vec::new();
        for (i, a) in all.iter().enumerate() {
            if !a.is_empty() && all[..i].contains(a) && !dup.contains(&a.to_string()) {
                dup.push(a.to_string());
            }
        }
        dup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_defaults_have_no_duplicates() {
        for (n, k) in KEYS {
            assert_eq!(key_code(n), Some(*k));
            assert_eq!(key_name(*k), Some(*n));
        }
        let s = Shortcuts::default();
        assert!(s.duplicates().is_empty());
        assert!(key_code(&s.wall_down).is_some());
        let mut d = s.clone();
        d.wall_up = d.face_auto.clone();
        assert_eq!(d.duplicates(), vec!["F5".to_string()]);
    }

    #[test]
    fn theme_roundtrip() {
        let t = ThemePrefs::default().to_theme();
        assert_eq!(rgb8(t.bg), ThemePrefs::default().bg);
    }
}

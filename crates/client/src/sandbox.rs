//! 映像サンドボックス（入力担当のウィンドウ）。メディアサーバーに届いた映像を全部キャンバスに並べ、枠を置いて使い道を決める。
//!
//! - キャンバス: 中ボタンのドラッグで移動、ホイールで拡大・縮小
//! - ソースをクリック: 再生（動画）とキャプチャ画（1枚）を切り替える。右クリックで「映像なし」なども選べる（軽くするため）
//! - ソースの上を左ドラッグ: 枠を置く（使い道は上で選んでおく）。枠をクリックで選び、右の欄で使い道・位置を直す
//! - 「メディアサーバーに保存」で枠・ソースを送ると、メディアサーバーが枠で切り出した映像を各処理に回す
//!   （選手顔 → 配信画面の選手画面。手牌確認・河/鳴き確認 → 牌の自動読み取り・確認用）

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use janscope_video::sandbox::{
    MediaStatus, Region, RegionRole, SandboxConfig, SourceDef, SourceKind, safe_id,
};
use janscope_video::{MjpegSplitter, RtmpMode, http};

use crate::config::{ClientConfig, FaceOutput};
use crate::face::FaceRuntime;

/// 選手顔（席 seat）の切り出し映像の URL
pub fn face_url(media_url: &str, seat: u8) -> String {
    format!(
        "{}/region/{}.mjpg",
        media_url.trim_end_matches('/'),
        RegionRole::Face(seat).id()
    )
}

/// ソースの見せ方（軽くするため選べる）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    /// 動画（5fps の縮小映像）
    Play,
    /// 1枚だけ（撮り直せる）
    Capture,
    /// 映像なし
    None,
}

type Slot = Arc<Mutex<Option<(u64, Arc<Vec<u8>>)>>>;

/// 再生中のソースの映像を受け続ける（drop で止まる）
struct Player {
    stop: Arc<AtomicBool>,
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn play(url: String, slot: Slot) -> Player {
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    std::thread::spawn(move || {
        let mut seq = 0u64;
        while !st.load(Ordering::SeqCst) {
            if let Ok(mut r) = http::stream(&url, Duration::from_secs(2)) {
                let mut sp = MjpegSplitter::default();
                let mut buf = vec![0u8; 128 * 1024];
                while !st.load(Ordering::SeqCst) {
                    use std::io::Read;
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            for j in sp.push(&buf[..n]) {
                                seq += 1;
                                *slot.lock().unwrap() = Some((seq, Arc::new(j)));
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    });
    Player { stop }
}

fn snapshot(url: String, slot: Slot) {
    std::thread::spawn(move || {
        if let Ok(j) = http::get(&url, Duration::from_secs(3)) {
            let seq = slot.lock().unwrap().as_ref().map(|s| s.0).unwrap_or(0) + 1;
            *slot.lock().unwrap() = Some((seq, Arc::new(j)));
        }
    });
}

/// 様子を1秒ごとに取ってくる
struct Poller {
    url: Arc<Mutex<String>>,
    status: Arc<Mutex<Option<Result<MediaStatus, String>>>>,
    stop: Arc<AtomicBool>,
}

impl Poller {
    fn start(url: String) -> Poller {
        let p = Poller {
            url: Arc::new(Mutex::new(url)),
            status: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (u, s, st) = (p.url.clone(), p.status.clone(), p.stop.clone());
        std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                let base = u.lock().unwrap().clone();
                let r = http::get(
                    &format!("{}/status", base.trim_end_matches('/')),
                    Duration::from_millis(1500),
                )
                .and_then(|b| serde_json::from_slice::<MediaStatus>(&b).map_err(|e| e.to_string()));
                *s.lock().unwrap() = Some(r);
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        p
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddKind {
    Webcam,
    Image,
    Video,
    Url,
}

#[derive(Resource)]
pub struct SandboxUi {
    pub open: bool,
    poller: Option<Poller>,
    draft: Option<SandboxConfig>,
    dirty: bool,
    view: HashMap<String, View>,
    players: HashMap<String, Player>,
    slots: HashMap<String, Slot>,
    tex: HashMap<String, (u64, egui::TextureHandle)>,
    pan: egui::Vec2,
    zoom: f32,
    /// 枠を置いている途中（ソース, 始点の割合）
    drag: Option<(String, egui::Pos2)>,
    selected: Option<RegionRole>,
    new_role: RegionRole,
    menu_target: Option<String>,
    add_kind: AddKind,
    add_text: String,
    add_name: String,
    devices: Arc<Mutex<Option<Vec<String>>>>,
    msg: Arc<Mutex<Option<String>>>,
    saved: Arc<Mutex<Option<SandboxConfig>>>,
}

impl Default for SandboxUi {
    fn default() -> Self {
        SandboxUi {
            // JANSCOPE_OPEN_SANDBOX で最初から開く（確認用）
            open: std::env::var_os("JANSCOPE_OPEN_SANDBOX").is_some(),
            poller: None,
            draft: None,
            dirty: false,
            view: HashMap::new(),
            players: HashMap::new(),
            slots: HashMap::new(),
            tex: HashMap::new(),
            pan: egui::vec2(20.0, 20.0),
            zoom: 0.8,
            drag: None,
            selected: None,
            new_role: RegionRole::Face(0),
            menu_target: None,
            add_kind: AddKind::Webcam,
            add_text: String::new(),
            add_name: String::new(),
            devices: Arc::new(Mutex::new(None)),
            msg: Arc::new(Mutex::new(None)),
            saved: Arc::new(Mutex::new(None)),
        }
    }
}

const TILE_W: f32 = 640.0;
const TILE_H: f32 = 360.0;
const HEAD: f32 = 26.0;
const GAP: f32 = 30.0;
const COLS: usize = 2;

fn role_color(r: RegionRole) -> egui::Color32 {
    match r {
        RegionRole::Face(_) => egui::Color32::from_rgb(90, 200, 255),
        RegionRole::Hand(_) => egui::Color32::from_rgb(255, 200, 70),
        RegionRole::River(_) => egui::Color32::from_rgb(120, 230, 120),
        RegionRole::Board => egui::Color32::from_rgb(230, 120, 230),
    }
}

impl SandboxUi {
    fn slot(&mut self, id: &str) -> Slot {
        self.slots.entry(id.to_string()).or_default().clone()
    }

    fn set_view(&mut self, base: &str, id: &str, v: View) {
        self.view.insert(id.to_string(), v);
        self.players.remove(id);
        let slot = self.slot(id);
        match v {
            View::Play => {
                let p = play(format!("{base}/preview/{id}.mjpg"), slot);
                self.players.insert(id.to_string(), p);
            }
            View::Capture => snapshot(format!("{base}/snapshot/{id}.jpg"), slot),
            View::None => {
                *slot.lock().unwrap() = None;
                self.tex.remove(id);
            }
        }
    }

    /// 使っていない使い道（新しい枠に勧める）
    fn next_role(&self) -> RegionRole {
        let used: Vec<RegionRole> = self
            .draft
            .as_ref()
            .map(|d| d.regions.iter().map(|r| r.role).collect())
            .unwrap_or_default();
        RegionRole::ALL
            .into_iter()
            .find(|r| !used.contains(r))
            .unwrap_or(RegionRole::Board)
    }
}

/// 毎フレーム: ウィンドウを描く
pub fn sandbox_ui(
    mut contexts: EguiContexts,
    mut sb: ResMut<SandboxUi>,
    mut cfg: ResMut<ClientConfig>,
    mut face: ResMut<FaceRuntime>,
) -> Result {
    let sb = &mut *sb;
    if !sb.open {
        if sb.poller.is_some() {
            // 閉じたら止める（軽くする）
            sb.poller = None;
            sb.players.clear();
        }
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let base = cfg.media_url.trim_end_matches('/').to_string();
    if sb.poller.is_none() {
        sb.poller = Some(Poller::start(base.clone()));
    }
    if let Some(p) = &sb.poller {
        *p.url.lock().unwrap() = base.clone();
    }
    let status = sb
        .poller
        .as_ref()
        .and_then(|p| p.status.lock().unwrap().clone());
    // 保存した結果・まだ変えていなければメディアサーバーの設定を下書きにする
    if let Some(c) = sb.saved.lock().unwrap().take() {
        sb.draft = Some(c);
        sb.dirty = false;
    }
    if let Some(Ok(st)) = &status {
        if !sb.dirty {
            sb.draft = Some(st.config.clone());
        }
        // 初めて見るソースはキャプチャ画（軽い）にする
        for s in &st.sources {
            if !sb.view.contains_key(&s.id) && s.live {
                let id = s.id.clone();
                sb.set_view(&base, &id, View::Capture);
            }
        }
    }
    // 届いた映像を絵にする
    let ids: Vec<String> = sb.slots.keys().cloned().collect();
    for id in ids {
        let frame = sb.slots[&id].lock().unwrap().clone();
        let Some((seq, jpeg)) = frame else { continue };
        if sb.tex.get(&id).is_some_and(|(s, _)| *s == seq) {
            continue;
        }
        if let Ok((w, h, px)) = janscope_video::decode_rgba(&jpeg) {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
            match sb.tex.get_mut(&id) {
                Some((s, t)) => {
                    t.set(img, egui::TextureOptions::LINEAR);
                    *s = seq;
                }
                None => {
                    let t = ctx.load_texture(format!("sb-{id}"), img, egui::TextureOptions::LINEAR);
                    sb.tex.insert(id.clone(), (seq, t));
                }
            }
        }
    }
    let mut open = sb.open;
    egui::Window::new("映像サンドボックス")
        .open(&mut open)
        .default_size([1300.0, 820.0])
        .resizable(true)
        .show(&ctx, |ui| {
            toolbar(ui, sb, &mut cfg, &mut face, &status, &base);
            ui.separator();
            ui.horizontal_top(|ui| {
                let side_w = 300.0;
                let avail = ui.available_size();
                let canvas_size =
                    egui::vec2((avail.x - side_w - 12.0).max(300.0), avail.y.max(300.0));
                canvas(ui, sb, &status, &base, canvas_size);
                ui.vertical(|ui| {
                    ui.set_width(side_w);
                    side_panel(ui, sb, &status);
                });
            });
        });
    sb.open = open;
    if !sb.open {
        sb.poller = None;
        sb.players.clear();
    }
    ctx.request_repaint_after(Duration::from_millis(100));
    Ok(())
}

fn toolbar(
    ui: &mut egui::Ui,
    sb: &mut SandboxUi,
    cfg: &mut ClientConfig,
    face: &mut FaceRuntime,
    status: &Option<Result<MediaStatus, String>>,
    base: &str,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label("メディアサーバー");
        let r = ui.add(egui::TextEdit::singleline(&mut cfg.media_url).desired_width(220.0));
        if r.lost_focus() {
            let _ = cfg.save();
        }
        match status {
            Some(Ok(s)) => {
                ui.colored_label(
                    egui::Color32::LIGHT_GREEN,
                    format!(
                        "● つながっています（{} ・ソース {} ・RTMP {}）",
                        s.version,
                        s.sources.len(),
                        s.rtmp_port
                    ),
                );
                if let Some(e) = &s.rtmp_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 140, 100), e);
                }
            }
            Some(Err(e)) => {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 140, 100),
                    format!("■ つながりません（ランチャーの「メディアサーバー」を起動してください）: {e}"),
                );
            }
            None => {
                ui.label("つないでいます…");
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("新しい枠の使い道:");
        egui::ComboBox::from_id_salt("sb_new_role")
            .selected_text(sb.new_role.label())
            .show_ui(ui, |ui| {
                for r in RegionRole::ALL {
                    ui.selectable_value(&mut sb.new_role, r, r.label());
                }
            });
        ui.weak("（ソースの上を左ドラッグで枠を置く・中ボタンで移動・ホイールで拡大・クリックで再生／キャプチャ）");
    });
    ui.horizontal_wrapped(|ui| {
        let can_save = sb.dirty && sb.draft.is_some();
        if ui
            .add_enabled(can_save, egui::Button::new("メディアサーバーに保存"))
            .on_hover_text(
                "枠とソースをメディアサーバーに送ります。切り出した映像が各処理に回ります",
            )
            .clicked()
        {
            if let Some(d) = sb.draft.clone() {
                let (url, msg, saved) =
                    (format!("{base}/config"), sb.msg.clone(), sb.saved.clone());
                std::thread::spawn(move || {
                    let body = serde_json::to_vec(&d).unwrap_or_default();
                    let r = http::put_json(&url, &body, Duration::from_secs(5));
                    *msg.lock().unwrap() = Some(match &r {
                        Ok(_) => "メディアサーバーに保存しました".into(),
                        Err(e) => format!("保存できませんでした: {e}"),
                    });
                    if r.is_ok() {
                        *saved.lock().unwrap() = Some(d);
                    }
                });
            }
        }
        if sb.dirty {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 80),
                "保存していない変更があります",
            );
            if ui.button("変更をやめる").clicked() {
                sb.dirty = false;
            }
        }
        ui.separator();
        if ui
            .button("顔カメラをメディアサーバーから受ける")
            .on_hover_text("席A〜Dの顔カメラを、この枠（選手顔 席A〜D）の切り出し映像にします")
            .clicked()
        {
            cfg.face_output = FaceOutput::Video;
            for i in 0..4 {
                cfg.faces[i].enabled = true;
                cfg.faces[i].mode = RtmpMode::Media;
                cfg.faces[i].url = face_url(&cfg.media_url, i as u8);
            }
            match cfg.save() {
                Ok(()) => {
                    *sb.msg.lock().unwrap() =
                        Some("顔カメラをメディアサーバーから受けるようにしました".into())
                }
                Err(e) => *sb.msg.lock().unwrap() = Some(e),
            }
            face.apply(cfg);
        }
        if let Some(m) = sb.msg.lock().unwrap().clone() {
            ui.label(m);
        }
    });
    // ソースを足す
    ui.horizontal_wrapped(|ui| {
        ui.label("ソースを足す:");
        ui.selectable_value(&mut sb.add_kind, AddKind::Webcam, "Webカメラ");
        ui.selectable_value(&mut sb.add_kind, AddKind::Image, "画像");
        ui.selectable_value(&mut sb.add_kind, AddKind::Video, "映像");
        ui.selectable_value(&mut sb.add_kind, AddKind::Url, "URL");
        match sb.add_kind {
            AddKind::Webcam => {
                let devs = sb.devices.lock().unwrap().clone();
                egui::ComboBox::from_id_salt("sb_dev")
                    .selected_text(if sb.add_text.is_empty() {
                        "（デバイスを選ぶ）".to_string()
                    } else {
                        sb.add_text.clone()
                    })
                    .show_ui(ui, |ui| {
                        for d in devs.unwrap_or_default() {
                            ui.selectable_value(&mut sb.add_text, d.clone(), d);
                        }
                    });
                if ui.button("探す").clicked() {
                    let (url, devs) = (format!("{base}/devices"), sb.devices.clone());
                    std::thread::spawn(move || {
                        let v = http::get(&url, Duration::from_secs(10))
                            .ok()
                            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
                            .unwrap_or_default();
                        *devs.lock().unwrap() = Some(v);
                    });
                }
            }
            _ => {
                ui.add(
                    egui::TextEdit::singleline(&mut sb.add_text)
                        .hint_text(match sb.add_kind {
                            AddKind::Url => "rtmp://… / http://…",
                            _ => "ファイルの場所（メディアサーバーのPCの）",
                        })
                        .desired_width(300.0),
                );
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut sb.add_name)
                .hint_text("名前")
                .desired_width(120.0),
        );
        if ui
            .add_enabled(!sb.add_text.trim().is_empty(), egui::Button::new("足す"))
            .clicked()
        {
            let t = sb.add_text.trim().to_string();
            let (prefix, kind) = match sb.add_kind {
                AddKind::Webcam => ("cam", SourceKind::Webcam { device: t.clone() }),
                AddKind::Image => ("img", SourceKind::Image { path: t.clone() }),
                AddKind::Video => ("vid", SourceKind::Video { path: t.clone() }),
                AddKind::Url => ("url", SourceKind::Url { url: t.clone() }),
            };
            let d = sb.draft.get_or_insert_with(SandboxConfig::default);
            let mut n = 1;
            while d.sources.iter().any(|s| s.id == format!("{prefix}-{n}")) {
                n += 1;
            }
            let name = if sb.add_name.trim().is_empty() {
                t.rsplit(['/', '\\']).next().unwrap_or(&t).to_string()
            } else {
                sb.add_name.trim().to_string()
            };
            d.sources.push(SourceDef {
                id: safe_id(&format!("{prefix}-{n}")),
                name,
                kind,
            });
            sb.dirty = true;
            sb.add_text.clear();
            sb.add_name.clear();
        }
    });
}

/// キャンバス（ソースを並べて枠を置く）
fn canvas(
    ui: &mut egui::Ui,
    sb: &mut SandboxUi,
    status: &Option<Result<MediaStatus, String>>,
    base: &str,
    size: egui::Vec2,
) {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
    // 中ボタンで移動、ホイールで拡大・縮小
    let (middle, delta, scroll, pointer) = ui.input(|i| {
        (
            i.pointer.middle_down(),
            i.pointer.delta(),
            i.smooth_scroll_delta.y,
            i.pointer.hover_pos(),
        )
    });
    if resp.hovered() || resp.dragged() {
        if middle {
            sb.pan += delta / sb.zoom;
        }
        if scroll != 0.0 {
            if let Some(p) = pointer {
                let before = (p - rect.min) / sb.zoom - sb.pan;
                sb.zoom = (sb.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 4.0);
                let after = (p - rect.min) / sb.zoom - sb.pan;
                sb.pan += after - before;
            }
        }
    }
    let zoom = sb.zoom;
    let pan = sb.pan;
    let to_screen = |w: egui::Pos2| rect.min + (w.to_vec2() + pan) * zoom;
    let sources: Vec<(String, String, String, bool, f32, Option<String>)> = match status {
        Some(Ok(s)) => s
            .sources
            .iter()
            .map(|x| {
                (
                    x.id.clone(),
                    x.name.clone(),
                    x.kind.clone(),
                    x.live,
                    x.fps,
                    x.error.clone(),
                )
            })
            .collect(),
        _ => vec![],
    };
    if sources.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "ソースがありません（カメラから RTMP で送るか、上の「ソースを足す」で足して保存してください）",
            egui::FontId::proportional(16.0),
            egui::Color32::GRAY,
        );
    }
    // ソースの置き場所（world 座標）と、映像を描いた場所（screen）
    let mut image_rects: Vec<(String, egui::Rect)> = vec![];
    for (i, (id, name, kind, live, fps, err)) in sources.iter().enumerate() {
        let col = (i % COLS) as f32;
        let row = (i / COLS) as f32;
        let w0 = egui::pos2(col * (TILE_W + GAP), row * (TILE_H + HEAD + GAP));
        let head =
            egui::Rect::from_min_max(to_screen(w0), to_screen(w0 + egui::vec2(TILE_W, HEAD)));
        let area = egui::Rect::from_min_max(
            to_screen(w0 + egui::vec2(0.0, HEAD)),
            to_screen(w0 + egui::vec2(TILE_W, HEAD + TILE_H)),
        );
        let view = sb.view.get(id).copied().unwrap_or(View::None);
        painter.rect_filled(head, 4.0, egui::Color32::from_gray(50));
        let vlabel = match view {
            View::Play => "再生",
            View::Capture => "キャプチャ画",
            View::None => "映像なし",
        };
        painter.text(
            head.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!(
                "{name}（{kind}） {} {:.0}fps ・{vlabel}",
                if *live { "●" } else { "○" },
                fps
            ),
            egui::FontId::proportional((14.0 * zoom).clamp(9.0, 22.0)),
            if *live {
                egui::Color32::WHITE
            } else {
                egui::Color32::GRAY
            },
        );
        painter.rect_filled(area, 0.0, egui::Color32::BLACK);
        // 映像（縦横比を保って枠の中に）
        let img_rect = match (view, sb.tex.get(id)) {
            (View::Play | View::Capture, Some((_, t))) => {
                let [tw, th] = t.size();
                let s = (area.width() / tw as f32).min(area.height() / th as f32);
                let r = egui::Rect::from_center_size(
                    area.center(),
                    egui::vec2(tw as f32 * s, th as f32 * s),
                );
                painter.image(
                    t.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                r
            }
            _ => {
                painter.text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    if view == View::None {
                        "映像なし（クリックでキャプチャ画）"
                    } else {
                        "映像を待っています"
                    },
                    egui::FontId::proportional(14.0),
                    egui::Color32::GRAY,
                );
                area
            }
        };
        if let Some(e) = err {
            painter.text(
                area.left_bottom() + egui::vec2(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                e,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(255, 140, 100),
            );
        }
        // 枠
        if let Some(d) = &sb.draft {
            for r in d.regions.iter().filter(|r| r.source == *id) {
                let rr = egui::Rect::from_min_size(
                    img_rect.min + egui::vec2(r.x * img_rect.width(), r.y * img_rect.height()),
                    egui::vec2(r.w * img_rect.width(), r.h * img_rect.height()),
                );
                let c = role_color(r.role);
                let sel = sb.selected == Some(r.role);
                painter.rect_stroke(
                    rr,
                    0.0,
                    egui::Stroke::new(if sel { 3.0 } else { 1.5 }, c),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    rr.min + egui::vec2(4.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    r.role.label(),
                    egui::FontId::proportional(13.0),
                    c,
                );
            }
        }
        image_rects.push((id.clone(), img_rect));
    }
    let find = |p: egui::Pos2| image_rects.iter().find(|(_, r)| r.contains(p)).cloned();
    let norm = |r: &egui::Rect, p: egui::Pos2| {
        egui::pos2(
            ((p.x - r.min.x) / r.width()).clamp(0.0, 1.0),
            ((p.y - r.min.y) / r.height()).clamp(0.0, 1.0),
        )
    };
    // 左ドラッグで枠を置く
    if resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
            if let Some((id, r)) = find(p) {
                sb.drag = Some((id, norm(&r, p)));
            }
        }
    }
    if let (Some((id, start)), Some(p)) = (sb.drag.clone(), pointer) {
        if let Some((_, r)) = image_rects.iter().find(|(i, _)| *i == id) {
            let a = egui::pos2(
                r.min.x + start.x * r.width(),
                r.min.y + start.y * r.height(),
            );
            painter.rect_stroke(
                egui::Rect::from_two_pos(a, p),
                0.0,
                egui::Stroke::new(2.0, role_color(sb.new_role)),
                egui::StrokeKind::Inside,
            );
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                let e = norm(r, p);
                let (x0, x1) = (start.x.min(e.x), start.x.max(e.x));
                let (y0, y1) = (start.y.min(e.y), start.y.max(e.y));
                if x1 - x0 > 0.02 && y1 - y0 > 0.02 {
                    let role = sb.new_role;
                    let d = sb.draft.get_or_insert_with(SandboxConfig::default);
                    d.regions.push(Region {
                        source: id.clone(),
                        role,
                        x: x0,
                        y: y0,
                        w: x1 - x0,
                        h: y1 - y0,
                    });
                    d.normalize();
                    sb.dirty = true;
                    sb.selected = Some(role);
                    sb.new_role = sb.next_role();
                }
                sb.drag = None;
            }
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) && !resp.drag_stopped() {
        sb.drag = None;
    }
    // クリック: 枠の上なら選ぶ、それ以外は再生 ↔ キャプチャ画
    if resp.clicked_by(egui::PointerButton::Primary) {
        if let Some(p) = pointer {
            if let Some((id, r)) = find(p) {
                let n = norm(&r, p);
                let hit = sb.draft.as_ref().and_then(|d| {
                    d.regions
                        .iter()
                        .rev()
                        .find(|g| {
                            g.source == id
                                && n.x >= g.x
                                && n.x <= g.x + g.w
                                && n.y >= g.y
                                && n.y <= g.y + g.h
                        })
                        .map(|g| g.role)
                });
                match hit {
                    Some(role) => sb.selected = Some(role),
                    None => {
                        let v = match sb.view.get(&id).copied().unwrap_or(View::None) {
                            View::Play => View::Capture,
                            _ => View::Play,
                        };
                        sb.set_view(base, &id, v);
                    }
                }
            }
        }
    }
    if resp.secondary_clicked() {
        sb.menu_target = pointer.and_then(find).map(|(id, _)| id);
    }
    let target = sb.menu_target.clone();
    resp.context_menu(|ui| {
        let Some(id) = target.clone() else {
            ui.label("ソースの上で右クリックしてください");
            return;
        };
        if ui.button("再生").clicked() {
            sb.set_view(base, &id, View::Play);
            ui.close();
        }
        if ui.button("キャプチャ画（撮り直す）").clicked() {
            sb.set_view(base, &id, View::Capture);
            ui.close();
        }
        if ui.button("映像なし").clicked() {
            sb.set_view(base, &id, View::None);
            ui.close();
        }
        let configured = sb
            .draft
            .as_ref()
            .is_some_and(|d| d.sources.iter().any(|s| s.id == id));
        if configured && ui.button("このソースを消す").clicked() {
            if let Some(d) = sb.draft.as_mut() {
                d.sources.retain(|s| s.id != id);
                d.regions.retain(|r| r.source != id);
            }
            sb.dirty = true;
            ui.close();
        }
    });
}

/// 右の欄: 枠の一覧と、選んだ枠の直し
fn side_panel(ui: &mut egui::Ui, sb: &mut SandboxUi, status: &Option<Result<MediaStatus, String>>) {
    ui.heading("枠");
    let names: HashMap<String, String> = match status {
        Some(Ok(s)) => s
            .sources
            .iter()
            .map(|x| (x.id.clone(), x.name.clone()))
            .collect(),
        _ => HashMap::new(),
    };
    let Some(d) = sb.draft.as_mut() else {
        ui.weak("（メディアサーバーにつながると出ます）");
        return;
    };
    if d.regions.is_empty() {
        ui.weak("まだ枠がありません。ソースの上を左ドラッグで置きます");
    }
    egui::ScrollArea::vertical()
        .id_salt("sb_regions")
        .max_height(260.0)
        .show(ui, |ui| {
            for r in &d.regions {
                let sel = sb.selected == Some(r.role);
                let src = names
                    .get(&r.source)
                    .cloned()
                    .unwrap_or_else(|| r.source.clone());
                if ui
                    .selectable_label(
                        sel,
                        egui::RichText::new(format!("{} ← {src}", r.role.label()))
                            .color(role_color(r.role)),
                    )
                    .clicked()
                {
                    sb.selected = Some(r.role);
                }
            }
        });
    ui.separator();
    let Some(role) = sb.selected else {
        ui.weak("枠をクリックで選ぶと、ここで直せます");
        return;
    };
    let Some(idx) = d.regions.iter().position(|r| r.role == role) else {
        sb.selected = None;
        return;
    };
    let mut changed = false;
    let mut new_role = role;
    ui.label("使い道");
    egui::ComboBox::from_id_salt("sb_sel_role")
        .selected_text(role.label())
        .show_ui(ui, |ui| {
            for r in RegionRole::ALL {
                ui.selectable_value(&mut new_role, r, r.label());
            }
        });
    let g = &mut d.regions[idx];
    egui::Grid::new("sb_pos").show(ui, |ui| {
        for (label, v) in [
            ("左", &mut g.x),
            ("上", &mut g.y),
            ("幅", &mut g.w),
            ("高さ", &mut g.h),
        ] {
            ui.label(label);
            if ui
                .add(
                    egui::DragValue::new(v)
                        .speed(0.002)
                        .range(0.0..=1.0)
                        .fixed_decimals(3),
                )
                .changed()
            {
                changed = true;
            }
            ui.end_row();
        }
    });
    if new_role != role {
        g.role = new_role;
        changed = true;
        sb.selected = Some(new_role);
    }
    let mut delete = false;
    if ui.button("この枠を消す").clicked() {
        delete = true;
    }
    if delete {
        d.regions.remove(idx);
        sb.selected = None;
        changed = true;
    }
    if changed {
        d.normalize();
        sb.dirty = true;
    }
    ui.separator();
    ui.weak("選手顔の枠は配信画面の選手画面に、手牌確認・河/鳴き確認・卓全体の枠は牌の自動読み取り（これから）と確認用に回ります。保存するとメディアサーバーが切り出します");
}

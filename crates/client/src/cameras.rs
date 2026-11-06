//! 操作ウィンドウの「カメラの受信」: カメラに入れる URL の発行、受信の様子、プレビュー、未割り当てのカメラの割り当て。
//!
//! 本ソフトが配信サーバー（RTMP の受け口）になる。カメラは `rtmp://<このPCのIP>:1935/live/<キー>` に送る。

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use bevy_egui::egui;
use janscope_core::tile::SeatId;
use janscope_video::RtmpMode;
use janscope_video::rtmp_server::{PublisherInfo, local_ip, parse_listen_url, server_status};

use crate::config::ClientConfig;
use crate::face::FaceRuntime;

#[derive(Default)]
pub struct CameraUi {
    ip: Option<IpAddr>,
    ip_checked: Option<Instant>,
    msg: Option<String>,
}

impl CameraUi {
    fn ip(&mut self) -> Option<IpAddr> {
        if self
            .ip_checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(5))
        {
            self.ip = local_ip();
            self.ip_checked = Some(Instant::now());
        }
        self.ip
    }
}

/// 待ち受けの URL（rtmp://0.0.0.0:<port>/live/<key>）のポートとキー
fn port_key(url: &str) -> (u16, String, String) {
    parse_listen_url(url).unwrap_or((1935, "live".into(), String::new()))
}

fn listen_url(port: u16, app: &str, key: &str) -> String {
    format!("rtmp://0.0.0.0:{port}/{app}/{key}")
}

fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 64
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn describe(p: &PublisherInfo) -> String {
    let mut parts = vec![];
    match (p.width, p.height) {
        (Some(w), Some(h)) => parts.push(format!("{w}×{h}")),
        _ => parts.push("解像度不明".into()),
    }
    if let Some(f) = p.fps {
        parts.push(format!("{f:.0}fps"));
    }
    parts.push(if p.kbps > 0 {
        format!("{} kbps", p.kbps)
    } else {
        "計測中".into()
    });
    parts.push(format!("{}", p.remote.ip()));
    parts.join(" ・ ")
}

/// 待ち受けの設定と受信の様子。`d` は編集中の設定（「適用」で反映）
pub fn camera_section(
    ui: &mut egui::Ui,
    st: &mut CameraUi,
    d: &mut ClientConfig,
    applied: &ClientConfig,
    face: &FaceRuntime,
    tex: &HashMap<String, egui::TextureId>,
) {
    let ip = st.ip();
    let ip_text = ip
        .map(|i| i.to_string())
        .unwrap_or_else(|| "<このPCのIP>".into());
    // 待ち受けのポート（全部の席で共通）
    let listen: Vec<usize> = (0..4)
        .filter(|&i| d.faces[i].mode == RtmpMode::Listen)
        .collect();
    if listen.is_empty() {
        return;
    }
    let mut port = port_key(&d.faces[listen[0]].url).0;

    ui.separator();
    ui.label(egui::RichText::new("カメラの受信（このソフトが配信サーバーになります）").strong());
    ui.horizontal(|ui| {
        ui.label(format!("このPCの IP: {ip_text}"));
        ui.label("ポート");
        if ui
            .add(egui::DragValue::new(&mut port).range(1024..=65535))
            .changed()
        {
            for &i in &listen {
                let (_, app, key) = port_key(&d.faces[i].url);
                d.faces[i].url = listen_url(port, &app, &key);
            }
        }
    });
    // 受け口の様子（適用済みの設定のポート）
    let applied_port = (0..4)
        .find(|&i| applied.faces[i].mode == RtmpMode::Listen && applied.faces[i].enabled)
        .map(|i| port_key(&applied.faces[i].url).0);
    let status = applied_port.and_then(server_status);
    match (&status, applied_port) {
        (Some(s), _) if s.bind_error.is_some() => {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 100),
                s.bind_error.clone().unwrap(),
            );
        }
        (Some(s), _) => {
            ui.colored_label(
                egui::Color32::LIGHT_GREEN,
                format!("ポート {} で待ち受けています", s.port),
            );
        }
        (None, _) => {
            ui.weak("「使う」にチェックして「適用」すると待ち受けを始めます");
        }
    }
    let pubs: Vec<PublisherInfo> = status
        .as_ref()
        .map(|s| s.publishers.clone())
        .unwrap_or_default();

    for &i in &listen {
        let seat = SeatId(i as u8);
        let (p, app, mut key) = port_key(&d.faces[i].url);
        let cam_url = format!("rtmp://{ip_text}:{p}/{app}/{key}");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(seat.label().to_string()).strong());
            ui.label("キー");
            let r = ui
                .add(egui::TextEdit::singleline(&mut key).desired_width(80.0))
                .on_hover_text("半角の英数字と - _。カメラ側と同じにします");
            if r.changed() {
                key.retain(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                d.faces[i].url = listen_url(p, &app, &key);
            }
            ui.monospace(&cam_url);
            if ui
                .small_button("コピー")
                .on_hover_text("カメラの RTMP URL にこのまま入れます（GoPro は Quik アプリのライブ配信の設定）")
                .clicked()
            {
                ui.ctx().copy_text(cam_url.clone());
                st.msg = Some(format!("{} の URL をコピーしました", seat.label()));
            }
        });
        let pubinfo = pubs.iter().find(|x| x.app == app && x.key == key);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(18.0);
            if !d.faces[i].enabled {
                ui.weak("使っていません（上の「使う」にチェック）");
            } else if !valid_key(&key) {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "キーは半角の英数字と - _ で入れてください",
                );
            } else {
                let s = face.streams[i].as_ref().map(|s| (s.status(), s.fps()));
                match (s, pubinfo) {
                    (Some((janscope_video::StreamStatus::Live, fps)), Some(p)) => {
                        ui.colored_label(egui::Color32::LIGHT_GREEN, format!("受信中 {fps:.0}fps"));
                        ui.weak(describe(p));
                    }
                    (_, Some(p)) => {
                        ui.colored_label(egui::Color32::YELLOW, "届いています（映し始めています）");
                        ui.weak(describe(p));
                    }
                    (Some((janscope_video::StreamStatus::Failed(e), _)), None) => {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 100), e);
                    }
                    (Some(_), None) => {
                        ui.colored_label(egui::Color32::GRAY, "カメラを待っています");
                    }
                    (None, None) => {
                        ui.weak("「適用」すると待ち受けます");
                    }
                }
            }
        });
    }

    // 小さなプレビュー（受信中の席だけ）
    let live: Vec<usize> = listen
        .iter()
        .copied()
        .filter(|&i| face.live(SeatId(i as u8)))
        .collect();
    if !live.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for i in live {
                if let Some(id) = tex.get(&format!("face{i}")) {
                    ui.vertical(|ui| {
                        ui.label(SeatId(i as u8).label().to_string());
                        ui.image(egui::load::SizedTexture::new(*id, egui::vec2(192.0, 108.0)));
                    });
                }
            }
        });
    }

    // どの席にも割り当てていないキーから届いているカメラ
    let used: Vec<(String, String)> = listen
        .iter()
        .map(|&i| {
            let (_, a, k) = port_key(&d.faces[i].url);
            (a, k)
        })
        .collect();
    let stray: Vec<&PublisherInfo> = pubs
        .iter()
        .filter(|p| !used.iter().any(|(a, k)| *a == p.app && *k == p.key))
        .collect();
    for p in stray {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 80),
                format!(
                    "割り当てていないカメラ: キー「{}」（{}）",
                    p.key,
                    describe(p)
                ),
            );
            for &i in &listen {
                if ui
                    .small_button(format!("{}に割り当てる", SeatId(i as u8).label()))
                    .clicked()
                {
                    let (port, _, _) = port_key(&d.faces[i].url);
                    d.faces[i].url = listen_url(port, &p.app, &p.key);
                    d.faces[i].enabled = true;
                    st.msg = Some(format!(
                        "キー「{}」を{}に割り当てました。「適用」で反映します",
                        p.key,
                        SeatId(i as u8).label()
                    ));
                }
            }
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.weak("ほかの機器からつながらないとき: このPCのファイアウォールで、このソフト（または TCP のポート）を許可してください。");
        #[cfg(windows)]
        if ui.small_button("ファイアウォールに許可を追加…").on_hover_text("Windows の確認画面（管理者）が出ます").clicked() {
            st.msg = Some(match add_firewall_rule(port) {
                Ok(()) => "許可の追加を頼みました（確認画面で「はい」を押してください）".into(),
                Err(e) => e,
            });
        }
    });
    if let Some(m) = &st.msg {
        ui.label(m.clone());
    }
}

/// Windows のファイアウォールに、RTMP のポートを受けてよい規則を足す（管理者の確認画面を出す）
#[cfg(windows)]
fn add_firewall_rule(port: u16) -> Result<(), String> {
    let args = format!(
        "advfirewall firewall add rule name=\"JanScope camera RTMP {port}\" dir=in action=allow protocol=TCP localport={port} profile=private,domain"
    );
    let ps = format!(
        "Start-Process -FilePath netsh -Verb RunAs -WindowStyle Hidden -ArgumentList '{args}'"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("ファイアウォールの設定を開けません: {e}"))
}

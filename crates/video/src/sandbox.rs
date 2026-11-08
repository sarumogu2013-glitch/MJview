//! 映像サンドボックスの設定（メディアサーバーと入力担当で共通）。
//!
//! - 入力ソース: RTMP（内蔵の受け口に届いたもの。自動で並ぶ）・Webカメラ・画像・映像ファイル・URL
//! - 枠（Region）: ソースの映像のどこを、何に使うか（選手顔・手牌確認・河/鳴き確認・卓全体）。位置は映像に対する割合（0〜1）
//! - メディアサーバーは枠で切り出した映像を `/region/<枠の名前>.mjpg` で出す。入力担当はそれを顔カメラ・確認用に使う

use serde::{Deserialize, Serialize};

/// 既定の、メディアサーバーの HTTP（入力担当が映像を受け取る口）
pub const DEFAULT_MEDIA_URL: &str = "http://127.0.0.1:47810";
pub const DEFAULT_HTTP_PORT: u16 = 47810;
pub const DEFAULT_RTMP_PORT: u16 = 1935;

/// 選手顔の切り出しの大きさ（配信画面の選手画面に合わせる）
pub const FACE_W: u32 = 1066;
pub const FACE_H: u32 = 600;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceKind {
    /// 内蔵の RTMP の受け口に届いた映像（rtmp://このPC:1935/<app>/<key>）
    Rtmp { app: String, key: String },
    /// Webカメラ・キャプチャボード（Windows は DirectShow のデバイス名）
    Webcam { device: String },
    /// 静止画（確認・枠の位置合わせ用）
    Image { path: String },
    /// 映像ファイル（繰り返し再生）
    Video { path: String },
    /// URL（rtmp://・http:// など ffmpeg が読めるもの）
    Url { url: String },
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Rtmp { .. } => "RTMP",
            SourceKind::Webcam { .. } => "Webカメラ",
            SourceKind::Image { .. } => "画像",
            SourceKind::Video { .. } => "映像",
            SourceKind::Url { .. } => "URL",
        }
    }

    /// ffmpeg の入力の引数（RTMP は標準入力から FLV）
    pub fn input_args(&self) -> Vec<String> {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        match self {
            SourceKind::Rtmp { .. } => v(&[
                "-probesize",
                "65536",
                "-analyzeduration",
                "500000",
                "-f",
                "flv",
                "-i",
                "pipe:0",
            ]),
            SourceKind::Webcam { device } => {
                if cfg!(windows) {
                    let mut a = v(&["-f", "dshow", "-rtbufsize", "100M", "-i"]);
                    a.push(format!("video={device}"));
                    a
                } else if cfg!(target_os = "macos") {
                    let mut a = v(&["-f", "avfoundation", "-i"]);
                    a.push(device.clone());
                    a
                } else {
                    let mut a = v(&["-f", "v4l2", "-i"]);
                    a.push(device.clone());
                    a
                }
            }
            SourceKind::Image { path } => {
                let mut a = v(&["-re", "-loop", "1", "-framerate", "5", "-i"]);
                a.push(path.clone());
                a
            }
            SourceKind::Video { path } => {
                let mut a = v(&["-stream_loop", "-1", "-re", "-i"]);
                a.push(path.clone());
                a
            }
            SourceKind::Url { url } => {
                let mut a = v(&["-i"]);
                a.push(url.clone());
                a
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDef {
    /// 英数字と - だけ（URL に使う）
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
}

/// 枠の使い道
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "seat")]
pub enum RegionRole {
    /// 選手顔（席 0〜3）→ 配信画面の選手画面
    Face(u8),
    /// 手牌確認（席 0〜3）→ 牌の自動読み取り・入力担当の確認
    Hand(u8),
    /// 河・鳴き確認（席 0〜3）
    River(u8),
    /// 卓全体（河・鳴き・ドラ表示牌をまとめて）
    Board,
}

impl RegionRole {
    pub const ALL: [RegionRole; 13] = [
        RegionRole::Face(0),
        RegionRole::Face(1),
        RegionRole::Face(2),
        RegionRole::Face(3),
        RegionRole::Hand(0),
        RegionRole::Hand(1),
        RegionRole::Hand(2),
        RegionRole::Hand(3),
        RegionRole::River(0),
        RegionRole::River(1),
        RegionRole::River(2),
        RegionRole::River(3),
        RegionRole::Board,
    ];

    /// URL に使う名前（face-A など）
    pub fn id(self) -> String {
        let seat = |s: u8| (b'A' + s.min(3)) as char;
        match self {
            RegionRole::Face(s) => format!("face-{}", seat(s)),
            RegionRole::Hand(s) => format!("hand-{}", seat(s)),
            RegionRole::River(s) => format!("river-{}", seat(s)),
            RegionRole::Board => "board".into(),
        }
    }

    pub fn label(self) -> String {
        let seat = |s: u8| (b'A' + s.min(3)) as char;
        match self {
            RegionRole::Face(s) => format!("選手顔 席{}", seat(s)),
            RegionRole::Hand(s) => format!("手牌確認 席{}", seat(s)),
            RegionRole::River(s) => format!("河・鳴き確認 席{}", seat(s)),
            RegionRole::Board => "卓全体（河・鳴き・ドラ）".into(),
        }
    }

    /// 切り出した映像の大きさと fps（選手顔は配信画面の大きさ、確認用は読み取りに足りる大きさ）
    pub fn output(self) -> (u32, u32, u32) {
        match self {
            RegionRole::Face(_) => (FACE_W, FACE_H, 30),
            _ => (1280, 720, 10),
        }
    }
}

/// 枠（映像に対する割合で、左上 x,y と幅・高さ）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub source: String,
    pub role: RegionRole,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Region {
    /// 映像からはみ出さないように直す
    pub fn clamped(&self) -> Region {
        let w = self.w.clamp(0.02, 1.0);
        let h = self.h.clamp(0.02, 1.0);
        Region {
            x: self.x.clamp(0.0, 1.0 - w),
            y: self.y.clamp(0.0, 1.0 - h),
            w,
            h,
            ..self.clone()
        }
    }

    /// ffmpeg のフィルター（切り出し → 大きさ → fps）
    pub fn filter(&self) -> String {
        let r = self.clamped();
        let (w, h, fps) = r.role.output();
        let pad = if matches!(r.role, RegionRole::Face(_)) {
            format!(",pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black")
        } else {
            String::new()
        };
        format!(
            "crop=w=iw*{:.4}:h=ih*{:.4}:x=iw*{:.4}:y=ih*{:.4},scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2{pad},fps={fps}",
            r.w, r.h, r.x, r.y
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// 手で足したソース（RTMP は届いたものが自動で並ぶので、ここになくてもよい）
    pub sources: Vec<SourceDef>,
    /// 枠（使い道ごとに1つ）
    pub regions: Vec<Region>,
}

impl SandboxConfig {
    /// 同じ使い道の枠は1つにする（あとのほうを残す）
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<Region> = vec![];
        for r in self.regions.iter().rev() {
            if seen.insert(r.role) {
                out.push(r.clamped());
            }
        }
        out.reverse();
        self.regions = out;
    }
}

/// メディアサーバーの様子（`GET /status`）
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaStatus {
    pub version: String,
    pub rtmp_port: u16,
    pub rtmp_error: Option<String>,
    pub sources: Vec<SourceStatus>,
    pub config: SandboxConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceStatus {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// 映像が届いている
    pub live: bool,
    pub fps: f32,
    /// 元の映像の大きさ（分かれば）
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
    /// 手で足したソースか（RTMP の自動は false）
    pub configured: bool,
}

/// ソースの id に使える文字だけにする
pub fn safe_id(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if t.is_empty() { "src".into() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_filter_and_ids() {
        let r = Region {
            source: "rtmp-a".into(),
            role: RegionRole::Face(1),
            x: 0.9,
            y: 0.0,
            w: 0.5,
            h: 0.5,
        };
        let f = r.filter();
        assert!(
            f.starts_with("crop=w=iw*0.5000:h=ih*0.5000:x=iw*0.5000"),
            "{f}"
        );
        assert!(f.contains("pad=1066:600"));
        assert_eq!(RegionRole::Face(1).id(), "face-B");
        assert_eq!(safe_id("live/カメラ1"), "live----1");
        let mut c = SandboxConfig {
            sources: vec![],
            regions: vec![r.clone(), Region { x: 0.1, ..r }],
        };
        c.normalize();
        assert_eq!(c.regions.len(), 1);
        assert!((c.regions[0].x - 0.1).abs() < 1e-6);
    }
}

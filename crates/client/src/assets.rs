//! 素材の読み込み: 牌画像（SVG をラスタライズ）とフォント（設計書 7章）。

use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use janscope_core::tile::{Suit, Tile};

/// 素材フォルダ。環境変数 JANSCOPE_ASSETS → 実行ファイルの隣の assets → 開発時のリポジトリの assets の順に探す。
pub fn assets_dir() -> PathBuf {
    if let Ok(p) = std::env::var("JANSCOPE_ASSETS") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("assets");
            if p.join("tiles").is_dir() {
                return p;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// 牌画像の幅・高さ（ラスタライズの解像度。表示はこれを縮小する）。
pub const TILE_PX_W: u32 = 150;
pub const TILE_PX_H: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TileTheme {
    #[default]
    Regular,
    /// 黒版（テーマ設定で切り替える。設定UIは段階B8）
    #[allow(dead_code)]
    Black,
}

impl TileTheme {
    fn dir(self) -> &'static str {
        match self {
            TileTheme::Regular => "regular",
            TileTheme::Black => "black",
        }
    }
}

/// 牌の画像（34種＋赤5×3＋裏）。
#[derive(Resource, Clone)]
pub struct TileImages {
    kinds: Vec<Handle<Image>>,
    reds: [Handle<Image>; 3],
    pub back: Handle<Image>,
}

impl TileImages {
    pub fn get(&self, t: Tile) -> Handle<Image> {
        if t.is_unknown() {
            return self.back.clone();
        }
        if t.is_red() {
            let i = match t.suit() {
                Suit::Man => 0,
                Suit::Pin => 1,
                _ => 2,
            };
            return self.reds[i].clone();
        }
        self.kinds[t.kind() as usize].clone()
    }
}

/// フォント。
#[derive(Resource, Clone)]
pub struct Fonts {
    pub regular: Handle<Font>,
    pub bold: Handle<Font>,
}

fn fluffy_name(t: Tile) -> String {
    if t.is_honor() {
        return ["Ton", "Nan", "Shaa", "Pei", "Haku", "Hatsu", "Chun"][(t.number() - 1) as usize]
            .into();
    }
    let s = match t.suit() {
        Suit::Man => "Man",
        Suit::Pin => "Pin",
        _ => "Sou",
    };
    if t.is_red() {
        format!("{s}5-Dora")
    } else {
        format!("{s}{}", t.number())
    }
}

/// SVG を重ねてラスタライズする（RGBA、ストレートアルファ）。
pub fn rasterize(layers: &[&Path], w: u32, h: u32) -> Result<Vec<u8>, String> {
    use resvg::{tiny_skia, usvg};
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("pixmap")?;
    for p in layers {
        let data = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
            .map_err(|e| format!("{}: {e}", p.display()))?;
        let size = tree.size();
        let tf =
            tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
        resvg::render(&tree, tf, &mut pixmap.as_mut());
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(out)
}

fn to_image(rgba: Vec<u8>, w: u32, h: u32) -> Image {
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

pub fn load_tile_images(
    images: &mut Assets<Image>,
    theme: TileTheme,
) -> Result<TileImages, String> {
    let dir = assets_dir().join("tiles").join(theme.dir());
    let front = dir.join("Front.svg");
    let mut make = |t: Tile| -> Result<Handle<Image>, String> {
        let face = dir.join(format!("{}.svg", fluffy_name(t)));
        let rgba = rasterize(&[&front, &face], TILE_PX_W, TILE_PX_H)?;
        Ok(images.add(to_image(rgba, TILE_PX_W, TILE_PX_H)))
    };
    let mut kinds = Vec::new();
    for t in Tile::all_kinds() {
        kinds.push(make(t)?);
    }
    let reds = [
        make(Tile::red_five(Suit::Man))?,
        make(Tile::red_five(Suit::Pin))?,
        make(Tile::red_five(Suit::Sou))?,
    ];
    let back_rgba = rasterize(&[&dir.join("Back.svg")], TILE_PX_W, TILE_PX_H)?;
    let back = images.add(to_image(back_rgba, TILE_PX_W, TILE_PX_H));
    Ok(TileImages { kinds, reds, back })
}

pub fn load_fonts(server: &AssetServer) -> Fonts {
    Fonts {
        regular: server.load("fonts/NotoSansJP-Regular.otf"),
        bold: server.load("fonts/NotoSansJP-Bold.otf"),
    }
}

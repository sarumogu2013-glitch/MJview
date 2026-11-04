//! 仮デザインのテーマ（設計書 1章「デザインは仮。テーマとして分離」）。配色はここだけで決める。

use bevy::prelude::*;

#[derive(Resource, Clone)]
pub struct Theme {
    pub bg: Color,
    pub panel: Color,
    pub panel_alt: Color,
    pub band: Color,
    pub text: Color,
    pub text_dim: Color,
    pub dealer: Color,
    pub riichi: Color,
    pub plus: Color,
    pub minus: Color,
    pub warn: Color,
    pub highlight: Color,
    pub face_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            bg: Color::srgb(0.06, 0.08, 0.10),
            panel: Color::srgb(0.10, 0.13, 0.17),
            panel_alt: Color::srgb(0.13, 0.17, 0.22),
            band: Color::srgba(0.0, 0.0, 0.0, 0.62),
            text: Color::srgb(0.95, 0.96, 0.97),
            text_dim: Color::srgb(0.62, 0.67, 0.73),
            dealer: Color::srgb(0.88, 0.30, 0.25),
            riichi: Color::srgb(0.98, 0.78, 0.20),
            plus: Color::srgb(0.40, 0.85, 0.45),
            minus: Color::srgb(0.95, 0.40, 0.40),
            warn: Color::srgb(0.98, 0.45, 0.20),
            highlight: Color::srgb(0.30, 0.62, 0.95),
            face_bg: Color::srgb(0.02, 0.02, 0.03),
        }
    }
}

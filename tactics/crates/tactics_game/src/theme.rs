//! Colours and sizes in one place, so the look can be tuned without hunting
//! through systems.

use bevy::prelude::*;

pub const SKY: Color = Color::srgb(0.06, 0.07, 0.09);

pub const FLOOR_A: Color = Color::srgb(0.20, 0.22, 0.24);
pub const FLOOR_B: Color = Color::srgb(0.23, 0.25, 0.27);
pub const LOW_COVER: Color = Color::srgb(0.55, 0.42, 0.25);
pub const HIGH_COVER: Color = Color::srgb(0.38, 0.40, 0.45);

pub const PLAYER: Color = Color::srgb(0.25, 0.55, 0.95);
pub const ENEMY: Color = Color::srgb(0.85, 0.25, 0.25);

pub const MOVE_NEAR: Color = Color::srgba(0.3, 0.65, 1.0, 0.9);
pub const MOVE_FAR: Color = Color::srgba(1.0, 0.8, 0.2, 0.9);
pub const HOVER: Color = Color::srgb(1.0, 1.0, 1.0);
pub const SELECTED: Color = Color::srgb(0.4, 1.0, 0.6);
pub const TARGET: Color = Color::srgb(1.0, 0.35, 0.3);
pub const TRACER: Color = Color::srgb(1.0, 0.9, 0.4);
pub const HEAL: Color = Color::srgb(0.4, 1.0, 0.5);
pub const EXPLOSION: Color = Color::srgb(1.0, 0.55, 0.15);
pub const COVER_HALF: Color = Color::srgb(0.5, 0.75, 1.0);
pub const COVER_FULL: Color = Color::srgb(0.2, 0.5, 1.0);

pub const UI_PANEL: Color = Color::srgba(0.05, 0.06, 0.08, 0.85);
pub const UI_BUTTON: Color = Color::srgb(0.16, 0.19, 0.24);
pub const UI_BUTTON_HOVER: Color = Color::srgb(0.24, 0.29, 0.36);
pub const UI_BUTTON_DISABLED: Color = Color::srgb(0.11, 0.11, 0.12);
pub const UI_BUTTON_PRESSED: Color = Color::srgb(0.30, 0.55, 0.40);
pub const UI_TEXT: Color = Color::srgb(0.92, 0.93, 0.95);
pub const UI_DIM: Color = Color::srgb(0.6, 0.63, 0.68);

/// World-space height of a unit's body.
pub const UNIT_HEIGHT: f32 = 1.2;
/// Heights of cover blocks.
pub const LOW_COVER_HEIGHT: f32 = 0.6;
pub const HIGH_COVER_HEIGHT: f32 = 1.8;

//! # tactics_game
//!
//! A Bevy 3D frontend for [`tactics_core`]. The core crate decides *what*
//! happens; this crate decides how it *looks and feels*.
//!
//! ## Data flow
//!
//! ```text
//!  input.rs / hud.rs ──Action──► state.rs::BattleState::apply ──GameEvents──► playback.rs
//!        ▲                              │ (rules, dice: tactics_core)            │ animates
//!        │                              ▼                                        ▼
//!   overlay.rs (gizmos) ◄── queries ── Battle ──────── units.rs (meshes) ◄── transforms
//!                                       ▲
//!  enemy_turn.rs ──Action (from AI)─────┘
//! ```
//!
//! * Nothing outside `state.rs` mutates the [`Battle`](tactics_core::battle::Battle).
//! * While [`Playback`](playback::Playback) has queued events or a camera
//!   script runs, input and AI are paused, so the player always sees the
//!   consequences of each action.
//! * How things *look* (models, animation clips, camera scripts, projectiles)
//!   is module data too: see [`models`], [`animation`], [`camera_director`].
//!
//! ## Plugins
//! Each module exposes one plugin; [`TacticsPlugin`] bundles them. Swap or extend
//! any of them independently (e.g. replace `board.rs` with glTF level art).

pub mod animation;
pub mod board;
pub mod camera;
pub mod camera_director;
pub mod characters;
pub mod content;
pub mod coords;
pub mod cutaway;
pub mod editor;
pub mod enemy_turn;
pub mod html_ui;
pub mod hud;
pub mod input;
pub mod lab;
pub mod loadout;
pub mod missions;
pub mod models;
pub mod overlay;
pub mod playback;
pub mod rig;
pub mod state;
pub mod theme;
pub mod tooltip;
pub mod units;

use bevy::prelude::*;
use bevy::window::WindowResolution;

/// System sets that order the frame. Configured once here so each plugin can
/// slot its systems into the right phase without knowing about the others.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameSet {
    /// Read devices, update camera and hovered tile, issue player actions.
    Input,
    /// Pick AI actions, apply actions to the battle.
    Logic,
    /// Advance animations of queued events.
    Playback,
    /// Draw overlays and refresh UI text from the current state.
    Present,
}

/// Bundles every plugin of the game.
pub struct TacticsPlugin;

impl Plugin for TacticsPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (FrameSet::Input, FrameSet::Logic, FrameSet::Playback, FrameSet::Present).chain(),
        )
        .add_plugins((
            state::StatePlugin,
            board::BoardPlugin,
            models::ModelsPlugin,
            units::UnitsPlugin,
            animation::AnimationPlugin,
            camera::CameraPlugin,
            camera_director::CameraDirectorPlugin,
            input::InputPlugin,
            enemy_turn::EnemyTurnPlugin,
            playback::PlaybackPlugin,
            overlay::OverlayPlugin,
            hud::HudPlugin,
            missions::MissionsPlugin,
            html_ui::HtmlUiPlugin,
            editor::EditorPlugin,
        ))
        .add_plugins((characters::CharactersPlugin, lab::LabPlugin, tooltip::TooltipPlugin, cutaway::CutawayPlugin));
    }
}

/// Builds and runs the app. Shared by every platform's entry point.
pub fn run() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Tactics".into(),
                        resolution: WindowResolution::new(1280, 720),
                        // Web: let the <canvas> follow its parent element's size.
                        fit_canvas_to_parent: true,
                        // Web: keep browser shortcuts (F5, Ctrl+R...) working.
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                // Web/Android: don't try to read `.meta` files that don't exist.
                .set(AssetPlugin {
                    meta_check: bevy::asset::AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .insert_resource(ClearColor(theme::SKY))
        .add_plugins(TacticsPlugin)
        .run();
}

/// Android entry point. `bevy_main` generates the `android_main` symbol that
/// NativeActivity / GameActivity calls into.
#[cfg(target_os = "android")]
#[bevy_main]
fn android_main_entry() {
    run();
}

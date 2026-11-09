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
pub mod fire;
pub mod fog;
pub mod html_ui;
pub mod hud;
pub mod input;
pub mod lab;
pub mod loading;
pub mod loadout;
pub mod looks;
pub mod minimap;
pub mod missions;
pub mod models;
pub mod overlay;
pub mod playback;
pub mod rig;
pub mod roster;
pub mod campaign;
pub mod graphics;
pub mod scenery;
pub mod settings;
pub mod sky;
pub mod state;
pub mod theme;
pub mod tooltip;
pub mod touch;
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
        .add_plugins((characters::CharactersPlugin, lab::LabPlugin, tooltip::TooltipPlugin, cutaway::CutawayPlugin, minimap::MinimapPlugin, settings::SettingsPlugin, looks::LooksPlugin, roster::RosterPlugin, campaign::CampaignPlugin, graphics::GraphicsPlugin))
            .add_plugins((fog::FogPlugin, touch::TouchPlugin, sky::SkyPlugin, loading::LoadingScreenPlugin, fire::FirePlugin));
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

/// Reads a save file (roster, settings): a file on desktop / Android, the
/// browser's localStorage on the web. `None` = nothing saved yet.
pub fn read_save(name: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?.local_storage().ok()??.get_item(&format!("tbsium/{name}")).ok()?
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        save_path(name).and_then(|p| std::fs::read_to_string(p).ok())
    }
}

/// Writes a save file (see [`read_save`]).
pub fn write_save(name: &str, text: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or("no localStorage")?;
        storage.set_item(&format!("tbsium/{name}"), text).map_err(|e| format!("{e:?}"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = save_path(name).ok_or("no save folder")?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

/// Where a save file called `name` lives: the working directory on desktop,
/// the app's private data folder on Android, nowhere on the web.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_path(name: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    {
        bevy::android::ANDROID_APP.get()?.internal_data_path().map(|dir| dir.join(name))
    }
    #[cfg(not(target_os = "android"))]
    {
        Some(std::path::PathBuf::from(name))
    }
}

/// Android entry point. `bevy_main` generates the `android_main` symbol that
/// NativeActivity / GameActivity calls into.
/// (The macro requires the function to be called `main`.)
#[cfg(target_os = "android")]
#[bevy_main]
fn main() {
    run();
}

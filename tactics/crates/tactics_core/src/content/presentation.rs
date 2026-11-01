//! Presentation content: models, animations, camera scripts and per-ability
//! visuals.
//!
//! This is **pure data**: the rules engine never reads it, so battles stay
//! deterministic and headless servers/tests never touch assets. The frontend
//! (`tactics_game`) looks these definitions up by id. Like all content they come
//! from modules (Rust or RON), and later modules override earlier ones.
//!
//! | Definition | Keyed by | Used for |
//! |---|---|---|
//! | [`ModelDef`] | model id (`UnitClass::model`) | which 3D model a unit uses and its animation clips |
//! | [`AbilityPresentation`] | ability id | animation to play, camera script, projectile style, timing |
//! | [`CameraScriptDef`] | script id | cinematic camera moves (over-the-shoulder, kill cam...) |
//! | [`PresentationSettings`] | — | global hooks such as the kill camera |
//!
//! ## Animation slots
//! Animations are referred to by **slot name**. The frontend plays these
//! automatically; abilities may name any other slot (e.g. `"cast"`):
//!
//! | slot | when |
//! |---|---|
//! | `idle` | default loop |
//! | `walk` | while moving (loop) |
//! | `hunker` | while defending (loop) |
//! | `shoot`, `throw`, `overwatch`, `reload`, `heal` | default for the matching ability effect |
//! | `hit` | when damaged |
//! | `death` | when killed |
//!
//! If a model has no clip for a slot, the frontend falls back to a built-in
//! procedural animation (bob, recoil, flinch, crouch...), so primitive models
//! and partially animated models still look alive.

use std::collections::BTreeMap;

use super::AbilityId;

/// Built-in primitive shapes used when a model has no glTF scene (or while it loads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum PrimitiveShape {
    /// Capsule body, small head, rifle.
    #[default]
    Soldier,
    /// Capsule body, big head.
    Alien,
    /// Hovering box with an eye.
    Drone,
}

/// One animation clip of a model.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AnimationClipDef {
    /// Asset path with label, e.g. `"models/drone.gltf#Animation0"`.
    pub clip: String,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Loop (idle/walk) or play once (shoot/hit/death).
    pub looping: bool,
}

impl Default for AnimationClipDef {
    fn default() -> Self {
        Self { clip: String::new(), speed: 1.0, looping: false }
    }
}

/// A unit model: an optional glTF scene plus animation clips per slot.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ModelDef {
    /// Id referenced by [`UnitClass::model`](crate::unit::UnitClass::model).
    pub id: String,
    /// glTF scene path relative to the `assets/` folder, e.g.
    /// `"models/drone.gltf#Scene0"`. `None` = use [`ModelDef::fallback`].
    pub scene: Option<String>,
    /// Primitive used when there is no scene.
    pub fallback: PrimitiveShape,
    /// Uniform scale applied to the scene.
    pub scale: f32,
    /// Offset of the scene from the tile centre (x, y up, z).
    pub offset: [f32; 3],
    /// Rotation correction so the model faces -Z ("forward"), in degrees.
    pub yaw_degrees: f32,
    /// Animation clips by slot name (`"idle"`, `"walk"`, `"shoot"`...).
    pub animations: BTreeMap<String, AnimationClipDef>,
}

impl Default for ModelDef {
    fn default() -> Self {
        Self {
            id: String::new(),
            scene: None,
            fallback: PrimitiveShape::Soldier,
            scale: 1.0,
            offset: [0.0; 3],
            yaw_degrees: 0.0,
            animations: BTreeMap::new(),
        }
    }
}

impl ModelDef {
    /// A model drawn with a primitive shape only.
    pub fn primitive(id: impl Into<String>, shape: PrimitiveShape) -> Self {
        Self { id: id.into(), fallback: shape, ..Default::default() }
    }
}

/// How an attack's projectile is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum ProjectileStyle {
    /// Fast straight streak (bullets).
    #[default]
    Tracer,
    /// Thick, lingering straight line (lasers, psionics).
    Beam,
    /// Lobbed arc (grenades). Drawn from the user to the target point.
    Arc,
    /// Nothing drawn.
    None,
}

/// How an ability looks when used.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AbilityPresentation {
    /// Ability this decorates.
    pub ability: AbilityId,
    /// Animation slot to play on the user. `None` = default for the effect
    /// kind (`shoot`, `throw`, `overwatch`, `hunker`, `reload`, `heal`).
    pub animation: Option<String>,
    /// Camera script to run when the ability is used.
    pub camera: Option<String>,
    /// Chance (0-100) that the camera script runs, so cinematics stay fresh.
    pub camera_chance: u32,
    /// Projectile style for attacks.
    pub projectile: ProjectileStyle,
    /// Seconds between the start of the animation and the effect (e.g. the
    /// moment the grenade leaves the hand).
    pub windup: f32,
}

impl Default for AbilityPresentation {
    fn default() -> Self {
        Self {
            ability: AbilityId::default(),
            animation: None,
            camera: None,
            camera_chance: 100,
            projectile: ProjectileStyle::Tracer,
            windup: 0.35,
        }
    }
}

/// A point a camera step can refer to. Resolved by the frontend from the
/// action that triggered the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraAnchor {
    /// The unit performing the action.
    #[default]
    Actor,
    /// The target unit or tile (for kill cams: the victim).
    Target,
    /// Halfway between actor and target.
    Midpoint,
    /// Wherever the camera is currently looking.
    Current,
}

/// One instruction of a camera script. Durations are in real seconds (not
/// affected by slow motion).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraStep {
    /// Move the look-at point to an anchor.
    Focus {
        on: CameraAnchor,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Rotate so the camera looks from `from` towards `to` (over-the-shoulder),
    /// plus `offset_degrees` to the side.
    FaceAlong {
        from: CameraAnchor,
        to: CameraAnchor,
        #[cfg_attr(feature = "data", serde(default))]
        offset_degrees: f32,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Rotate around the focus point by this many degrees (relative).
    Orbit {
        degrees: f32,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Move to this distance from the focus point.
    Zoom {
        distance: f32,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Set the angle above the horizon, in degrees (90 = top-down).
    Pitch {
        degrees: f32,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Shake the camera.
    Shake {
        strength: f32,
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
    /// Change game speed (slow motion). Reset by `Restore` or when the script ends.
    TimeScale { scale: f32 },
    /// Hold.
    Wait { duration: f32 },
    /// Run several steps at the same time; finishes when the longest does.
    Together(Vec<CameraStep>),
    /// Return smoothly to the camera the player had before the script.
    Restore {
        #[cfg_attr(feature = "data", serde(default))]
        duration: f32,
    },
}

/// A named camera script.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct CameraScriptDef {
    pub id: String,
    pub steps: Vec<CameraStep>,
}

/// Global presentation hooks.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct PresentationSettings {
    /// Camera script played when a unit dies (actor = killer, target = victim).
    pub kill_camera: Option<String>,
    /// Chance (0-100) for the kill camera.
    pub kill_camera_chance: u32,
}

impl Default for PresentationSettings {
    fn default() -> Self {
        Self { kill_camera: None, kill_camera_chance: 100 }
    }
}

/// An HTML/CSS user-interface layout (e.g. the in-battle HUD).
///
/// The frontend renders a subset of HTML + CSS natively (no browser), with
/// `{{ data }}` bindings and `data-command` buttons; see `UI.md`. The layout
/// with id `"hud"` replaces the default battle HUD.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct UiLayoutDef {
    pub id: String,
    /// HTML markup, optionally with `<style>` blocks.
    pub html: String,
}

//! Unit animation: glTF clips when the model has them, procedural motion
//! otherwise.
//!
//! Every unit root has a [`UnitAnimator`]. Each frame it decides which **slot**
//! the unit should show, in priority order:
//!
//! 1. `death` once killed,
//! 2. a **one-shot** requested by playback (`shoot`, `throw`, `hit`, an
//!    ability's custom slot...), until it finishes,
//! 3. `walk` while moving,
//! 4. a **stance** loop from the battle state (`hunker` while defending),
//! 5. `idle`.
//!
//! If the unit's model has a glTF clip for that slot it is cross-faded in via
//! `AnimationTransitions`. Otherwise [`procedural_pose`] animates the unit's
//! pivot entity (bob, recoil, flinch, crouch...), so primitive models and
//! partially animated glTF models both stay expressive.
//!
//! Other systems request animations with the [`AnimCommand`] message.
//!
//! ## Adding a procedural animation
//! Add a match arm to [`procedural_pose`] for the new slot name; any ability
//! can then use it via `AbilityPresentation::animation`.

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::models::ModelLibrary;
use crate::state::BattleState;
use crate::units::UnitEntities;

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<AnimCommand>()
            .add_systems(Update, (apply_commands, drive_animators).chain().in_set(FrameSet::Present));
    }
}

/// Requests from playback (or any gameplay code).
#[derive(Message, Debug, Clone, PartialEq)]
pub enum AnimCommand {
    /// Play a slot once, then return to the base loop.
    OneShot { unit: UnitId, slot: String },
    /// Start/stop the walk loop.
    Walking { unit: UnitId, on: bool },
    /// Play `death` and hold it.
    Died { unit: UnitId },
}

/// Default length of one-shots that have no glTF clip (seconds).
pub const PROCEDURAL_ONE_SHOT: f32 = 0.55;

/// Per-unit animation state.
#[derive(Component, Debug)]
pub struct UnitAnimator {
    /// Model id (for clip lookups).
    pub model: String,
    walking: bool,
    dead: bool,
    /// (slot, elapsed, length)
    one_shot: Option<(String, f32, f32)>,
    /// Time in the current slot.
    slot_time: f32,
    /// Slot shown last frame.
    shown: String,
    /// Entity with the glTF `AnimationPlayer`, once the scene has loaded.
    player: Option<Entity>,
    /// Slot currently playing on the glTF player.
    playing_clip: Option<String>,
}

impl UnitAnimator {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            walking: false,
            dead: false,
            one_shot: None,
            slot_time: 0.0,
            shown: "idle".into(),
            player: None,
            playing_clip: None,
        }
    }

    pub fn attach_player(&mut self, player: Entity) {
        self.player = Some(player);
        self.playing_clip = Some("idle".into());
    }

    /// The slot being shown right now.
    pub fn current_slot(&self) -> &str {
        &self.shown
    }
}

/// The child entity procedural animation moves (models hang below it).
#[derive(Component)]
pub struct UnitPivot;

fn apply_commands(
    mut commands: MessageReader<AnimCommand>,
    entities: Res<UnitEntities>,
    library: Res<ModelLibrary>,
    clips: Res<Assets<AnimationClip>>,
    mut animators: Query<&mut UnitAnimator>,
) {
    for command in commands.read() {
        let unit = match command {
            AnimCommand::OneShot { unit, .. } | AnimCommand::Walking { unit, .. } | AnimCommand::Died { unit } => *unit,
        };
        let Some(mut a) = entities.0.get(&unit).and_then(|e| animators.get_mut(*e).ok()) else { continue };
        match command {
            AnimCommand::OneShot { slot, .. } => {
                // Length: the glTF clip's duration if known, else a default.
                let length = library
                    .clip(&a.model, slot)
                    .and_then(|c| clips.get(&c.clip).map(|clip| clip.duration() / c.speed.max(0.01)))
                    .unwrap_or(PROCEDURAL_ONE_SHOT);
                a.one_shot = Some((slot.clone(), 0.0, length));
            }
            AnimCommand::Walking { on, .. } => a.walking = *on,
            AnimCommand::Died { .. } => a.dead = true,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_animators(
    time: Res<Time>,
    state: Res<BattleState>,
    library: Res<ModelLibrary>,
    mut animators: Query<(&mut UnitAnimator, &crate::units::UnitVisual, &Children)>,
    mut pivots: Query<&mut Transform, With<UnitPivot>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let dt = time.delta_secs();
    for (mut a, visual, children) in &mut animators {
        // Advance a running one-shot.
        if let Some((_, elapsed, length)) = a.one_shot.as_mut() {
            *elapsed += dt;
            if *elapsed >= *length {
                a.one_shot = None;
            }
        }

        // Decide the slot.
        let stance = state
            .battle
            .unit(visual.id)
            .filter(|u| u.status.defense_bonus > 0)
            .map(|_| "hunker");
        let slot: String = if a.dead {
            "death".into()
        } else if let Some((slot, _, _)) = &a.one_shot {
            slot.clone()
        } else if a.walking {
            "walk".into()
        } else {
            stance.unwrap_or("idle").into()
        };
        if slot != a.shown {
            a.shown = slot.clone();
            a.slot_time = 0.0;
        } else {
            a.slot_time += dt;
        }

        // glTF clip if the model has one for this slot; else keep its base
        // loop running and animate procedurally on top.
        let clip = library.clip(&a.model, &slot);
        if let Some(player_entity) = a.player {
            let wanted = if clip.is_some() { slot.clone() } else { "idle".to_string() };
            if a.playing_clip.as_deref() != Some(wanted.as_str())
                && let (Some(c), Ok((mut player, mut transitions))) =
                    (library.clip(&a.model, &wanted), players.get_mut(player_entity))
                {
                    let anim = transitions.play(&mut player, c.node, Duration::from_millis(180));
                    anim.set_speed(c.speed);
                    if c.looping {
                        anim.repeat();
                    }
                    a.playing_clip = Some(wanted);
                }
        }

        let pose = if clip.is_some() {
            Transform::IDENTITY
        } else {
            let progress = a.one_shot.as_ref().map_or(0.0, |(_, e, l)| (e / l).clamp(0.0, 1.0));
            procedural_pose(&slot, a.slot_time, progress)
        };
        for child in children.iter() {
            if let Ok(mut t) = pivots.get_mut(child) {
                *t = pose;
            }
        }
    }
}

/// A smooth 0→1→0 bump over a one-shot's progress `k`.
fn bump(k: f32) -> f32 {
    (PI * k).sin()
}

/// Procedural pose of the pivot for `slot`. `t` = seconds in the slot,
/// `k` = one-shot progress (0..1). The pivot sits on the ground at the tile
/// centre, the unit faces -Z. **Add new procedural animations here.**
pub fn procedural_pose(slot: &str, t: f32, k: f32) -> Transform {
    let mut tr = Transform::IDENTITY;
    match slot {
        // Breathing.
        "idle" => tr.scale.y = 1.0 + 0.015 * (2.2 * t).sin(),
        // Bobbing, leaning into the walk.
        "walk" => {
            tr.translation.y = 0.06 * (9.0 * t).sin().abs();
            tr.rotation = Quat::from_rotation_x(-0.08);
        }
        // Crouched behind cover.
        "hunker" => tr.scale = Vec3::new(1.08, 0.72, 1.08),
        // Brace, aim down.
        "overwatch" => {
            tr.rotation = Quat::from_rotation_x(-0.15 * bump(k.min(0.5) * 2.0).max(0.6));
            tr.scale.y = 0.92;
        }
        // Recoil: kick back and tilt up.
        "shoot" => {
            let r = bump(k) * (1.0 - k);
            tr.translation.z = 0.18 * r;
            tr.rotation = Quat::from_rotation_x(0.18 * r);
        }
        // Wind up backwards, then lunge forwards.
        "throw" => {
            let lean = if k < 0.4 { 0.35 * k / 0.4 } else { 0.35 - 0.7 * ((k - 0.4) / 0.6).min(1.0) };
            tr.rotation = Quat::from_rotation_x(lean * (1.0 - k).max(0.2));
        }
        // Flinch shake that dies out.
        "hit" => {
            let decay = 1.0 - k;
            tr.translation.x = 0.07 * (45.0 * t).sin() * decay;
            tr.rotation = Quat::from_rotation_z(0.12 * (38.0 * t).sin() * decay);
        }
        // Tilt the weapon to the side.
        "reload" => {
            tr.rotation = Quat::from_rotation_z(0.25 * bump(k));
            tr.translation.y = -0.05 * bump(k);
        }
        // Kneel down briefly.
        "heal" => {
            tr.scale.y = 1.0 - 0.2 * bump(k);
            tr.rotation = Quat::from_rotation_x(-0.2 * bump(k));
        }
        // Rise and spin (psionics, buffs).
        "cast" => {
            tr.translation.y = 0.18 * bump(k);
            tr.rotation = Quat::from_rotation_y(TAU * k);
        }
        // Death and unknown slots: no procedural motion (death topple is
        // handled by playback for models without a death clip).
        _ => {}
    }
    tr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_poses_return_to_rest() {
        for slot in ["shoot", "throw", "hit", "reload", "heal", "cast"] {
            let end = procedural_pose(slot, PROCEDURAL_ONE_SHOT, 1.0);
            assert!(end.translation.length() < 0.05, "{slot} ends displaced: {:?}", end.translation);
        }
    }

    #[test]
    fn unknown_slot_is_identity() {
        assert_eq!(procedural_pose("does_not_exist", 0.3, 0.5), Transform::IDENTITY);
    }
}

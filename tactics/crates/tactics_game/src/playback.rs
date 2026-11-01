//! Event playback: turns the instant results of the rules engine into
//! animations that play one after another.
//!
//! [`Playback`] holds a FIFO of [`GameEvent`]s. Each frame, if no animation is
//! running, the next event is popped and converted into an [`Anim`]. Events with
//! no visual (e.g. AP changes) complete immediately. While anything is queued
//! or running, [`Playback::is_busy`] is `true`, which pauses player input and
//! the AI.
//!
//! To add a new kind of feedback (sound, camera shake, particles), match the
//! event in [`start_event`] and add an `Anim` variant or spawn a one-shot
//! entity — the rules engine doesn't need to change.

use std::collections::VecDeque;

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::grid_to_world;
use crate::hud::{FloatingText, LogMessages};
use crate::state::BattleState;
use crate::theme;
use crate::units::UnitEntities;

pub struct PlaybackPlugin;

impl Plugin for PlaybackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Playback>()
            .add_systems(Update, run_playback.in_set(FrameSet::Playback));
    }
}

/// Seconds a unit takes to cross one tile.
const STEP_TIME: f32 = 0.16;
/// Duration of a shot tracer.
const SHOT_TIME: f32 = 0.45;
/// Pause before a shot so the player sees who is aiming at whom.
const AIM_TIME: f32 = 0.35;
/// Death collapse duration.
const DEATH_TIME: f32 = 0.6;

/// A running animation.
#[derive(Debug, Clone)]
pub enum Anim {
    /// Walk through `points` (world positions), one segment per tile.
    Walk { entity: Entity, from: Vec3, points: Vec<Vec3>, index: usize, t: f32 },
    /// Turn to face the target, then fire a tracer.
    Shot { shooter: Vec3, target: Vec3, outcome: ShotOutcome, reaction: bool, t: f32 },
    /// Sink into the floor and despawn.
    Death { entity: Entity, t: f32 },
    /// Do nothing for a while (lets banners/floating text breathe).
    Wait { remaining: f32 },
}

/// Queue of events awaiting presentation plus the animation in progress.
#[derive(Resource, Default)]
pub struct Playback {
    queue: VecDeque<GameEvent>,
    current: Option<Anim>,
}

impl Playback {
    pub fn enqueue(&mut self, events: impl IntoIterator<Item = GameEvent>) {
        self.queue.extend(events);
    }

    /// `true` while events are queued or an animation runs.
    pub fn is_busy(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }

    /// The animation currently running (used by the overlay to draw tracers).
    pub fn current(&self) -> Option<&Anim> {
        self.current.as_ref()
    }

    /// Drops everything (used when restarting a battle).
    pub fn clear(&mut self) {
        self.queue.clear();
        self.current = None;
    }
}

/// Height at which shots are fired / hit.
const CHEST: f32 = theme::UNIT_HEIGHT * 0.65;

#[allow(clippy::too_many_arguments)]
fn run_playback(
    time: Res<Time>,
    mut commands: Commands,
    mut playback: ResMut<Playback>,
    state: Res<BattleState>,
    entities: Res<UnitEntities>,
    mut transforms: Query<&mut Transform>,
    mut rig: Single<&mut CameraRig>,
    mut log: ResMut<LogMessages>,
) {
    let dt = time.delta_secs();
    // Process instant events and start the next animation. Bounded loop so a
    // long run of instant events can't stall a frame forever.
    for _ in 0..64 {
        if playback.current.is_some() {
            break;
        }
        let Some(event) = playback.queue.pop_front() else { break };
        playback.current = start_event(&event, &state.battle, &entities, &mut commands, &mut rig, &mut log, &transforms);
    }

    let Some(anim) = playback.current.as_mut() else { return };
    let finished = match anim {
        Anim::Walk { entity, from, points, index, t } => {
            *t += dt / STEP_TIME;
            while *t >= 1.0 && *index < points.len() {
                *t -= 1.0;
                *from = points[*index];
                *index += 1;
            }
            if let Ok(mut tr) = transforms.get_mut(*entity) {
                if let Some(next) = points.get(*index) {
                    tr.translation = from.lerp(*next, *t);
                    face(&mut tr, *next);
                    rig.target_focus = tr.translation;
                } else {
                    tr.translation = *from;
                }
            }
            *index >= points.len()
        }
        Anim::Shot { shooter, target, outcome, t, .. } => {
            let before = *t;
            *t += dt;
            // Spawn the result text the moment the tracer lands.
            if before < AIM_TIME + SHOT_TIME && *t >= AIM_TIME + SHOT_TIME {
                let (text, color) = match outcome {
                    ShotOutcome::Miss => ("MISSED".to_string(), theme::UI_DIM),
                    ShotOutcome::Hit { damage, critical: true } => (format!("CRIT {damage}"), theme::TRACER),
                    ShotOutcome::Hit { damage, .. } => (format!("-{damage}"), theme::TARGET),
                };
                FloatingText::spawn(&mut commands, *target + Vec3::Y * 0.6, text, color);
            }
            let _ = shooter;
            *t >= AIM_TIME + SHOT_TIME + 0.15
        }
        Anim::Death { entity, t } => {
            *t += dt / DEATH_TIME;
            if let Ok(mut tr) = transforms.get_mut(*entity) {
                // Topple sideways while sinking.
                tr.rotation = Quat::from_rotation_z(t.min(1.0) * std::f32::consts::FRAC_PI_2);
                tr.translation.y = -t.min(1.0) * 0.4;
            }
            if *t >= 1.0 {
                commands.entity(*entity).despawn();
                true
            } else {
                false
            }
        }
        Anim::Wait { remaining } => {
            *remaining -= dt;
            *remaining <= 0.0
        }
    };
    if finished {
        playback.current = None;
    }
}

/// Rotates a unit (around Y only) to look at a world point.
fn face(tr: &mut Transform, point: Vec3) {
    let flat = Vec3::new(point.x, tr.translation.y, point.z);
    if flat.distance_squared(tr.translation) > 1e-4 {
        tr.look_at(flat, Vec3::Y);
    }
}

/// Converts a core event into an animation (or `None` for instant events).
fn start_event(
    event: &GameEvent,
    battle: &Battle,
    entities: &UnitEntities,
    commands: &mut Commands,
    rig: &mut CameraRig,
    log: &mut LogMessages,
    transforms: &Query<&mut Transform>,
) -> Option<Anim> {
    let name = |id: UnitId| battle.unit(id).map_or("?".to_string(), |u| u.name.clone());
    let pos_of = |id: UnitId| -> Option<Vec3> {
        entities.0.get(&id).and_then(|e| transforms.get(*e).ok()).map(|t| t.translation)
    };
    match event {
        GameEvent::UnitMoved { unit, from, path } => {
            let entity = *entities.0.get(unit)?;
            Some(Anim::Walk {
                entity,
                from: grid_to_world(*from),
                points: path.iter().map(|p| grid_to_world(*p)).collect(),
                index: 0,
                t: 0.0,
            })
        }
        GameEvent::ShotFired { shooter, target, hit_chance, outcome, reaction } => {
            let s = pos_of(*shooter)?;
            let t = pos_of(*target)?;
            // Face each other.
            if let Some(e) = entities.0.get(shooter) {
                commands.entity(*e).insert(Transform::from_translation(s).looking_at(Vec3::new(t.x, s.y, t.z), Vec3::Y));
            }
            rig.target_focus = (s + t) / 2.0;
            let verb = if *reaction { "reaction-fires at" } else { "shoots at" };
            let result = match outcome {
                ShotOutcome::Miss => "miss".to_string(),
                ShotOutcome::Hit { damage, critical } => {
                    format!("{}{damage} dmg", if *critical { "CRIT " } else { "" })
                }
            };
            log.push(format!("{} {verb} {} ({hit_chance}%): {result}", name(*shooter), name(*target)));
            if *reaction {
                FloatingText::spawn(commands, s + Vec3::Y * 1.6, "OVERWATCH".into(), theme::TRACER);
            }
            Some(Anim::Shot {
                shooter: s + Vec3::Y * CHEST,
                target: t + Vec3::Y * CHEST,
                outcome: *outcome,
                reaction: *reaction,
                t: 0.0,
            })
        }
        GameEvent::UnitKilled { unit } => {
            log.push(format!("{} is down!", name(*unit)));
            let entity = *entities.0.get(unit)?;
            Some(Anim::Death { entity, t: 0.0 })
        }
        GameEvent::OverwatchSet { unit } | GameEvent::Hunkered { unit } | GameEvent::Reloaded { unit } => {
            let label = match event {
                GameEvent::OverwatchSet { .. } => "OVERWATCH",
                GameEvent::Hunkered { .. } => "HUNKER DOWN",
                _ => "RELOAD",
            };
            if let Some(p) = pos_of(*unit) {
                FloatingText::spawn(commands, p + Vec3::Y * 1.6, label.into(), theme::UI_TEXT);
                rig.target_focus = p;
            }
            Some(Anim::Wait { remaining: 0.5 })
        }
        GameEvent::TurnStarted { team, round } => {
            let who = match team {
                Team::Player => "XCOM",
                Team::Enemy => "ALIEN",
            };
            log.push(format!("- Round {round}: {who} turn -"));
            Some(Anim::Wait { remaining: 0.6 })
        }
        GameEvent::BattleEnded { winner } => {
            log.push(match winner {
                Team::Player => "Mission accomplished!".to_string(),
                Team::Enemy => "Squad wiped out. Mission failed.".to_string(),
            });
            None
        }
        GameEvent::ActionPointsChanged { .. } => None,
    }
}

//! Event playback: turns the instant results of the rules engine into
//! animations, camera moves and effects that play one after another.
//!
//! [`Playback`] holds a FIFO of [`GameEvent`]s. When idle it pops the next
//! event and converts it into a short sequence of [`Anim`]s (see
//! [`start_event`]). While anything is queued or running,
//! [`Playback::is_busy`] is `true`, which pauses player input and the AI.
//!
//! Presentation is data-driven: for every ability, the ruleset's
//! [`AbilityPresentation`](tactics_core::content::AbilityPresentation) says
//! which **animation slot** the user plays, which **camera script** runs,
//! how the **projectile** looks and how long the **wind-up** is. Unit
//! animations are requested through [`AnimCommand`]; camera scripts through
//! the [`CameraDirector`].
//!
//! To add new feedback (sound, particles...), match the event in
//! [`start_event`] — the rules engine doesn't need to change.

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use tactics_core::content::{AbilityEffect, ProjectileStyle};
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::animation::AnimCommand;
use crate::camera::CameraRig;
use crate::camera_director::{CameraContext, CameraDirector};
use crate::coords::tile_top;
use crate::hud::{FloatingText, LogMessages};
use crate::models::ModelLibrary;
use crate::state::BattleState;
use crate::theme;
use crate::units::UnitEntities;

pub struct PlaybackPlugin;

impl Plugin for PlaybackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Playback>().add_systems(Update, run_playback.in_set(FrameSet::Playback));
    }
}

/// Movement speed in world units (metres) per second: a steady jog.
pub const MOVE_SPEED: f32 = 3.3;
/// Height of the little hop when stepping up or down a level.
const HOP_HEIGHT: f32 = 0.35;
/// Flight time of a tracer/beam.
pub const SHOT_TIME: f32 = 0.45;
/// Pause before a shot so the player sees who is aiming at whom.
pub const AIM_TIME: f32 = 0.2;
/// Flight time of a lobbed projectile.
pub const ARC_TIME: f32 = 0.6;
/// Blast ring duration.
pub const EXPLOSION_TIME: f32 = 0.5;
/// Death topple duration (models without a death clip).
const DEATH_TIME: f32 = 0.6;
/// Height at which shots are fired / hit.
const CHEST: f32 = theme::UNIT_HEIGHT * 0.65;

/// A running animation step.
#[derive(Debug, Clone)]
pub enum Anim {
    /// Walk through `points` (world positions), one segment per tile.
    Walk { unit: UnitId, entity: Entity, from: Vec3, points: Vec<Vec3>, index: usize, t: f32 },
    /// Aim, then fire a projectile that lands at `AIM_TIME + SHOT_TIME`.
    Shot { shooter: Vec3, target: Vec3, target_unit: UnitId, outcome: ShotOutcome, style: ProjectileStyle, t: f32 },
    /// A lobbed projectile (grenade) from `from` to `to`.
    Projectile { from: Vec3, to: Vec3, t: f32 },
    /// Expanding blast ring (drawn by the overlay).
    Explosion { center: Vec3, radius: f32, t: f32 },
    /// Death: topple + sink (primitive) or the model's death clip, then despawn.
    Death { entity: Entity, t: f32, duration: f32, topple: bool },
    /// Do nothing for a while (wind-ups, banners).
    Wait { remaining: f32 },
}

/// Queue of events awaiting presentation plus the animations in progress.
#[derive(Resource, Default)]
pub struct Playback {
    queue: VecDeque<GameEvent>,
    anims: VecDeque<Anim>,
    current: Option<Anim>,
    /// Last unit that attacked (the "actor" of kill cams).
    last_attacker: Option<UnitId>,
}

impl Playback {
    pub fn enqueue(&mut self, events: impl IntoIterator<Item = GameEvent>) {
        self.queue.extend(events);
    }

    /// `true` while events are queued or an animation runs.
    pub fn is_busy(&self) -> bool {
        self.current.is_some() || !self.anims.is_empty() || !self.queue.is_empty()
    }

    /// The animation currently running (used by the overlay to draw effects).
    pub fn current(&self) -> Option<&Anim> {
        self.current.as_ref()
    }

    /// Drops everything (used when restarting a battle).
    pub fn clear(&mut self) {
        self.queue.clear();
        self.anims.clear();
        self.current = None;
        self.last_attacker = None;
    }
}

/// Everything [`start_event`] needs, bundled as one system parameter.
#[derive(SystemParam)]
pub struct Presenter<'w, 's> {
    commands: Commands<'w, 's>,
    state: Res<'w, BattleState>,
    entities: Res<'w, UnitEntities>,
    library: Res<'w, ModelLibrary>,
    clips: Res<'w, Assets<AnimationClip>>,
    director: ResMut<'w, CameraDirector>,
    rig: Single<'w, 's, &'static mut CameraRig>,
    log: ResMut<'w, LogMessages>,
    anim: MessageWriter<'w, AnimCommand>,
    board: MessageWriter<'w, crate::board::BoardDirty>,
    spawn_visual: MessageWriter<'w, crate::units::SpawnUnitVisual>,
}

/// Moves the camera unless a camera script owns it.
fn look_at(director: &CameraDirector, rig: &mut CameraRig, p: Vec3) {
    if !director.is_active() {
        rig.target_focus = p;
    }
}

/// Starts camera script `id` (subject to `chance` and the C toggle).
/// Returns whether a script started.
fn play_camera(
    rules: &Ruleset,
    director: &mut CameraDirector,
    id: Option<&String>,
    chance: u32,
    actor: Vec3,
    target: Vec3,
) -> bool {
    let Some(script) = id.and_then(|id| rules.camera_script(id)) else {
        return false;
    };
    if !director.enabled || !director.roll(chance) {
        return false;
    }
    director.play(script, CameraContext { actor, target });
    true
}

fn run_playback(
    time: Res<Time>,
    mut playback: ResMut<Playback>,
    mut transforms: Query<&mut Transform>,
    mut p: Presenter,
) {
    let dt = time.delta_secs();
    // Start the next animation, converting events as needed. Bounded so a long
    // run of instant events can't stall a frame.
    for _ in 0..64 {
        if playback.current.is_some() {
            break;
        }
        if let Some(next) = playback.anims.pop_front() {
            playback.current = Some(next);
            break;
        }
        let Some(event) = playback.queue.pop_front() else { break };
        let anims = start_event(&event, &mut playback, &mut p, &transforms);
        playback.anims.extend(anims);
    }

    let Some(anim) = playback.current.as_mut() else { return };
    let finished = match anim {
        Anim::Walk { unit, entity, from, points, index, t } => {
            // Constant speed: progress along a segment depends on its length,
            // so diagonals take longer than straight steps.
            let seg = |from: Vec3, i: usize| points.get(i).map_or(1.0, |p| from.distance(*p).max(0.05));
            // Enemies moving in the fog walk fast and the camera ignores them.
            let b = &p.state.battle;
            let hidden = b.fog()
                && b.unit(*unit).is_some_and(|u| u.team == Team::Enemy)
                && !b.team_can_see(Team::Player, crate::coords::world_to_grid(*from));
            *t += dt * MOVE_SPEED * if hidden { 6.0 } else { 1.0 } / seg(*from, *index);
            while *t >= 1.0 && *index < points.len() {
                let leftover = (*t - 1.0) * seg(*from, *index);
                *from = points[*index];
                *index += 1;
                *t = leftover / seg(*from, *index);
            }
            if let Ok(mut tr) = transforms.get_mut(*entity) {
                if let Some(next) = points.get(*index) {
                    let mut pos = from.lerp(*next, *t);
                    // Hop over a change of level instead of sliding up a slope.
                    if (next.y - from.y).abs() > 0.01 {
                        pos.y += (std::f32::consts::PI * *t).sin() * HOP_HEIGHT;
                    }
                    tr.translation = pos;
                    p.commands.entity(*entity).insert(crate::units::FaceTarget(Some(*next)));
                    if !hidden {
                        look_at(&p.director, &mut p.rig, tr.translation);
                    }
                } else {
                    tr.translation = *from;
                }
            }
            let done = *index >= points.len();
            if done {
                p.anim.write(AnimCommand::Walking { unit: *unit, on: false });
            }
            done
        }
        Anim::Shot { target, target_unit, outcome, t, .. } => {
            let before = *t;
            *t += dt;
            // The moment the projectile lands: result text + hit reaction.
            let land = AIM_TIME + SHOT_TIME;
            if before < land && *t >= land {
                spawn_result_text(&mut p.commands, *target + Vec3::Y * 0.6, outcome);
                if matches!(outcome, ShotOutcome::Hit { .. }) {
                    p.anim.write(AnimCommand::OneShot { unit: *target_unit, slot: "hit".into() });
                }
            }
            *t >= land + 0.2
        }
        Anim::Projectile { t, .. } => {
            *t += dt;
            *t >= ARC_TIME
        }
        Anim::Explosion { t, .. } => {
            *t += dt;
            *t >= EXPLOSION_TIME
        }
        Anim::Death { entity, t, duration, topple } => {
            *t += dt;
            let k = (*t / *duration).min(1.0);
            if *topple && let Ok(mut tr) = transforms.get_mut(*entity) {
                // Topple sideways while sinking.
                tr.rotation = Quat::from_rotation_z(k * std::f32::consts::FRAC_PI_2);
                tr.translation.y = -k * 0.4;
            }
            if *t >= *duration {
                p.commands.entity(*entity).despawn();
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

/// Default animation slot for an ability without an explicit one.
fn default_slot(def: &AbilityDef) -> &'static str {
    match &def.effect {
        AbilityEffect::Attack(_) if def.targeting == Targeting::Tile => "throw",
        AbilityEffect::Attack(_) => "shoot",
        AbilityEffect::Overwatch(_) => "overwatch",
        AbilityEffect::Defend { .. } => "hunker",
        AbilityEffect::Reload => "reload",
        AbilityEffect::Heal { .. } => "heal",
    }
}

/// Converts a core event into animation steps (empty for instant events).
///
/// **Add presentation for new event kinds here** (sounds, particles...).
fn start_event(event: &GameEvent, playback: &mut Playback, p: &mut Presenter, transforms: &Query<&mut Transform>) -> Vec<Anim> {
    let battle = &p.state.battle;
    let name = |id: UnitId| battle.unit(id).map_or("?".to_string(), |u| u.name.clone());
    let entity_of = |id: UnitId| p.entities.0.get(&id).copied();
    let pos_of = |id: UnitId| -> Option<Vec3> { entity_of(id).and_then(|e| transforms.get(e).ok()).map(|t| t.translation) };

    match event {
        GameEvent::UnitMoved { unit, from, path } => {
            let Some(entity) = entity_of(*unit) else { return vec![] };
            p.anim.write(AnimCommand::Walking { unit: *unit, on: true });
            vec![Anim::Walk {
                unit: *unit,
                entity,
                from: tile_top(battle.map(), *from),
                points: path.iter().map(|g| tile_top(battle.map(), *g)).collect(),
                index: 0,
                t: 0.0,
            }]
        }

        GameEvent::AbilityUsed { unit, ability, target } => {
            let Some(def) = battle.ability(ability).cloned() else { return vec![] };
            let pres = battle.rules().presentation_of(ability).cloned().unwrap_or_default();
            let target_text = match target {
                Target::Unit(t) if t != unit => format!(" on {}", name(*t)),
                _ => String::new(),
            };
            p.log.push(format!("{} uses {}{target_text}", name(*unit), def.name));
            let Some(actor) = pos_of(*unit) else { return vec![] };
            let aim = match target {
                Target::Unit(t) => pos_of(*t),
                Target::Tile(tile) => Some(tile_top(battle.map(), *tile)),
                Target::None => None,
            };

            // Face the target.
            if let (Some(aim), Some(e)) = (aim, entity_of(*unit))
                && aim.distance_squared(actor) > 0.01
            {
                p.commands.entity(e).insert(crate::units::FaceTarget(Some(aim)));
            }

            // Animation, camera, label.
            let slot = pres.animation.clone().unwrap_or_else(|| default_slot(&def).to_string());
            p.anim.write(AnimCommand::OneShot { unit: *unit, slot });
            let scripted = play_camera(battle.rules(), &mut p.director, pres.camera.as_ref(), pres.camera_chance, actor, aim.unwrap_or(actor));
            if !scripted {
                look_at(&p.director, &mut p.rig, aim.map_or(actor, |a| (a + actor) / 2.0));
            }
            FloatingText::spawn(&mut p.commands, actor + Vec3::Y * 1.6, def.name.to_uppercase(), theme::UI_TEXT);

            let mut steps = vec![Anim::Wait { remaining: pres.windup.max(0.0) }];
            if pres.projectile == ProjectileStyle::Arc
                && let Some(to) = aim
            {
                steps.push(Anim::Projectile { from: actor + Vec3::Y * CHEST, to: to + Vec3::Y * 0.2, t: 0.0 });
            }
            steps
        }

        GameEvent::Explosion { center, radius } => {
            let c = tile_top(battle.map(), *center);
            look_at(&p.director, &mut p.rig, c);
            vec![Anim::Explosion { center: c, radius: *radius, t: 0.0 }]
        }

        GameEvent::AttackResolved { attacker, ability, target, hit_chance, outcome, reaction, origin } => {
            playback.last_attacker = Some(*attacker);
            let (Some(s), Some(t)) = (pos_of(*attacker), pos_of(*target)) else { return vec![] };
            let verb = if *reaction { "reaction-fires at" } else { "hits at" };
            let result = match outcome {
                ShotOutcome::Miss => "miss".to_string(),
                ShotOutcome::Hit { damage, critical } => format!("{}{damage} dmg", if *critical { "CRIT " } else { "" }),
            };
            p.log.push(format!("  {} {verb} {} ({hit_chance}%): {result}", name(*attacker), name(*target)));

            let style = battle.rules().presentation_of(ability).map_or(ProjectileStyle::Tracer, |pr| pr.projectile);
            let from_attacker = *reaction || battle.unit(*attacker).is_some_and(|u| u.pos == *origin);
            if *reaction {
                FloatingText::spawn(&mut p.commands, s + Vec3::Y * 1.6, "OVERWATCH".into(), theme::TRACER);
                p.anim.write(AnimCommand::OneShot { unit: *attacker, slot: "shoot".into() });
            }
            if from_attacker && !matches!(style, ProjectileStyle::None | ProjectileStyle::Arc) {
                if let Some(e) = entity_of(*attacker) {
                    p.commands.entity(e).insert(crate::units::FaceTarget(Some(t)));
                }
                look_at(&p.director, &mut p.rig, (s + t) / 2.0);
                vec![Anim::Shot {
                    shooter: s + Vec3::Y * CHEST,
                    target: t + Vec3::Y * CHEST,
                    target_unit: *target,
                    outcome: *outcome,
                    style,
                    t: 0.0,
                }]
            } else {
                // Area damage / no projectile: show the result directly.
                spawn_result_text(&mut p.commands, t + Vec3::Y * (CHEST + 0.6), outcome);
                if matches!(outcome, ShotOutcome::Hit { .. }) {
                    p.anim.write(AnimCommand::OneShot { unit: *target, slot: "hit".into() });
                }
                vec![Anim::Wait { remaining: 0.2 }]
            }
        }

        GameEvent::UnitHealed { unit, amount } => {
            p.log.push(format!("  {} recovers {amount} HP", name(*unit)));
            if let Some(pos) = pos_of(*unit) {
                FloatingText::spawn(&mut p.commands, pos + Vec3::Y * 1.3, format!("+{amount}"), theme::HEAL);
            }
            vec![Anim::Wait { remaining: 0.4 }]
        }

        GameEvent::UnitKilled { unit } => {
            p.log.push(format!("{} is down!", name(*unit)));
            let Some(entity) = entity_of(*unit) else { return vec![] };
            let victim = pos_of(*unit).unwrap_or_default();

            // Stop turning so the death animation owns the rotation.
            p.commands.entity(entity).insert(crate::units::FaceTarget(None));

            // Kill camera: actor = killer, target = victim.
            let settings = battle.rules().presentation.clone();
            let killer = playback.last_attacker.and_then(pos_of).unwrap_or(victim);
            play_camera(battle.rules(), &mut p.director, settings.kill_camera.as_ref(), settings.kill_camera_chance, killer, victim);

            // The model's death clip if it has one, else the procedural topple.
            p.anim.write(AnimCommand::Died { unit: *unit });
            let model = battle.unit(*unit).map(|u| u.model.clone()).unwrap_or_default();
            // The model's death animation (glTF clip or skeleton) if it has
            // one, else the procedural topple.
            let clip_len = p.library.animation_length(&model, "death", &p.clips);
            vec![match clip_len {
                Some(len) => Anim::Death { entity, t: 0.0, duration: len + 0.4, topple: false },
                None => Anim::Death { entity, t: 0.0, duration: DEATH_TIME, topple: true },
            }]
        }

        GameEvent::TurnStarted { team, round } => {
            let who = match team {
                Team::Player => "XCOM",
                Team::Enemy => "ALIEN",
            };
            p.log.push(format!("- Round {round}: {who} turn -"));
            vec![Anim::Wait { remaining: 0.6 }]
        }

        GameEvent::BattleEnded { winner } => {
            p.log.push(match winner {
                Team::Player => "Mission accomplished!".to_string(),
                Team::Enemy => "Squad wiped out. Mission failed.".to_string(),
            });
            vec![]
        }

        GameEvent::ActionPointsChanged { .. } => vec![],

        // --- mission / script events -------------------------------------
        GameEvent::Message { text } => {
            p.log.push(format!(">> {text}"));
            let at = p.rig.target_focus + Vec3::Y * 1.5;
            FloatingText::spawn(&mut p.commands, at, text.clone(), theme::TRACER);
            vec![Anim::Wait { remaining: 0.9 }]
        }
        GameEvent::ScriptError { message } => {
            p.log.push(format!("Script error: {message}"));
            warn!("script error: {message}");
            vec![]
        }
        GameEvent::UnitSpawned { unit } => {
            p.spawn_visual.write(crate::units::SpawnUnitVisual(*unit));
            if let Some(u) = battle.unit(*unit) {
                p.log.push(format!("{} arrives", u.name));
                look_at(&p.director, &mut p.rig, tile_top(battle.map(), u.pos));
            }
            vec![Anim::Wait { remaining: 0.5 }]
        }
        GameEvent::UnitDamaged { unit, amount } => {
            if let Some(pos) = pos_of(*unit) {
                FloatingText::spawn(&mut p.commands, pos + Vec3::Y * (CHEST + 0.6), format!("-{amount}"), theme::TARGET);
            }
            p.log.push(format!("  {} takes {amount} damage", name(*unit)));
            p.anim.write(AnimCommand::OneShot { unit: *unit, slot: "hit".into() });
            vec![Anim::Wait { remaining: 0.25 }]
        }
        GameEvent::PropDamaged { prop, hp } => {
            if let Some(pr) = battle.prop(*prop) {
                let at = tile_top(battle.map(), pr.pos) + Vec3::Y * 1.0;
                FloatingText::spawn(&mut p.commands, at, format!("{} {hp} hp", pr.kind), theme::UI_DIM);
            }
            vec![]
        }
        GameEvent::PropDestroyed { prop } => {
            if let Some(pr) = battle.prop(*prop) {
                p.log.push(format!("  {} destroyed", pr.kind));
            }
            p.board.write(crate::board::BoardDirty);
            vec![Anim::Wait { remaining: 0.2 }]
        }
        GameEvent::TerrainChanged { .. } => {
            p.board.write(crate::board::BoardDirty);
            vec![]
        }

        // --- mission mechanics ---------------------------------------------
        GameEvent::Alarm { by } => {
            let text = match by {
                Some(u) => format!("ALARM! {} spotted the squad", name(*u)),
                None => "ALARM! Concealment broken".to_string(),
            };
            p.log.push(text.clone());
            let at = by.and_then(pos_of).unwrap_or(p.rig.target_focus);
            if by.is_some() {
                look_at(&p.director, &mut p.rig, at);
            }
            FloatingText::spawn(&mut p.commands, at + Vec3::Y * 2.0, "ALARM!".into(), theme::TARGET);
            if !p.director.is_active() {
                p.rig.shake = p.rig.shake.max(0.4);
            }
            vec![Anim::Wait { remaining: 1.0 }]
        }
        GameEvent::UnitAlerted { unit } => {
            // Only announce what the squad can see; the rest wake in the dark.
            if battle.is_revealed(*unit)
                && let Some(pos) = pos_of(*unit)
            {
                FloatingText::spawn(&mut p.commands, pos + Vec3::Y * (CHEST + 0.9), "!".into(), theme::TARGET);
                p.log.push(format!("  {} is alerted", name(*unit)));
                return vec![Anim::Wait { remaining: 0.15 }];
            }
            vec![]
        }
        GameEvent::DoorChanged { prop, open } => {
            if *open
                && let Some(pr) = battle.prop(*prop)
            {
                let at = tile_top(battle.map(), pr.pos) + Vec3::Y * 2.0;
                FloatingText::spawn(&mut p.commands, at, "door opens".into(), theme::UI_DIM);
            }
            p.board.write(crate::board::BoardDirty);
            vec![Anim::Wait { remaining: 0.15 }]
        }
        GameEvent::ObjectiveChanged { text } => {
            p.log.push(format!("Objective: {text}"));
            vec![]
        }
    }
}

/// "-4", "CRIT 6" or "MISSED" above a unit.
fn spawn_result_text(commands: &mut Commands, at: Vec3, outcome: &ShotOutcome) {
    let (text, color) = match outcome {
        ShotOutcome::Miss => ("MISSED".to_string(), theme::UI_DIM),
        ShotOutcome::Hit { damage, critical: true } => (format!("CRIT {damage}"), theme::TRACER),
        ShotOutcome::Hit { damage, .. } => (format!("-{damage}"), theme::TARGET),
    };
    FloatingText::spawn(commands, at, text, color);
}

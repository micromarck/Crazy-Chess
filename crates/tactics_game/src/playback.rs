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
        app.init_resource::<Playback>()
            .add_systems(Update, (run_playback, run_background_deaths).in_set(FrameSet::Playback));
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
    /// A ballistic volley: each traced bullet flies from `muzzle` to its end,
    /// rounds `interval` apart (pellets of one round together).
    /// Rounds (or grenade fragments) leaving `muzzle` after `aim` seconds,
    /// `per_round` at a time, `interval` apart.
    Volley { muzzle: Vec3, bullets: Vec<VolleyBullet>, style: ProjectileStyle, aim: f32, interval: f32, per_round: usize, t: f32 },
    /// A lobbed projectile (grenade) from `from` to `to`.
    Projectile { from: Vec3, to: Vec3, t: f32 },
    /// Expanding blast ring (drawn by the overlay).
    Explosion { center: Vec3, radius: f32, t: f32 },
    /// Death: topple + sink (primitive) or the model's death clip, then despawn.
    Death { entity: Entity, t: f32, duration: f32, topple: bool },
    /// Do nothing for a while (wind-ups, banners).
    Wait { remaining: f32 },
}

/// A death playing alongside everything else (weak horde enemies): same
/// look as [`Anim::Death`], but playback doesn't wait for it.
#[derive(Component, Debug)]
pub struct BackgroundDeath {
    t: f32,
    duration: f32,
    topple: bool,
}

fn run_background_deaths(time: Res<Time>, mut commands: Commands, mut q: Query<(Entity, &mut BackgroundDeath, &mut Transform)>) {
    for (entity, mut d, mut tr) in &mut q {
        d.t += time.delta_secs();
        let k = (d.t / d.duration).min(1.0);
        if d.topple {
            tr.rotation = Quat::from_rotation_z(k * std::f32::consts::FRAC_PI_2);
            tr.translation.y = -k * 0.4;
        }
        if d.t >= d.duration {
            commands.entity(entity).despawn();
        }
    }
}

/// One bullet of an [`Anim::Volley`].
#[derive(Debug, Clone)]
pub struct VolleyBullet {
    pub end: Vec3,
    pub hit: Option<UnitId>,
    pub damage: i32,
    pub critical: bool,
    pub blocked: bool,
    /// Its landing has been shown.
    pub landed: bool,
}

/// Seconds a bullet takes to fly, per tile (tracers are fast).
pub const BULLET_TIME_PER_TILE: f32 = 0.025;

impl VolleyBullet {
    /// When bullet `i` leaves the muzzle and when it lands.
    pub fn timing(&self, i: usize, per_round: usize, interval: f32, muzzle: Vec3, aim: f32) -> (f32, f32) {
        let fire = aim + (i / per_round.max(1)) as f32 * interval;
        (fire, fire + (self.end.distance(muzzle) * BULLET_TIME_PER_TILE).max(0.05))
    }
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
    banner: MessageWriter<'w, crate::tooltip::Banner>,
    settings: Res<'w, crate::settings::GameSettings>,
}

/// Moves the camera unless a camera script owns it.
fn look_at(director: &CameraDirector, rig: &mut CameraRig, p: Vec3) {
    if !director.is_active() {
        rig.follow(p);
    }
}

/// Keeps two points in view (shooter and target), unless a script owns the
/// camera. With the action camera off it only nudges towards the midpoint.
fn frame(director: &CameraDirector, rig: &mut CameraRig, action_camera: bool, a: Vec3, b: Vec3) {
    if director.is_active() {
        return;
    }
    if action_camera {
        rig.frame(a, b);
    } else {
        rig.follow((a + b) / 2.0);
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
            // Accelerate out of the first step and settle into the last one.
            let last = points.len().saturating_sub(1);
            let ease = if *index == 0 && last > 0 {
                0.6 + 0.4 * *t
            } else if *index >= last {
                1.0 - 0.35 * *t
            } else {
                1.0
            };
            // Slow units (zombies) walk slower; moves in the fog are near-instant
            // so a big horde shuffling around out of sight doesn't stall the turn.
            let pace = b.unit(*unit).map_or(1.0, |u| (u.stats.mobility as f32 / 12.0).clamp(0.45, 1.3));
            *t += dt * MOVE_SPEED * ease * if hidden { 40.0 } else { pace } / seg(*from, *index);
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
                    if !hidden && !p.director.is_active() {
                        p.rig.track(tr.translation);
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
        Anim::Volley { muzzle, bullets, interval, per_round, aim, t, .. } => {
            *t += dt;
            let mut last = 0.0f32;
            for (i, b) in bullets.iter_mut().enumerate() {
                let (_, land) = b.timing(i, *per_round, *interval, *muzzle, *aim);
                last = last.max(land);
                if !b.landed && *t >= land {
                    b.landed = true;
                    if let Some(u) = b.hit {
                        let (text, color) = if b.critical { (format!("CRIT {}", b.damage), theme::TRACER) } else { (format!("-{}", b.damage), theme::TARGET) };
                        FloatingText::spawn(&mut p.commands, b.end + Vec3::Y * 0.5, text, color);
                        p.anim.write(AnimCommand::OneShot { unit: u, slot: "hit".into() });
                    } else if b.blocked {
                        FloatingText::spawn(&mut p.commands, b.end + Vec3::Y * 0.3, "BLOCKED".into(), theme::UI_DIM);
                    }
                }
            }
            *t >= last + 0.25
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
        AbilityEffect::Suppress(_) => "overwatch",
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
                match aim {
                    Some(a) => frame(&p.director, &mut p.rig, p.settings.action_camera, actor, a),
                    None => look_at(&p.director, &mut p.rig, actor),
                }
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

        GameEvent::Volley { attacker, ability, target, origin, hit_chance, reaction, suppress, bullets } => {
            playback.last_attacker = Some(*attacker);
            let Some(s) = pos_of(*attacker) else { return vec![] };
            let t = pos_of(*target).unwrap_or(s);
            let muzzle = Vec3::new(origin[0], origin[2], -origin[1]);
            let hits_on_target = bullets.iter().filter(|b| b.hit == Some(*target)).count();
            let total: i32 = bullets.iter().filter(|b| b.hit == Some(*target)).map(|b| b.damage).sum();
            let what = if *suppress { "suppresses" } else if *reaction { "reaction-fires at" } else { "fires at" };
            p.log.push(format!(
                "  {} {what} {}: {} round(s) at {hit_chance}%, {hits_on_target} hit{}",
                name(*attacker),
                name(*target),
                bullets.len(),
                if total > 0 { format!(" for {total}") } else { String::new() }
            ));
            // Anyone else who caught a bullet.
            let attacker_team = battle.unit(*attacker).map(|u| u.team);
            let mut others: Vec<UnitId> = bullets.iter().filter_map(|b| b.hit).filter(|u| u != target).collect();
            others.dedup();
            for u in others {
                let friendly = battle.unit(u).map(|x| x.team) == attacker_team;
                let label = if friendly { "FRIENDLY FIRE" } else { "STRAY HIT" };
                p.log.push(format!("  {} is hit by {}!", name(u), if friendly { "friendly fire" } else { "a stray round" }));
                if let Some(pos) = pos_of(u) {
                    FloatingText::spawn(&mut p.commands, pos + Vec3::Y * 1.9, label.into(), if friendly { theme::TARGET } else { theme::TRACER });
                }
            }
            if *reaction {
                let label = if battle.unit(*target).is_some_and(|u| u.status.suppressed_by.is_none()) && !*suppress { "REACTION FIRE" } else { "OVERWATCH" };
                FloatingText::spawn(&mut p.commands, s + Vec3::Y * 1.6, label.into(), theme::TRACER);
                p.anim.write(AnimCommand::OneShot { unit: *attacker, slot: "shoot".into() });
            }
            if *suppress {
                FloatingText::spawn(&mut p.commands, t + Vec3::Y * 1.9, "SUPPRESSED".into(), theme::TRACER);
            }
            if hits_on_target == 0 && !bullets.is_empty() {
                FloatingText::spawn(&mut p.commands, t + Vec3::Y * (CHEST + 0.9), "MISSED".into(), theme::UI_DIM);
            }
            if let Some(e) = entity_of(*attacker) {
                p.commands.entity(e).insert(crate::units::FaceTarget(Some(t)));
                // Firing from a lean tile: peek around the corner.
                let from = GridPos::new(origin[0].round() as i32, origin[1].round() as i32);
                if battle.unit(*attacker).is_some_and(|u| u.pos != from) {
                    p.commands.entity(e).insert(crate::units::Leaning { toward: tile_top(battle.map(), from), elapsed: 0.0 });
                }
            }
            frame(&p.director, &mut p.rig, p.settings.action_camera, s, t);
            let style = battle.rules().presentation_of(ability).map_or(ProjectileStyle::Tracer, |pr| pr.projectile);
            let style = if matches!(style, ProjectileStyle::None | ProjectileStyle::Arc) { ProjectileStyle::Tracer } else { style };
            let per_round = battle.unit(*attacker).map_or(1, |u| u.weapon.pellets.max(1) as usize);
            let per_round = if per_round > 1 && bullets.len() % per_round == 0 { per_round } else { 1 };
            let interval = if *suppress { 0.06 } else { 0.09 };
            let bullets = bullets
                .iter()
                .map(|b| VolleyBullet {
                    end: Vec3::new(b.end[0], b.end[2], -b.end[1]),
                    hit: b.hit,
                    damage: b.damage,
                    critical: b.critical,
                    blocked: b.blocked,
                    landed: false,
                })
                .collect();
            vec![Anim::Volley { muzzle, bullets, style, aim: AIM_TIME, interval, per_round, t: 0.0 }]
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
                frame(&p.director, &mut p.rig, p.settings.action_camera, s, t);
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
            p.anim.write(AnimCommand::Died { unit: *unit });
            let model = battle.unit(*unit).map(|u| u.model.clone()).unwrap_or_default();
            let clip_len = p.library.animation_length(&model, "death", &p.clips);
            let (duration, topple) = match clip_len {
                Some(len) => (len + 0.4, false),
                None => (DEATH_TIME, true),
            };

            // Weak horde enemies (zombies) die in the background: no kill
            // camera and no waiting, so everyone a burst or a grenade drops
            // falls together and the action keeps going.
            if battle.unit(*unit).is_some_and(|u| u.ai.horde) {
                p.commands.entity(entity).insert(BackgroundDeath { t: 0.0, duration, topple });
                return vec![];
            }

            // Kill camera: actor = killer, target = victim.
            let settings = battle.rules().presentation.clone();
            let killer = playback.last_attacker.and_then(pos_of).unwrap_or(victim);
            play_camera(battle.rules(), &mut p.director, settings.kill_camera.as_ref(), settings.kill_camera_chance, killer, victim);

            // The model's death animation (glTF clip or skeleton) if it has
            // one, else the procedural topple; playback waits for it.
            vec![Anim::Death { entity, t: 0.0, duration, topple }]
        }

        GameEvent::TurnStarted { team, round } => {
            let who = match team {
                Team::Player => "XCOM",
                Team::Enemy => "ALIEN",
            };
            p.log.push(format!("- Round {round}: {who} turn -"));
            let (text, color) = match team {
                Team::Player => (format!("XCOM TURN  -  ROUND {round}"), theme::PLAYER),
                Team::Enemy => ("ALIEN ACTIVITY".to_string(), theme::ENEMY),
            };
            if !battle.is_over() {
                p.banner.write(crate::tooltip::Banner { text, color });
            }
            vec![Anim::Wait { remaining: 0.6 }]
        }

        GameEvent::BattleEnded { winner } => {
            p.banner.write(match winner {
                Team::Player => crate::tooltip::Banner { text: "MISSION ACCOMPLISHED".into(), color: theme::HEAL },
                Team::Enemy => crate::tooltip::Banner { text: "MISSION FAILED".into(), color: theme::TARGET },
            });
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
        GameEvent::Callout { unit, text } => {
            // Only for units the squad can see (no giving away hidden enemies).
            if battle.is_revealed(*unit) {
                p.log.push(format!("  {}: {text}", name(*unit)));
                if let Some(pos) = pos_of(*unit) {
                    FloatingText::spawn(&mut p.commands, pos + Vec3::Y * 2.1, text.clone(), theme::TRACER);
                }
                return vec![Anim::Wait { remaining: 0.35 }];
            }
            vec![]
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
            p.board.write(crate::board::BoardDirty(battle.prop(*prop).map(|pr| pr.pos)));
            vec![Anim::Wait { remaining: 0.2 }]
        }
        GameEvent::TerrainChanged { pos } => {
            p.board.write(crate::board::BoardDirty(Some(*pos)));
            vec![]
        }
        GameEvent::WallDamaged { pos, hp } => {
            let at = tile_top(battle.map(), *pos) + Vec3::Y * 1.2;
            FloatingText::spawn(&mut p.commands, at, format!("wall {hp}"), theme::UI_DIM);
            vec![]
        }
        GameEvent::WallDestroyed { pos } => {
            let at = tile_top(battle.map(), *pos) + Vec3::Y * 1.0;
            FloatingText::spawn(&mut p.commands, at, "BREACHED".into(), theme::EXPLOSION);
            p.board.write(crate::board::BoardDirty(Some(*pos)));
            vec![]
        }
        GameEvent::Shrapnel { center, fragments, .. } => {
            let hits = fragments.iter().filter(|f| f.hit.is_some()).count();
            if hits > 0 {
                p.log.push(format!("  shrapnel: {hits} hit{}", if hits == 1 { "" } else { "s" }));
            }
            let bullets = fragments
                .iter()
                .map(|b| VolleyBullet {
                    end: Vec3::new(b.end[0], b.end[2], -b.end[1]),
                    hit: b.hit,
                    damage: b.damage,
                    critical: false,
                    blocked: false,
                    landed: false,
                })
                .collect::<Vec<_>>();
            let n = bullets.len().max(1);
            // All fragments at once, no aiming.
            vec![Anim::Volley {
                muzzle: Vec3::new(center[0], center[2], -center[1]),
                bullets,
                style: ProjectileStyle::Tracer,
                aim: 0.0,
                interval: 0.0,
                per_round: n,
                t: 0.0,
            }]
        }
        GameEvent::SupplyFound { unit, prop, text } => {
            p.log.push(format!("  {} picks up supplies: {text}", name(*unit)));
            if let Some(pos) = pos_of(*unit) {
                FloatingText::spawn(&mut p.commands, pos + Vec3::Y * 1.9, text.to_uppercase(), theme::HEAL);
            }
            p.board.write(crate::board::BoardDirty(battle.prop(*prop).map(|pr| pr.pos)));
            vec![Anim::Wait { remaining: 0.3 }]
        }

        // --- mission mechanics ---------------------------------------------
        GameEvent::Alarm { by } => {
            let text = match by {
                Some(u) => format!("ALARM! {} spotted the squad", name(*u)),
                None => "ALARM! Concealment broken".to_string(),
            };
            p.log.push(text.clone());
            p.banner.write(crate::tooltip::Banner { text: "ALARM RAISED".into(), color: theme::TARGET });
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
                // Pod reveal: glance at it so the player sees who's coming.
                FloatingText::spawn(&mut p.commands, pos + Vec3::Y * (CHEST + 0.9), "CONTACT!".into(), theme::TARGET);
                p.log.push(format!("  {} is alerted", name(*unit)));
                look_at(&p.director, &mut p.rig, pos);
                return vec![Anim::Wait { remaining: 0.35 }];
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
            p.board.write(crate::board::BoardDirty(battle.prop(*prop).map(|pr| pr.pos)));
            vec![Anim::Wait { remaining: 0.15 }]
        }
        GameEvent::MoraleChanged { unit, level, .. } => {
            use tactics_core::content::FearLevel;
            let (text, color) = match level {
                FearLevel::Steady => ("RALLIED", theme::HEAL),
                FearLevel::Shaken => ("SHAKEN", theme::TRACER),
                FearLevel::Panicked => ("PANICKED!", theme::BEAM),
            };
            if battle.is_revealed(*unit) {
                if let Some(pos) = pos_of(*unit) {
                    FloatingText::spawn(&mut p.commands, pos + Vec3::Y * (CHEST + 1.1), text.into(), color);
                }
                p.log.push(format!("  {} is {}", name(*unit), level.name().to_lowercase()));
                if *level == FearLevel::Panicked {
                    p.anim.write(AnimCommand::OneShot { unit: *unit, slot: "hit".into() });
                    return vec![Anim::Wait { remaining: 0.35 }];
                }
            }
            vec![]
        }
        GameEvent::ZoneChanged { tag } => {
            if let Some(z) = battle.zones().iter().find(|z| &z.tag == tag && z.visible) {
                p.log.push(format!("Marked: {}", z.label));
                let (cx, cy) = z.center();
                let at = tile_top(battle.map(), GridPos::new(cx.round() as i32, cy.round() as i32));
                FloatingText::spawn(&mut p.commands, at + Vec3::Y * 2.0, z.label.clone(), theme::ZONE_EVAC);
                look_at(&p.director, &mut p.rig, at);
            }
            // Redraw the chunks the zone covers.
            if let Some(z) = battle.zones().iter().find(|z| &z.tag == tag) {
                let step = crate::board::CHUNK as usize;
                for y in (z.min.y..=z.max.y).step_by(step).chain([z.max.y]) {
                    for x in (z.min.x..=z.max.x).step_by(step).chain([z.max.x]) {
                        p.board.write(crate::board::BoardDirty(Some(GridPos::new(x, y))));
                    }
                }
            }
            vec![Anim::Wait { remaining: 0.3 }]
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

//! Camera scripts: cinematic camera moves triggered by actions.
//!
//! A [`CameraScriptDef`] (from any module) is a list of [`CameraStep`]s. When
//! playback starts one (an ability's `AbilityPresentation::camera`, or the kill
//! camera), the [`CameraDirector`]:
//!
//! 1. remembers the player's camera,
//! 2. runs the steps in order (`Together([...])` runs several at once),
//!    tweening the orbit rig's *targets* with smooth easing,
//! 3. hands control back (`Restore`, or simply at the end).
//!
//! While a script runs, player camera input is ignored and gameplay waits.
//! **Space** skips a script and **C** toggles cinematic cameras on/off.
//! Durations use real time, so slow motion (`TimeScale`) doesn't stretch them.
//!
//! Anchors (`Actor`, `Target`, `Midpoint`) are resolved from the
//! [`CameraContext`] given when the script starts.
//!
//! ## Adding a step type
//! Add a variant to `CameraStep` (tactics_core `content/presentation.rs`) and
//! handle it in [`Running::start`] and [`Running::apply`].

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use tactics_core::content::{CameraAnchor, CameraScriptDef, CameraStep};

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::hud::LogMessages;

pub struct CameraDirectorPlugin;

impl Plugin for CameraDirectorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraDirector>()
            .add_systems(Update, director_hotkeys.in_set(FrameSet::Input))
            .add_systems(Update, run_director.in_set(FrameSet::Playback));
    }
}

/// World positions the anchors refer to.
#[derive(Debug, Clone, Copy, Default)]
pub struct CameraContext {
    pub actor: Vec3,
    pub target: Vec3,
}

/// The rig values a script manipulates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigSnapshot {
    pub focus: Vec3,
    pub yaw: f32,
    pub distance: f32,
    pub pitch: f32,
}

impl RigSnapshot {
    fn of(rig: &CameraRig) -> Self {
        Self { focus: rig.target_focus, yaw: rig.target_yaw, distance: rig.target_distance, pitch: rig.target_pitch }
    }
}

/// A leaf step in progress.
#[derive(Debug, Clone)]
struct Running {
    step: CameraStep,
    elapsed: f32,
    duration: f32,
    start: RigSnapshot,
    /// Resolved end yaw / focus for steps that need it.
    end_yaw: f32,
    end_focus: Vec3,
}

/// Runs camera scripts. Gameplay code calls [`CameraDirector::play`].
#[derive(Resource)]
pub struct CameraDirector {
    /// Cinematic cameras on/off (C key).
    pub enabled: bool,
    queue: VecDeque<CameraStep>,
    running: Vec<Running>,
    /// Camera the player had before the script.
    saved: Option<RigSnapshot>,
    ctx: CameraContext,
    /// Game speed requested by the script (applied to `Time<Virtual>`).
    time_scale: f32,
    /// Presentation-only randomness for `camera_chance` (never the battle RNG).
    rng: u64,
}

impl Default for CameraDirector {
    fn default() -> Self {
        Self {
            enabled: true,
            queue: VecDeque::new(),
            running: Vec::new(),
            saved: None,
            ctx: CameraContext::default(),
            time_scale: 1.0,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }
}

impl CameraDirector {
    /// Starts `script`. Replaces a running script but keeps the original
    /// player camera so `Restore` still returns there.
    pub fn play(&mut self, script: &CameraScriptDef, ctx: CameraContext) {
        if !self.enabled {
            return;
        }
        self.queue = script.steps.iter().cloned().collect();
        self.running.clear();
        self.ctx = ctx;
    }

    /// Rolls a `chance` (0-100) for optional cinematics.
    pub fn roll(&mut self, chance: u32) -> bool {
        // xorshift64*: cheap, fine for "should this camera play".
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let r = (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % 100;
        (r as u32) < chance
    }

    /// Is a script running? (Gameplay and player camera input wait.)
    pub fn is_active(&self) -> bool {
        !self.queue.is_empty() || !self.running.is_empty()
    }

    /// Jumps to a quick restore of the player camera.
    pub fn skip(&mut self) {
        if self.is_active() {
            self.queue.clear();
            self.running.clear();
            self.queue.push_back(CameraStep::Restore { duration: 0.25 });
        }
    }

    fn anchor(&self, anchor: CameraAnchor, rig: &CameraRig) -> Vec3 {
        match anchor {
            CameraAnchor::Actor => self.ctx.actor,
            CameraAnchor::Target => self.ctx.target,
            CameraAnchor::Midpoint => (self.ctx.actor + self.ctx.target) / 2.0,
            CameraAnchor::Current => rig.target_focus,
        }
    }
}

/// Flattens `Together` into leaves.
fn leaves(step: CameraStep, out: &mut Vec<CameraStep>) {
    match step {
        CameraStep::Together(steps) => steps.into_iter().for_each(|s| leaves(s, out)),
        other => out.push(other),
    }
}

/// Angle `to` shifted by full turns to be within half a turn of `from`.
fn nearest_angle(from: f32, to: f32) -> f32 {
    let mut d = (to - from) % TAU;
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    from + d
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Running {
    fn start(step: CameraStep, director: &mut CameraDirector, rig: &CameraRig) -> Self {
        let start = RigSnapshot::of(rig);
        let mut end_yaw = start.yaw;
        let mut end_focus = start.focus;
        let duration = match &step {
            CameraStep::Focus { on, duration } => {
                end_focus = director.anchor(*on, rig);
                *duration
            }
            CameraStep::FaceAlong { from, to, offset_degrees, duration } => {
                let d = director.anchor(*to, rig) - director.anchor(*from, rig);
                if d.x.abs() + d.z.abs() > 1e-3 {
                    // The rig looks along -(sin yaw, _, cos yaw).
                    let yaw = (-d.x).atan2(-d.z) + offset_degrees.to_radians();
                    end_yaw = nearest_angle(start.yaw, yaw);
                }
                *duration
            }
            CameraStep::Orbit { degrees, duration } => {
                end_yaw = start.yaw + degrees.to_radians();
                *duration
            }
            CameraStep::Zoom { duration, .. }
            | CameraStep::Pitch { duration, .. }
            | CameraStep::Shake { duration, .. }
            | CameraStep::Wait { duration }
            | CameraStep::Restore { duration } => *duration,
            CameraStep::TimeScale { scale } => {
                director.time_scale = scale.clamp(0.05, 4.0);
                0.0
            }
            CameraStep::Together(_) => 0.0, // flattened before reaching here
        };
        if matches!(step, CameraStep::Restore { .. }) {
            director.time_scale = 1.0;
        }
        Self { step, elapsed: 0.0, duration, start, end_yaw, end_focus }
    }

    /// Applies the step at its current progress. Returns `true` when done.
    fn apply(&mut self, dt: f32, rig: &mut CameraRig, saved: Option<RigSnapshot>) -> bool {
        self.elapsed += dt;
        let k = if self.duration <= 0.0 { 1.0 } else { smoothstep(self.elapsed / self.duration) };
        let s = self.start;
        match &self.step {
            CameraStep::Focus { .. } => rig.target_focus = s.focus.lerp(self.end_focus, k),
            CameraStep::FaceAlong { .. } | CameraStep::Orbit { .. } => {
                rig.target_yaw = s.yaw + (self.end_yaw - s.yaw) * k;
            }
            CameraStep::Zoom { distance, .. } => {
                let d = distance.clamp(CameraRig::MIN_DISTANCE * 0.5, CameraRig::MAX_DISTANCE);
                rig.target_distance = s.distance + (d - s.distance) * k;
            }
            CameraStep::Pitch { degrees, .. } => {
                let p = degrees.clamp(10.0, 89.0).to_radians();
                rig.target_pitch = s.pitch + (p - s.pitch) * k;
            }
            CameraStep::Shake { strength, .. } => rig.shake = strength * (1.0 - k),
            CameraStep::Restore { .. } => {
                if let Some(to) = saved {
                    rig.target_focus = s.focus.lerp(to.focus, k);
                    rig.target_yaw = s.yaw + (nearest_angle(s.yaw, to.yaw) - s.yaw) * k;
                    rig.target_distance = s.distance + (to.distance - s.distance) * k;
                    rig.target_pitch = s.pitch + (to.pitch - s.pitch) * k;
                }
                rig.shake = 0.0;
            }
            CameraStep::Wait { .. } | CameraStep::TimeScale { .. } | CameraStep::Together(_) => {}
        }
        self.elapsed >= self.duration
    }
}

fn run_director(
    real: Res<Time<Real>>,
    mut virt: ResMut<Time<Virtual>>,
    mut director: ResMut<CameraDirector>,
    mut rig: Single<&mut CameraRig>,
) {
    if !director.is_active() {
        return;
    }
    if director.saved.is_none() {
        director.saved = Some(RigSnapshot::of(&rig));
    }
    let dt = real.delta_secs();

    // Start the next step when the previous one finished.
    if director.running.is_empty()
        && let Some(step) = director.queue.pop_front() {
            let mut flat = Vec::new();
            leaves(step, &mut flat);
            let started: Vec<Running> = flat.into_iter().map(|s| Running::start(s, &mut director, &rig)).collect();
            director.running = started;
        }

    let saved = director.saved;
    let mut running = std::mem::take(&mut director.running);
    running.retain_mut(|r| !r.apply(dt, &mut rig, saved));
    director.running = running;

    let scale = director.time_scale;
    if (virt.relative_speed() - scale).abs() > f32::EPSILON {
        virt.set_relative_speed(scale);
    }

    // Script finished: always leave normal speed and no shake behind.
    if !director.is_active() {
        director.saved = None;
        director.time_scale = 1.0;
        virt.set_relative_speed(1.0);
        rig.shake = 0.0;
    }
}

fn director_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut director: ResMut<CameraDirector>, mut log: ResMut<LogMessages>) {
    if keys.just_pressed(KeyCode::Space) {
        director.skip();
    }
    if keys.just_pressed(KeyCode::KeyC) {
        director.enabled = !director.enabled;
        log.push(format!("Cinematic cameras {}", if director.enabled { "ON" } else { "OFF" }));
        if !director.enabled {
            director.skip();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_take_the_short_way() {
        assert!((nearest_angle(0.1, TAU - 0.1) - (-0.1)).abs() < 1e-5);
        assert!((nearest_angle(3.0, -3.0) - (TAU - 3.0)).abs() < 1e-5);
    }

    #[test]
    fn chance_roll_is_roughly_fair() {
        let mut d = CameraDirector::default();
        let hits = (0..1000).filter(|_| d.roll(50)).count();
        assert!((400..600).contains(&hits), "{hits}");
        assert!((0..100).all(|_| !d.roll(0)));
        assert!((0..100).all(|_| d.roll(100)));
    }
}

//! Tactical orbit camera.
//!
//! The camera orbits a *focus point* on the ground at a fixed pitch, like most
//! squad-tactics games. All motion is smoothed: input changes the *target*
//! values and the rig eases towards them every frame, which also gives us free
//! "fly to unit" transitions via [`CameraRig::focus_on`].
//!
//! | Action | Desktop | Touch |
//! |---|---|---|
//! | Pan    | left- or middle-drag the ground (it stays under the cursor), WASD / arrows | one-finger drag (the ground stays under the finger; flick to glide) |
//! | Rotate / tilt | right-drag (horizontal / vertical), Q / E (45° steps) | two-finger twist / two fingers dragged up or down |
//! | Zoom   | mouse wheel (towards the cursor), Z / X | pinch (towards the fingers) |
//! | Reset view | Home (default tilt and zoom) | |
//! | Move a soldier | drag from the selected soldier to a tile and release | Move button, tap a tile, confirm |
//! | Cancel aiming | right-click (without dragging) | |
//!
//! ## Smoothness
//! * The focus point moves on a critically damped spring ([`smooth_damp`]):
//!   long glides ease in *and* out instead of starting with a jolt.
//! * Gameplay uses [`CameraRig::follow`], which only moves the camera when
//!   the point of interest leaves a dead zone around the screen centre, so
//!   the view doesn't twitch for every small step or shot.
//! * Actions may [`CameraRig::frame`] two points (zooming out to fit both);
//!   the player's own zoom ([`CameraRig::user_distance`]) is restored once
//!   things calm down ([`CameraRig::restore_zoom`]).
//!
//! A press only counts as a **click** if the mouse didn't move more than
//! [`CLICK_SLOP`] pixels; [`MouseDrag`] tells the input code which happened.
//!
//! Camera *scripts* (cinematics) drive the same rig through
//! [`CameraDirector`](crate::camera_director::CameraDirector); player input
//! is ignored while one runs.

use std::f32::consts::{FRAC_PI_4, TAU};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::grid::GridPos;

use crate::FrameSet;
use crate::coords::grid_to_world;
use crate::camera_director::CameraDirector;
use crate::input::pointer_over_ui;
use crate::state::BattleState;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchGesture>()
            .init_resource::<MouseDrag>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (mouse_controls, keyboard_mouse_controls, touch_controls, apply_rig)
                    .chain()
                    .in_set(FrameSet::Input)
                    .in_set(CameraSystems),
            );
    }
}

/// Camera update systems. Input systems that raycast from the camera run after
/// this set so they use the transform of the current frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CameraSystems;

/// Orbit parameters. Fields prefixed `target_` are where the rig is heading.
#[derive(Component, Debug, Clone)]
pub struct CameraRig {
    pub focus: Vec3,
    pub yaw: f32,
    pub distance: f32,
    pub target_focus: Vec3,
    pub target_yaw: f32,
    pub target_distance: f32,
    /// Angle above the horizon, radians.
    pub pitch: f32,
    pub target_pitch: f32,
    /// Current shake strength (set by camera scripts, 0 = none).
    pub shake: f32,
    /// Allowed focus area (map bounds), so the camera can't get lost.
    pub bounds: Rect,
    /// Velocity of the focus spring.
    pub focus_velocity: Vec3,
    /// Zoom the player chose (actions may zoom out temporarily).
    pub user_distance: f32,
}

impl CameraRig {
    pub const MIN_DISTANCE: f32 = 3.0;
    pub const MAX_DISTANCE: f32 = 40.0;
    /// Higher = snappier (yaw, pitch, zoom).
    pub const SMOOTHING: f32 = 7.0;
    /// Seconds the focus spring takes to (nearly) arrive.
    pub const FOCUS_TIME: f32 = 0.22;
    /// Fraction of the camera distance the point of interest may stray from
    /// the focus before [`CameraRig::follow`] moves the camera.
    pub const DEAD_ZONE: f32 = 0.2;

    /// Smoothly moves the camera to look at a tile (at ground level 0; use
    /// [`CameraRig::focus_at`] with `coords::tile_top` to respect height).
    pub fn focus_on(&mut self, pos: GridPos) {
        self.target_focus = grid_to_world(pos);
    }

    /// Smoothly moves the camera to look at a world point.
    pub fn focus_at(&mut self, point: Vec3) {
        self.target_focus = point;
    }

    /// Keeps `point` in view without fussing: moves only if it's outside the
    /// dead zone around the current focus (or at a different height).
    pub fn follow(&mut self, point: Vec3) {
        let off = Vec2::new(point.x - self.target_focus.x, point.z - self.target_focus.z);
        if off.length() > self.target_distance * Self::DEAD_ZONE || (point.y - self.target_focus.y).abs() > 0.6 {
            self.target_focus = point;
            self.clamp_focus();
        }
    }

    /// For moving subjects (a walking soldier): drags the camera along just
    /// enough to keep `point` inside the dead zone, so it glides with the
    /// walker instead of jumping each time it reaches the edge.
    pub fn track(&mut self, point: Vec3) {
        let dz = self.target_distance * Self::DEAD_ZONE * 0.5;
        let off = Vec2::new(point.x - self.target_focus.x, point.z - self.target_focus.z);
        if off.length() > dz {
            let push = off.normalize() * (off.length() - dz);
            self.target_focus.x += push.x;
            self.target_focus.z += push.y;
        }
        self.target_focus.y += (point.y - self.target_focus.y) * 0.1;
        self.clamp_focus();
    }

    /// Shows both `a` and `b`: focus between them, zooming out (never in) so
    /// both fit. Undo the zoom with [`CameraRig::restore_zoom`].
    pub fn frame(&mut self, a: Vec3, b: Vec3) {
        self.target_focus = (a + b) / 2.0;
        self.clamp_focus();
        let needed = a.distance(b) * 1.1 + 7.0;
        self.target_distance = needed.min(Self::MAX_DISTANCE * 0.8).max(self.user_distance);
    }

    /// Back to the player's own zoom.
    pub fn restore_zoom(&mut self) {
        self.target_distance = self.user_distance;
    }

    /// Camera-relative "right" and "forward" on the ground plane.
    fn ground_axes(&self) -> (Vec3, Vec3) {
        let forward = Vec3::new(-self.target_yaw.sin(), 0.0, -self.target_yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        (right, forward)
    }

    /// Pans by a camera-relative offset (x = right, y = forward), in world units.
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan(delta);
    }

    fn pan(&mut self, screen_delta: Vec2) {
        let (right, forward) = self.ground_axes();
        self.target_focus += right * screen_delta.x + forward * screen_delta.y;
        self.clamp_focus();
    }

    /// Keeps the focus point over the map.
    fn clamp_focus(&mut self) {
        self.target_focus.x = self.target_focus.x.clamp(self.bounds.min.x, self.bounds.max.x);
        self.target_focus.z = self.target_focus.z.clamp(self.bounds.min.y, self.bounds.max.y);
        self.focus.x = self.focus.x.clamp(self.bounds.min.x, self.bounds.max.x);
        self.focus.z = self.focus.z.clamp(self.bounds.min.y, self.bounds.max.y);
    }

    fn zoom(&mut self, factor: f32) {
        self.target_distance =
            (self.target_distance * factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
        self.user_distance = self.target_distance;
    }
}

/// Pixels the mouse may move between press and release and still be a click.
pub const CLICK_SLOP: f32 = 6.0;

/// What the current mouse drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragMode {
    #[default]
    None,
    /// Grab-and-drag the ground.
    Pan,
    /// Rotate (horizontal) and tilt (vertical).
    Orbit,
    /// Started on the selected soldier: release on a tile to move/attack there.
    Order,
}

/// Mouse press/drag state, shared with the input code.
#[derive(Resource, Debug, Default)]
pub struct MouseDrag {
    button: Option<MouseButton>,
    start: Vec2,
    /// The press moved beyond [`CLICK_SLOP`].
    pub dragged: bool,
    /// What this press will do once it becomes a drag.
    candidate: DragMode,
    /// What the drag is doing now.
    pub mode: DragMode,
    /// Ground point grabbed when panning.
    grab: Option<Vec3>,
    /// Pan momentum (world units / second), applied after release.
    velocity: Vec3,
    /// Set for one frame: a left click (press + release without dragging).
    pub clicked: bool,
    /// Set for one frame: an order-drag was released.
    pub order_released: bool,
    /// Set for one frame: a right click (press + release without dragging).
    pub right_clicked: bool,
}

/// Where a camera ray through `cursor` hits the horizontal plane at `height`.
fn ground_point(camera: &Camera, cam_tr: &GlobalTransform, cursor: Vec2, height: f32) -> Option<Vec3> {
    let ray = camera.viewport_to_world(cam_tr, cursor).ok()?;
    ray.plane_intersection_point(Vec3::Y * height, InfinitePlane3d::new(Vec3::Y))
}

/// Mouse: click vs. drag, ground-grab panning with momentum, orbit/tilt,
/// order-drags from the selected soldier, zoom towards the cursor.
#[allow(clippy::too_many_arguments)]
fn mouse_controls(
    time: Res<Time<Real>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Single<&Window>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    director: Res<CameraDirector>,
    selection: Res<crate::state::Selection>,
    state: Res<crate::state::BattleState>,
    camera: Single<(&Camera, &GlobalTransform), With<CameraRig>>,
    settings: Res<crate::settings::GameSettings>,
    mut drag: ResMut<MouseDrag>,
    mut rig: Single<&mut CameraRig>,
) {
    let dt = time.delta_secs();
    drag.clicked = false;
    drag.order_released = false;
    drag.right_clicked = false;
    let cursor = window.cursor_position();
    let over_ui = pointer_over_ui(&hover_map, &ui_nodes);
    let (cam, cam_tr) = *camera;
    let plane = rig.focus.y;

    if director.is_active() {
        *drag = MouseDrag::default();
        return;
    }

    // --- press ----------------------------------------------------------
    for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
        if buttons.just_pressed(button) && drag.button.is_none() && !over_ui {
            let on_selected = selection.unit.and_then(|u| state.battle.unit(u)).is_some_and(|u| {
                Some(u.pos) == selection.hovered && u.can_act() && state.battle.active_team() == u.team
            });
            *drag = MouseDrag {
                button: Some(button),
                start: cursor.unwrap_or_default(),
                candidate: match button {
                    MouseButton::Right => DragMode::Orbit,
                    MouseButton::Left if on_selected => DragMode::Order,
                    _ => DragMode::Pan,
                },
                grab: cursor.and_then(|c| ground_point(cam, cam_tr, c, plane)),
                ..default()
            };
        }
    }

    // --- hold / drag ----------------------------------------------------
    if let (Some(button), Some(c)) = (drag.button, cursor) {
        if buttons.pressed(button) {
            if !drag.dragged && c.distance(drag.start) > CLICK_SLOP {
                drag.dragged = true;
                drag.mode = drag.candidate;
            }
            match drag.mode {
                DragMode::Pan => {
                    // Keep the grabbed ground point under the cursor.
                    if let (Some(grab), Some(now)) = (drag.grab, ground_point(cam, cam_tr, c, plane)) {
                        let delta = Vec3::new(grab.x - now.x, 0.0, grab.z - now.z);
                        let before = rig.target_focus;
                        rig.focus += delta;
                        rig.target_focus = rig.focus;
                        rig.focus_velocity = Vec3::ZERO;
                        rig.clamp_focus();
                        let moved = rig.target_focus - before;
                        if dt > 0.0 {
                            drag.velocity = drag.velocity.lerp(moved / dt, 0.5);
                        }
                    }
                }
                DragMode::Orbit => {
                    let tilt = if settings.invert_orbit { -1.0 } else { 1.0 };
                    rig.target_yaw -= motion.delta.x * 0.005;
                    rig.target_pitch = (rig.target_pitch + tilt * motion.delta.y * 0.004).clamp(0.3, 1.45);
                }
                DragMode::Order | DragMode::None => {}
            }
        }
        // --- release ----------------------------------------------------
        if buttons.just_released(button) {
            if !drag.dragged && button == MouseButton::Left && !over_ui {
                drag.clicked = true;
            }
            if !drag.dragged && button == MouseButton::Right && !over_ui {
                drag.right_clicked = true;
            }
            if drag.dragged && drag.mode == DragMode::Order && !over_ui {
                drag.order_released = true;
            }
            if drag.mode != DragMode::Pan {
                drag.velocity = Vec3::ZERO;
            }
            drag.button = None;
            drag.mode = DragMode::None;
            drag.dragged = false;
        }
    }

    // --- momentum after a pan ---------------------------------------------
    if drag.mode != DragMode::Pan && drag.velocity.length_squared() > 0.01 {
        let v = drag.velocity;
        rig.target_focus += v * dt;
        rig.clamp_focus();
        drag.velocity *= (-5.0 * dt).exp();
    } else if drag.mode != DragMode::Pan {
        drag.velocity = Vec3::ZERO;
    }

    // --- wheel: zoom towards the cursor -------------------------------------
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if lines != 0.0 && !over_ui {
        let old = rig.target_distance;
        rig.zoom(1.0 - lines * 0.12);
        let shrink = 1.0 - rig.target_distance / old; // > 0 when zooming in
        if let Some(p) = cursor.and_then(|c| ground_point(cam, cam_tr, c, plane)) {
            let toward = Vec3::new(p.x - rig.target_focus.x, 0.0, p.z - rig.target_focus.z);
            rig.target_focus += toward * shrink;
            rig.clamp_focus();
        }
    }
}

/// Tracks whether the current touch/mouse press turned into a drag, so a
/// release after panning isn't mistaken for a tap on a tile.
#[derive(Resource, Default)]
pub struct TouchGesture {
    pub dragged: bool,
    /// The gesture started on a UI panel (it scrolls the UI, not the camera).
    pub over_ui: bool,
    /// Ground point held under the finger while panning.
    grab: Option<Vec3>,
    /// Glide after a flick (world units per second).
    velocity: Vec3,
    /// Fingers down last frame (re-grab when going from two to one).
    fingers: usize,
}

/// Pixels a pointer must travel before a press counts as a drag.
pub const DRAG_THRESHOLD: f32 = 12.0;

fn spawn_camera(mut commands: Commands, state: Res<BattleState>) {
    let map = state.battle.map();
    let center = grid_to_world(GridPos::new(map.width() / 2, map.height() / 4));
    let far = grid_to_world(GridPos::new(map.width() - 1, map.height() - 1));
    let rig = CameraRig {
        focus: center,
        yaw: 0.0,
        distance: 18.0,
        target_focus: center,
        target_yaw: 0.0,
        target_distance: 18.0,
        pitch: 0.95,
        target_pitch: 0.95,
        shake: 0.0,
        focus_velocity: Vec3::ZERO,
        user_distance: 18.0,
        // grid y maps to -Z, so the far corner has the most negative z.
        bounds: Rect::from_corners(Vec2::new(0.0, 0.0), Vec2::new(far.x, far.z)),
    };
    commands.spawn((Camera3d::default(), rig_transform(&rig), rig));
}

/// Converts orbit parameters into a camera transform.
fn rig_transform(rig: &CameraRig) -> Transform {
    rig_transform_at(rig, rig.focus)
}

fn rig_transform_at(rig: &CameraRig, focus: Vec3) -> Transform {
    let offset = Vec3::new(
        rig.yaw.sin() * rig.pitch.cos(),
        rig.pitch.sin(),
        rig.yaw.cos() * rig.pitch.cos(),
    ) * rig.distance;
    Transform::from_translation(focus + offset).looking_at(focus, Vec3::Y)
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their inputs as parameters.
fn keyboard_mouse_controls(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    director: Res<CameraDirector>,
    settings: Res<crate::settings::GameSettings>,
    mut rig: Single<&mut CameraRig>,
) {
    if director.is_active() {
        return; // a camera script owns the camera
    }
    let dt = time.delta_secs() * settings.camera_speed;
    let speed = rig.target_distance * 1.2 * dt;

    let mut pan = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        pan.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        pan.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        pan.x += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        pan.x -= 1.0;
    }
    if pan != Vec2::ZERO {
        rig.pan(pan.normalize() * speed);
    }

    // 45° rotation steps, the classic tactics-camera feel.
    if keys.just_pressed(KeyCode::KeyQ) {
        rig.target_yaw -= FRAC_PI_4;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        rig.target_yaw += FRAC_PI_4;
    }

    // Mouse drags and the wheel are handled by `mouse_controls`.
    if keys.pressed(KeyCode::KeyZ) {
        rig.zoom(1.0 - dt);
    }
    if keys.pressed(KeyCode::KeyX) {
        rig.zoom(1.0 + dt);
    }
    // Home: default tilt and zoom, rotation snapped to the nearest 45°.
    if keys.just_pressed(KeyCode::Home) {
        rig.target_pitch = 0.95;
        rig.target_distance = 18.0;
        rig.user_distance = 18.0;
        rig.target_yaw = (rig.target_yaw / FRAC_PI_4).round() * FRAC_PI_4;
    }
}

#[allow(clippy::too_many_arguments)]
fn touch_controls(
    time: Res<Time<Real>>,
    touches: Res<Touches>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    settings: Res<crate::settings::GameSettings>,
    mut gesture: ResMut<TouchGesture>,
    director: Res<CameraDirector>,
    camera: Single<(&Camera, &GlobalTransform), With<CameraRig>>,
    mut rig: Single<&mut CameraRig>,
) {
    if director.is_active() {
        gesture.velocity = Vec3::ZERO;
        return;
    }
    let dt = time.delta_secs();
    let (cam, cam_tr) = *camera;
    let plane = rig.focus.y;
    let active: Vec<_> = touches.iter().collect();
    if touches.any_just_pressed() && active.len() == 1 {
        gesture.dragged = false;
        gesture.over_ui = pointer_over_ui(&hover_map, &ui_nodes);
        gesture.velocity = Vec3::ZERO;
        gesture.grab = ground_point(cam, cam_tr, active[0].position(), plane);
    }
    if gesture.over_ui {
        gesture.fingers = active.len();
        return;
    }
    match active.as_slice() {
        [one] => {
            // Coming back from two fingers: hold the ground where this finger is now.
            if gesture.fingers != 1 {
                gesture.grab = ground_point(cam, cam_tr, one.position(), plane);
            }
            if one.distance().length() > DRAG_THRESHOLD {
                gesture.dragged = true;
            }
            if gesture.dragged
                && let (Some(grab), Some(now)) = (gesture.grab, ground_point(cam, cam_tr, one.position(), plane))
            {
                // The grabbed ground point stays under the finger.
                let before = rig.target_focus;
                rig.focus += Vec3::new(grab.x - now.x, 0.0, grab.z - now.z);
                rig.target_focus = rig.focus;
                rig.focus_velocity = Vec3::ZERO;
                rig.clamp_focus();
                let moved = rig.target_focus - before;
                if dt > 0.0 {
                    gesture.velocity = gesture.velocity.lerp(moved / dt, 0.5);
                }
            }
        }
        [a, b, ..] => {
            gesture.dragged = true;
            gesture.velocity = Vec3::ZERO;
            let now = a.position() - b.position();
            let before = a.previous_position() - b.previous_position();
            let mid = (a.position() + b.position()) / 2.0;
            let (da, db) = (a.delta(), b.delta());
            // Both fingers sliding the same way vertically (and not apart):
            // tilt the camera.
            let parallel = da.y * db.y > 0.0 && da.y.abs() > da.x.abs() && db.y.abs() > db.x.abs();
            if parallel && (now.length() - before.length()).abs() < 4.0 {
                let tilt = if settings.invert_orbit { -1.0 } else { 1.0 };
                let dy = (da.y + db.y) / 2.0;
                rig.target_pitch = (rig.target_pitch + tilt * dy * 0.005).clamp(0.3, 1.45);
            } else if before.length() > 1.0 && now.length() > 1.0 {
                // Pinch: zoom towards the point between the fingers.
                let old = rig.target_distance;
                rig.zoom(before.length() / now.length());
                let shrink = 1.0 - rig.target_distance / old;
                if let Some(p) = ground_point(cam, cam_tr, mid, plane) {
                    let toward = Vec3::new(p.x - rig.target_focus.x, 0.0, p.z - rig.target_focus.z);
                    rig.target_focus += toward * shrink;
                    rig.clamp_focus();
                }
                // Twist: rotate with the fingers.
                rig.target_yaw -= before.angle_to(now);
            }
        }
        [] => {
            // Glide after a flick, slowing down.
            if gesture.velocity.length_squared() > 0.01 {
                let v = gesture.velocity;
                rig.target_focus += v * dt;
                rig.clamp_focus();
                gesture.velocity *= (-5.0 * dt).exp();
            } else {
                gesture.velocity = Vec3::ZERO;
            }
        }
    }
    gesture.fingers = active.len();
}

/// Eases the rig towards its targets and writes the camera transform.
/// Uses real time so slow motion (camera scripts) doesn't make the camera sluggish.
fn apply_rig(
    time: Res<Time<Real>>,
    settings: Res<crate::settings::GameSettings>,
    mut q: Single<(&mut CameraRig, &mut Transform)>,
) {
    let (rig, transform) = &mut *q;
    // Faster camera setting = shorter glides.
    let dt = time.delta_secs() * settings.camera_speed.max(0.1);
    let t = 1.0 - (-CameraRig::SMOOTHING * dt).exp();
    let mut velocity = rig.focus_velocity;
    rig.focus = smooth_damp(rig.focus, rig.target_focus, &mut velocity, CameraRig::FOCUS_TIME, dt);
    rig.focus_velocity = velocity;
    rig.distance += (rig.target_distance - rig.distance) * t;
    rig.yaw += (rig.target_yaw - rig.yaw) * t;
    rig.pitch += (rig.target_pitch - rig.pitch) * t;
    // Keep angles bounded without visible jumps.
    if rig.yaw.abs() > TAU * 4.0 {
        let wrap = (rig.yaw / TAU).trunc() * TAU;
        rig.yaw -= wrap;
        rig.target_yaw -= wrap;
    }
    // Shake: jitter the look-at point (deterministic noise from time).
    let mut focus = rig.focus;
    if rig.shake > 0.0 {
        let e = time.elapsed_secs();
        focus += Vec3::new((e * 53.0).sin(), (e * 37.0 + 1.0).sin() * 0.5, (e * 61.0 + 2.0).sin()) * rig.shake * 0.3;
        // Fades out unless a camera script keeps setting it.
        rig.shake = (rig.shake - time.delta_secs() * 0.8).max(0.0);
    }
    **transform = rig_transform_at(rig, focus);
}

/// Critically damped spring towards `target` (the classic "SmoothDamp"):
/// eases in and out, never overshoots noticeably, frame-rate independent.
pub fn smooth_damp(current: Vec3, target: Vec3, velocity: &mut Vec3, smooth_time: f32, dt: f32) -> Vec3 {
    if dt <= 0.0 {
        return current;
    }
    let omega = 2.0 / smooth_time.max(1e-4);
    let x = omega * dt;
    let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = current - target;
    let temp = (*velocity + change * omega) * dt;
    *velocity = (*velocity - temp * omega) * decay;
    let out = target + (change + temp) * decay;
    if (target - current).dot(out - target) > 0.0 {
        // Overshot: settle exactly.
        *velocity = Vec3::ZERO;
        return target;
    }
    out
}

#[cfg(test)]
mod smoothing_tests {
    use super::*;

    #[test]
    fn spring_eases_in_and_arrives() {
        let (mut p, mut v) = (Vec3::ZERO, Vec3::ZERO);
        let target = Vec3::new(10.0, 0.0, 0.0);
        let mut steps = Vec::new();
        for _ in 0..120 {
            let before = p;
            p = smooth_damp(p, target, &mut v, 0.22, 1.0 / 60.0);
            steps.push((p - before).x);
        }
        assert!(steps[0] < steps[5], "eases in: starts slower than it continues");
        assert!((p - target).length() < 0.01, "arrives: {p}");
        assert!(steps.iter().all(|s| *s >= 0.0), "never goes backwards");
    }

    #[test]
    fn follow_ignores_small_moves_and_frame_restores_zoom() {
        let mut rig = CameraRig {
            focus: Vec3::ZERO,
            yaw: 0.0,
            distance: 18.0,
            target_focus: Vec3::ZERO,
            target_yaw: 0.0,
            target_distance: 18.0,
            pitch: 0.95,
            target_pitch: 0.95,
            shake: 0.0,
            bounds: Rect::from_corners(Vec2::new(-50.0, -50.0), Vec2::new(50.0, 50.0)),
            focus_velocity: Vec3::ZERO,
            user_distance: 18.0,
        };
        rig.follow(Vec3::new(2.0, 0.0, 1.0));
        assert_eq!(rig.target_focus, Vec3::ZERO, "inside the dead zone");
        rig.follow(Vec3::new(9.0, 0.0, 0.0));
        assert_eq!(rig.target_focus, Vec3::new(9.0, 0.0, 0.0));
        rig.frame(Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0));
        assert!(rig.target_distance > 18.0);
        rig.restore_zoom();
        assert_eq!(rig.target_distance, 18.0);
    }
}

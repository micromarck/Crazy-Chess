//! Tactical orbit camera.
//!
//! The camera orbits a *focus point* on the ground at a fixed pitch, like most
//! squad-tactics games. All motion is smoothed: input changes the *target*
//! values and the rig eases towards them every frame, which also gives us free
//! "fly to unit" transitions via [`CameraRig::focus_on`].
//!
//! | Action | Desktop | Touch |
//! |---|---|---|
//! | Pan    | left- or middle-drag the ground (it stays under the cursor), WASD / arrows | one-finger drag |
//! | Rotate / tilt | right-drag (horizontal / vertical), Q / E (45° steps) | two-finger twist |
//! | Zoom   | mouse wheel (towards the cursor), Z / X | pinch |
//! | Move a soldier | drag from the selected soldier to a tile and release | tap |
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
}

impl CameraRig {
    pub const MIN_DISTANCE: f32 = 6.0;
    pub const MAX_DISTANCE: f32 = 40.0;
    /// Higher = snappier.
    pub const SMOOTHING: f32 = 8.0;

    /// Smoothly moves the camera to look at a tile (at ground level 0; use
    /// [`CameraRig::focus_at`] with `coords::tile_top` to respect height).
    pub fn focus_on(&mut self, pos: GridPos) {
        self.target_focus = grid_to_world(pos);
    }

    /// Smoothly moves the camera to look at a world point.
    pub fn focus_at(&mut self, point: Vec3) {
        self.target_focus = point;
    }

    /// Camera-relative "right" and "forward" on the ground plane.
    fn ground_axes(&self) -> (Vec3, Vec3) {
        let forward = Vec3::new(-self.target_yaw.sin(), 0.0, -self.target_yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        (right, forward)
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
    mut drag: ResMut<MouseDrag>,
    mut rig: Single<&mut CameraRig>,
) {
    let dt = time.delta_secs();
    drag.clicked = false;
    drag.order_released = false;
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
                        rig.clamp_focus();
                        let moved = rig.target_focus - before;
                        if dt > 0.0 {
                            drag.velocity = drag.velocity.lerp(moved / dt, 0.5);
                        }
                    }
                }
                DragMode::Orbit => {
                    rig.target_yaw -= motion.delta.x * 0.005;
                    rig.target_pitch = (rig.target_pitch + motion.delta.y * 0.004).clamp(0.3, 1.45);
                }
                DragMode::Order | DragMode::None => {}
            }
        }
        // --- release ----------------------------------------------------
        if buttons.just_released(button) {
            if !drag.dragged && button == MouseButton::Left && !over_ui {
                drag.clicked = true;
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
    mut rig: Single<&mut CameraRig>,
) {
    if director.is_active() {
        return; // a camera script owns the camera
    }
    let dt = time.delta_secs();
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
}

fn touch_controls(
    touches: Res<Touches>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut gesture: ResMut<TouchGesture>,
    director: Res<CameraDirector>,
    mut rig: Single<&mut CameraRig>,
) {
    if director.is_active() {
        return;
    }
    if touches.any_just_pressed() && touches.iter().count() == 1 {
        gesture.dragged = false;
        gesture.over_ui = pointer_over_ui(&hover_map, &ui_nodes);
    }
    if gesture.over_ui {
        return;
    }
    let active: Vec<_> = touches.iter().collect();
    match active.as_slice() {
        [one] => {
            if one.distance().length() > DRAG_THRESHOLD {
                gesture.dragged = true;
            }
            if gesture.dragged {
                let k = rig.target_distance * 0.003;
                let d = one.delta();
                rig.pan(Vec2::new(-d.x, d.y) * k);
            }
        }
        [a, b, ..] => {
            gesture.dragged = true;
            // Pinch: ratio of finger distances this frame vs last frame.
            let now = a.position() - b.position();
            let before = a.previous_position() - b.previous_position();
            if before.length() > 1.0 && now.length() > 1.0 {
                rig.zoom(before.length() / now.length());
                // Twist: change of angle between the fingers.
                rig.target_yaw -= before.angle_to(now);
            }
        }
        [] => {}
    }
}

/// Eases the rig towards its targets and writes the camera transform.
/// Uses real time so slow motion (camera scripts) doesn't make the camera sluggish.
fn apply_rig(time: Res<Time<Real>>, mut q: Single<(&mut CameraRig, &mut Transform)>) {
    let (rig, transform) = &mut *q;
    let t = 1.0 - (-CameraRig::SMOOTHING * time.delta_secs()).exp();
    rig.focus = rig.focus.lerp(rig.target_focus, t);
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
    }
    **transform = rig_transform_at(rig, focus);
}

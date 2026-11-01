//! Tactical orbit camera.
//!
//! The camera orbits a *focus point* on the ground at a fixed pitch, like most
//! squad-tactics games. All motion is smoothed: input changes the *target*
//! values and the rig eases towards them every frame, which also gives us free
//! "fly to unit" transitions via [`CameraRig::focus_on`].
//!
//! | Action | Desktop | Touch |
//! |---|---|---|
//! | Pan    | WASD / arrows, middle-drag | one-finger drag |
//! | Rotate | Q / E (45° steps), right-drag | two-finger twist |
//! | Zoom   | mouse wheel, Z / X | pinch |

use std::f32::consts::{FRAC_PI_4, TAU};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::grid::GridPos;

use crate::FrameSet;
use crate::coords::grid_to_world;
use crate::input::pointer_over_ui;
use crate::state::BattleState;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchGesture>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (keyboard_mouse_controls, touch_controls, apply_rig)
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
    /// Allowed focus area (map bounds), so the camera can't get lost.
    pub bounds: Rect,
}

impl CameraRig {
    pub const MIN_DISTANCE: f32 = 6.0;
    pub const MAX_DISTANCE: f32 = 40.0;
    /// Higher = snappier.
    pub const SMOOTHING: f32 = 8.0;

    /// Smoothly moves the camera to look at a tile.
    pub fn focus_on(&mut self, pos: GridPos) {
        self.target_focus = grid_to_world(pos);
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
        self.target_focus.x = self.target_focus.x.clamp(self.bounds.min.x, self.bounds.max.x);
        self.target_focus.z = self.target_focus.z.clamp(self.bounds.min.y, self.bounds.max.y);
    }

    fn zoom(&mut self, factor: f32) {
        self.target_distance =
            (self.target_distance * factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
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
        // grid y maps to -Z, so the far corner has the most negative z.
        bounds: Rect::from_corners(Vec2::new(0.0, 0.0), Vec2::new(far.x, far.z)),
    };
    commands.spawn((Camera3d::default(), rig_transform(&rig), rig));
}

/// Converts orbit parameters into a camera transform.
fn rig_transform(rig: &CameraRig) -> Transform {
    let offset = Vec3::new(
        rig.yaw.sin() * rig.pitch.cos(),
        rig.pitch.sin(),
        rig.yaw.cos() * rig.pitch.cos(),
    ) * rig.distance;
    Transform::from_translation(rig.focus + offset).looking_at(rig.focus, Vec3::Y)
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their inputs as parameters.
fn keyboard_mouse_controls(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut rig: Single<&mut CameraRig>,
) {
    let dt = time.delta_secs();
    let over_ui = pointer_over_ui(&hover_map, &ui_nodes);
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

    // Free rotation / pan with mouse drags.
    if buttons.pressed(MouseButton::Right) {
        rig.target_yaw -= motion.delta.x * 0.006;
    }
    if buttons.pressed(MouseButton::Middle) {
        let k = rig.target_distance * 0.0025;
        rig.pan(Vec2::new(-motion.delta.x, motion.delta.y) * k);
    }

    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    // The wheel scrolls UI panels (e.g. the editor) when the pointer is over them.
    if lines != 0.0 && !over_ui {
        rig.zoom(1.0 - lines * 0.1);
    }
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
    mut rig: Single<&mut CameraRig>,
) {
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
fn apply_rig(time: Res<Time>, mut q: Single<(&mut CameraRig, &mut Transform)>) {
    let (rig, transform) = &mut *q;
    let t = 1.0 - (-CameraRig::SMOOTHING * time.delta_secs()).exp();
    rig.focus = rig.focus.lerp(rig.target_focus, t);
    rig.distance += (rig.target_distance - rig.distance) * t;
    rig.yaw += (rig.target_yaw - rig.yaw) * t;
    // Keep angles bounded without visible jumps.
    if rig.yaw.abs() > TAU * 4.0 {
        let wrap = (rig.yaw / TAU).trunc() * TAU;
        rig.yaw -= wrap;
        rig.target_yaw -= wrap;
    }
    **transform = rig_transform(rig);
}

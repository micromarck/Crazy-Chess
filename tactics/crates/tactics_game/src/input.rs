//! Player input: tile hovering, clicks/taps on the board, keyboard shortcuts.
//!
//! The pointer (mouse cursor or a finger) is turned into a ray from the camera,
//! intersected with the ground plane, and converted into a [`GridPos`]. A
//! click/tap then means, in priority order:
//! 1. on a friendly unit → select it;
//! 2. on a visible enemy → shoot it with the selected unit;
//! 3. on a reachable tile → move there.
//!
//! Everything is translated into core [`Action`]s or [`PlayerCommand`]s; this
//! module never touches unit data directly.

use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::{CameraRig, CameraSystems, TouchGesture};
use crate::coords::world_to_grid;
use crate::playback::Playback;
use crate::state::{BattleState, PlayerCommand, Selection};

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                update_hover,
                keyboard_shortcuts,
                (board_click, handle_commands)
                    .chain()
                    .run_if(player_can_act),
                auto_select,
            )
                .chain()
                .after(CameraSystems)
                .in_set(FrameSet::Input),
        );
    }
}

/// Run condition: it's the player's turn and nothing is animating.
pub fn player_can_act(state: Res<BattleState>, playback: Res<Playback>) -> bool {
    !playback.is_busy() && !state.battle.is_over() && state.battle.active_team() == Team::Player
}

/// Where the pointer is this frame, in window coordinates (mouse or first touch).
fn pointer_position(window: &Window, touches: &Touches) -> Option<Vec2> {
    window
        .cursor_position()
        .or_else(|| touches.first_pressed_position())
        .or_else(|| touches.iter_just_released().next().map(|t| t.position()))
}

/// Is the pointer over any UI node? Uses Bevy picking's hover map, which works
/// for mouse and touch alike.
fn pointer_over_ui(hover_map: &HoverMap, nodes: &Query<(), With<Node>>) -> bool {
    hover_map
        .values()
        .any(|hits| hits.keys().any(|e| nodes.contains(*e)))
}

fn update_hover(
    window: Single<&Window>,
    touches: Res<Touches>,
    camera: Single<(&Camera, &GlobalTransform), With<CameraRig>>,
    state: Res<BattleState>,
    mut selection: ResMut<Selection>,
) {
    let (camera, cam_transform) = *camera;
    selection.hovered = pointer_position(&window, &touches)
        .and_then(|p| camera.viewport_to_world(cam_transform, p).ok())
        .and_then(|ray| ray.plane_intersection_point(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)))
        .map(world_to_grid)
        .filter(|p| state.battle.map().in_bounds(*p));
}

fn keyboard_shortcuts(keys: Res<ButtonInput<KeyCode>>, mut commands: MessageWriter<PlayerCommand>) {
    let bindings = [
        (KeyCode::Tab, PlayerCommand::NextUnit),
        (KeyCode::KeyO, PlayerCommand::Overwatch),
        (KeyCode::KeyH, PlayerCommand::Hunker),
        (KeyCode::KeyR, PlayerCommand::Reload),
        (KeyCode::Enter, PlayerCommand::EndTurn),
        (KeyCode::F5, PlayerCommand::Restart),
    ];
    for (key, command) in bindings {
        if keys.just_pressed(key) {
            commands.write(command);
        }
    }
}

/// Resolves a left click / tap on the board.
#[allow(clippy::too_many_arguments)]
fn board_click(
    buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gesture: Res<TouchGesture>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut state: ResMut<BattleState>,
    mut selection: ResMut<Selection>,
    mut playback: ResMut<Playback>,
    mut rig: Single<&mut CameraRig>,
) {
    let tapped = touches.any_just_released() && !gesture.dragged;
    if !(buttons.just_pressed(MouseButton::Left) || tapped) {
        return;
    }
    if pointer_over_ui(&hover_map, &ui_nodes) {
        return;
    }
    let Some(tile) = selection.hovered else { return };
    let battle = &state.battle;

    // 1. Select a friendly unit.
    if let Some(unit) = battle.unit_at(tile)
        && unit.team == Team::Player {
            selection.unit = Some(unit.id);
            rig.focus_on(tile);
            return;
        }

    let Some(selected) = selection.unit else { return };

    // 2. Shoot an enemy.
    if let Some(target) = battle.unit_at(tile).filter(|u| u.team == Team::Enemy) {
        let action = Action::Shoot { unit: selected, target: target.id };
        let _ = state.apply(action, &mut playback);
        return;
    }

    // 3. Move.
    let action = Action::Move { unit: selected, to: tile };
    if battle.validate(&action).is_ok() {
        let _ = state.apply(action, &mut playback);
    }
}

/// Executes keyboard/HUD commands that act on the selected unit.
fn handle_commands(
    mut commands: MessageReader<PlayerCommand>,
    mut state: ResMut<BattleState>,
    mut selection: ResMut<Selection>,
    mut playback: ResMut<Playback>,
    mut rig: Single<&mut CameraRig>,
) {
    for command in commands.read() {
        let unit = selection.unit;
        let action = match (command, unit) {
            (PlayerCommand::NextUnit, _) => {
                selection.unit = next_ready_unit(&state.battle, unit);
                if let Some(u) = selection.unit.and_then(|id| state.battle.unit(id)) {
                    rig.focus_on(u.pos);
                }
                None
            }
            (PlayerCommand::Overwatch, Some(unit)) => Some(Action::Overwatch { unit }),
            (PlayerCommand::Hunker, Some(unit)) => Some(Action::Hunker { unit }),
            (PlayerCommand::Reload, Some(unit)) => Some(Action::Reload { unit }),
            (PlayerCommand::EndTurn, _) => Some(Action::EndTurn),
            // Restart is handled by the HUD (it works even when the battle is over).
            _ => None,
        };
        if let Some(action) = action {
            let _ = state.apply(action, &mut playback);
        }
    }
}

/// The next friendly unit (after `current`, wrapping) that still has AP.
pub fn next_ready_unit(battle: &Battle, current: Option<UnitId>) -> Option<UnitId> {
    let ready: Vec<UnitId> = battle
        .living(Team::Player)
        .filter(|u| u.can_act())
        .map(|u| u.id)
        .collect();
    if ready.is_empty() {
        return None;
    }
    let idx = current
        .and_then(|c| ready.iter().position(|id| *id > c))
        .unwrap_or(0);
    Some(ready[idx])
}

/// Keeps a sensible unit selected: if the current one is spent or dead, pick
/// the next ready one (only when it's the player's turn and nothing animates).
fn auto_select(
    state: Res<BattleState>,
    playback: Res<Playback>,
    mut selection: ResMut<Selection>,
    mut rig: Single<&mut CameraRig>,
    mut focused_round: Local<u32>,
) {
    if playback.is_busy() || state.battle.active_team() != Team::Player || state.battle.is_over() {
        return;
    }
    let valid = selection
        .unit
        .and_then(|id| state.battle.unit(id))
        .is_some_and(|u| u.can_act());
    // At the start of each player turn, bring the camera back to the squad.
    let new_turn = *focused_round != state.battle.round();
    *focused_round = state.battle.round();
    if !valid || new_turn {
        if !valid {
            selection.unit = next_ready_unit(&state.battle, selection.unit);
        }
        if let Some(u) = selection.unit.and_then(|id| state.battle.unit(id)) {
            rig.focus_on(u.pos);
        }
    }
    selection.refresh_range(&state);
}

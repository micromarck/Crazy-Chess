//! Player input: tile hovering, clicks/taps on the board, keyboard shortcuts,
//! and ability targeting.
//!
//! The pointer (mouse cursor or a finger) is turned into a ray from the camera,
//! intersected with the ground plane, and converted into a [`GridPos`].
//!
//! A click/tap means, in priority order:
//! 1. **targeting mode** (an ability is pending) → use it on the clicked
//!    unit/tile;
//! 2. on a friendly unit → select it;
//! 3. on an enemy → use the selected unit's primary attack;
//! 4. on a reachable tile → move there.
//!
//! Abilities are activated from HUD buttons or number keys. Self-targeted ones
//! (overwatch, reload...) fire immediately; the others enter targeting mode
//! (Esc or pressing the button again cancels).

use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::{CameraRig, CameraSystems, TouchGesture};
use crate::coords::world_to_grid;
use crate::hud::LogMessages;
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
                (board_click, handle_commands).chain().run_if(player_can_act),
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
pub fn pointer_over_ui(hover_map: &HoverMap, nodes: &Query<(), With<Node>>) -> bool {
    hover_map.values().any(|hits| hits.keys().any(|e| nodes.contains(*e)))
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
        (KeyCode::Escape, PlayerCommand::CancelTargeting),
        (KeyCode::Enter, PlayerCommand::EndTurn),
        (KeyCode::F5, PlayerCommand::Restart),
    ];
    for (key, command) in bindings {
        if keys.just_pressed(key) {
            commands.write(command);
        }
    }
    // Number keys 1-9 trigger the selected unit's abilities in order.
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (slot, key) in digits.into_iter().enumerate() {
        if keys.just_pressed(key) {
            commands.write(PlayerCommand::AbilitySlot(slot));
        }
    }
}

/// The target an ability would get if the player clicked `tile`.
pub fn target_for(battle: &Battle, def: &AbilityDef, tile: GridPos) -> Option<Target> {
    match def.targeting {
        Targeting::SelfOnly => Some(Target::None),
        Targeting::Enemy | Targeting::Ally => battle.unit_at(tile).map(|u| Target::Unit(u.id)),
        Targeting::Tile => Some(Target::Tile(tile)),
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
    mut log: ResMut<LogMessages>,
    mut rig: Single<&mut CameraRig>,
) {
    let tapped = touches.any_just_released() && !gesture.dragged;
    if !(buttons.just_pressed(MouseButton::Left) || tapped) || pointer_over_ui(&hover_map, &ui_nodes) {
        return;
    }
    let Some(tile) = selection.hovered else { return };
    let battle = &state.battle;

    // 1. Targeting mode.
    if let (Some(unit), Some(ability)) = (selection.unit, selection.pending.clone()) {
        let Some(def) = battle.ability(&ability) else { return };
        let Some(target) = target_for(battle, def, tile) else {
            log.push(format!("{}: pick a valid target (Esc to cancel)", def.name));
            return;
        };
        let action = Action::UseAbility { unit, ability, target };
        match state.apply(action, &mut playback) {
            Ok(()) => selection.pending = None,
            Err(e) => log.push(format!("Can't do that: {e}")),
        }
        return;
    }

    // 2. Select a friendly unit.
    if let Some(unit) = battle.unit_at(tile).filter(|u| u.team == Team::Player) {
        selection.select(Some(unit.id));
        rig.focus_on(tile);
        return;
    }

    let Some(selected) = selection.unit else { return };

    // 3. Primary attack on an enemy.
    if let Some(enemy) = battle.unit_at(tile).filter(|u| u.team == Team::Enemy) {
        let Some(primary) = battle.unit(selected).and_then(|u| battle.primary_attack(u)).map(|d| d.id.clone()) else {
            return;
        };
        let action = Action::UseAbility { unit: selected, ability: primary, target: Target::Unit(enemy.id) };
        if let Err(e) = state.apply(action, &mut playback) {
            log.push(format!("Can't shoot: {e}"));
        }
        return;
    }

    // 4. Move.
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
    mut log: ResMut<LogMessages>,
    mut rig: Single<&mut CameraRig>,
) {
    for command in commands.read() {
        let ability = match command {
            PlayerCommand::NextUnit => {
                let next = next_ready_unit(&state.battle, selection.unit);
                selection.select(next);
                if let Some(u) = next.and_then(|id| state.battle.unit(id)) {
                    rig.focus_on(u.pos);
                }
                continue;
            }
            PlayerCommand::CancelTargeting => {
                selection.pending = None;
                continue;
            }
            PlayerCommand::EndTurn => {
                selection.pending = None;
                let _ = state.apply(Action::EndTurn, &mut playback);
                continue;
            }
            // Restart is handled by the HUD (it works even when the battle is over).
            PlayerCommand::Restart => continue,
            PlayerCommand::UseAbility(id) => id.clone(),
            PlayerCommand::AbilitySlot(slot) => {
                let Some(id) = selection
                    .unit
                    .and_then(|u| state.battle.unit(u))
                    .and_then(|u| u.abilities.get(*slot).cloned())
                else {
                    continue;
                };
                id
            }
        };
        let Some(unit) = selection.unit else { continue };
        let Some(def) = state.battle.ability(&ability) else { continue };

        if def.targeting == Targeting::SelfOnly {
            selection.pending = None;
            let name = def.name.clone();
            if let Err(e) = state.apply(Action::UseAbility { unit, ability, target: Target::None }, &mut playback) {
                log.push(format!("Can't {name}: {e}"));
            }
        } else if selection.pending.as_ref() == Some(&ability) {
            selection.pending = None; // pressing it again cancels
        } else {
            // Check costs now so the player isn't left targeting something unusable.
            let probe = Action::UseAbility { unit, ability: ability.clone(), target: Target::None };
            match state.battle.validate(&probe) {
                Ok(())
                | Err(
                    ActionError::WrongTarget
                    | ActionError::OutOfRange
                    | ActionError::NoLineOfSight
                    | ActionError::InvalidTarget(_)
                    | ActionError::NothingToHeal,
                ) => {
                    log.push(format!("{}: choose a target", def.name));
                    selection.pending = Some(ability);
                }
                Err(e) => log.push(format!("Can't {}: {e}", def.name)),
            }
        }
    }
}

/// The next friendly unit (after `current`, wrapping) that still has AP.
pub fn next_ready_unit(battle: &Battle, current: Option<UnitId>) -> Option<UnitId> {
    let ready: Vec<UnitId> = battle.living(Team::Player).filter(|u| u.can_act()).map(|u| u.id).collect();
    if ready.is_empty() {
        return None;
    }
    let idx = current.and_then(|c| ready.iter().position(|id| *id > c)).unwrap_or(0);
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
    let valid = selection.unit.and_then(|id| state.battle.unit(id)).is_some_and(|u| u.can_act());
    // At the start of each player turn, bring the camera back to the squad.
    let new_turn = *focused_round != state.battle.round();
    *focused_round = state.battle.round();
    if !valid || new_turn {
        if !valid {
            let next = next_ready_unit(&state.battle, selection.unit);
            selection.select(next);
        }
        if let Some(u) = selection.unit.and_then(|id| state.battle.unit(id)) {
            rig.focus_on(u.pos);
        }
    }
    selection.refresh_range(&state);
}

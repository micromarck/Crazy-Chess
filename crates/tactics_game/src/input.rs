//! Player input: tile hovering, clicks/taps on the board, keyboard shortcuts,
//! and ability targeting.
//!
//! The pointer (mouse cursor or a finger) is turned into a ray from the camera,
//! intersected with the ground plane, and converted into a [`GridPos`].
//!
//! A click/tap (or releasing a drag that started on the selected soldier,
//! see [`MouseDrag`](crate::camera::MouseDrag)) means, in priority order:
//! 1. **targeting mode** (an ability is pending) → **aim** at the clicked
//!    unit/tile (the camera frames shooter and target, the odds show by the
//!    cursor); clicking the **same** target again fires;
//! 2. on a friendly unit → select it;
//! 3. on an enemy → aim the selected unit's primary attack at it (click
//!    again to fire);
//! 4. on a reachable tile → move there.
//!
//! Two steps for attacks means a slip of the mouse never wastes a turn.
//! Confirm with a second click/tap, **Space**, **Enter**, or the ability's
//! key/button again. Cancel with **right-click** or **Esc** (first the aim,
//! then targeting).
//!
//! Abilities are activated from HUD buttons or number keys. Self-targeted ones
//! (overwatch, reload...) fire immediately; the others enter targeting mode.
//!
//! | key | action |
//! |---|---|
//! | Tab / Shift+Tab | next / previous soldier |
//! | F | centre the camera on the selected soldier |
//! | 1-9 | abilities |
//! | Space | fire the aimed ability |
//! | Enter | fire if aiming, otherwise end the turn |
//! | Esc, right-click | cancel aim / targeting |
//! | hold Shift | fast-forward animations (x3) |
//! | F5 | restart the mission |

use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::{CameraRig, CameraSystems, TouchGesture};
use crate::camera_director::CameraDirector;
use crate::coords::pick_tile;
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

/// Run condition: it's the player's turn and nothing is animating (including
/// camera scripts).
pub fn player_can_act(state: Res<BattleState>, playback: Res<Playback>, director: Res<CameraDirector>) -> bool {
    !playback.is_busy() && !director.is_active() && !state.battle.is_over() && state.battle.active_team() == Team::Player
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
        .and_then(|ray| pick_tile(state.battle.map(), ray));
}

fn keyboard_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    drag: Res<crate::camera::MouseDrag>,
    director: Res<CameraDirector>,
    mut virt: ResMut<Time<Virtual>>,
    mut commands: MessageWriter<PlayerCommand>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let bindings = [
        (KeyCode::Tab, if shift { PlayerCommand::PreviousUnit } else { PlayerCommand::NextUnit }),
        (KeyCode::Escape, PlayerCommand::CancelTargeting),
        (KeyCode::Enter, PlayerCommand::ConfirmOrEndTurn),
        (KeyCode::NumpadEnter, PlayerCommand::ConfirmOrEndTurn),
        (KeyCode::Space, PlayerCommand::Confirm),
        (KeyCode::KeyF, PlayerCommand::FocusSelected),
        (KeyCode::F5, PlayerCommand::Restart),
    ];
    if drag.right_clicked {
        commands.write(PlayerCommand::CancelTargeting);
    }
    // Hold Shift to fast-forward (camera scripts control speed themselves).
    if !director.is_active() {
        let speed = if shift { 3.0 } else { 1.0 };
        if (virt.relative_speed() - speed).abs() > f32::EPSILON {
            virt.set_relative_speed(speed);
        }
    }
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
    drag: Res<crate::camera::MouseDrag>,
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
    // A click (press + release without dragging), a tap, or releasing an
    // order-drag that started on the selected soldier.
    let tapped = touches.any_just_released() && !gesture.dragged;
    if !(drag.clicked || drag.order_released || tapped) || pointer_over_ui(&hover_map, &ui_nodes) {
        return;
    }
    let Some(tile) = selection.hovered else { return };
    let battle = &state.battle;

    // 1. Targeting mode: first click aims, clicking the same target fires.
    if let (Some(unit), Some(ability)) = (selection.unit, selection.pending.clone()) {
        let Some(def) = battle.ability(&ability) else { return };
        let Some(target) = target_for(battle, def, tile) else {
            log.push(format!("{}: pick a valid target (right-click or Esc to cancel)", def.name));
            return;
        };
        if selection.aim == Some(target) {
            fire(&mut state, &mut selection, &mut playback, &mut log);
        } else {
            aim(&state, &mut selection, &mut log, &mut rig, unit, ability, target);
        }
        return;
    }

    // 2. Select a friendly unit (not when finishing an order-drag on one).
    if let Some(unit) = battle.unit_at(tile).filter(|u| u.team == Team::Player && !drag.order_released) {
        selection.select(Some(unit.id));
        rig.focus_at(crate::coords::tile_top(state.battle.map(), tile));
        return;
    }

    let Some(selected) = selection.unit else { return };

    // 3. Aim the primary attack at an enemy (click again to fire).
    if let Some(enemy) = battle.unit_at(tile).filter(|u| u.team == Team::Enemy && state.battle.is_revealed(u.id)) {
        let Some(primary) = battle.unit(selected).and_then(|u| battle.primary_attack(u)).map(|d| d.id.clone()) else {
            return;
        };
        let target = Target::Unit(enemy.id);
        aim(&state, &mut selection, &mut log, &mut rig, selected, primary, target);
        return;
    }

    // 4. Move.
    let action = Action::Move { unit: selected, to: tile };
    if battle.validate(&action).is_ok() {
        let _ = state.apply(action, &mut playback);
    }
}

/// Points `ability` at `target` (if legal) and frames the shot; the next
/// confirmation fires it.
fn aim(
    state: &BattleState,
    selection: &mut Selection,
    log: &mut LogMessages,
    rig: &mut CameraRig,
    unit: UnitId,
    ability: AbilityId,
    target: Target,
) {
    let battle = &state.battle;
    let name = battle.ability(&ability).map_or(String::new(), |d| d.name.clone());
    match battle.validate(&Action::UseAbility { unit, ability: ability.clone(), target }) {
        Ok(()) => {
            selection.pending = Some(ability);
            selection.aim = Some(target);
            let at = match target {
                Target::Unit(t) => battle.unit(t).map(|u| u.pos),
                Target::Tile(p) => Some(p),
                Target::None => None,
            };
            if let (Some(from), Some(to)) = (battle.unit(unit).map(|u| u.pos), at) {
                let map = battle.map();
                rig.frame(crate::coords::tile_top(map, from), crate::coords::tile_top(map, to));
            }
        }
        Err(e) => log.push(format!("Can't {name}: {e}")),
    }
}

/// Fires the aimed ability.
fn fire(state: &mut BattleState, selection: &mut Selection, playback: &mut Playback, log: &mut LogMessages) {
    let (Some(unit), Some(ability), Some(target)) = (selection.unit, selection.pending.clone(), selection.aim) else {
        return;
    };
    match state.apply(Action::UseAbility { unit, ability, target }, playback) {
        Ok(()) => {
            selection.pending = None;
            selection.aim = None;
        }
        Err(e) => {
            log.push(format!("Can't do that: {e}"));
            selection.aim = None;
        }
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
            PlayerCommand::NextUnit | PlayerCommand::PreviousUnit => {
                let next = if *command == PlayerCommand::NextUnit {
                    next_ready_unit(&state.battle, selection.unit)
                } else {
                    previous_ready_unit(&state.battle, selection.unit)
                };
                selection.select(next);
                if let Some(u) = next.and_then(|id| state.battle.unit(id)) {
                    rig.focus_at(crate::coords::tile_top(state.battle.map(), u.pos));
                }
                continue;
            }
            PlayerCommand::FocusSelected => {
                if let Some(u) = selection.unit.and_then(|id| state.battle.unit(id)) {
                    rig.focus_at(crate::coords::tile_top(state.battle.map(), u.pos));
                }
                continue;
            }
            PlayerCommand::CancelTargeting => {
                // Step back one level: aim → targeting → nothing.
                if selection.aim.take().is_none() {
                    selection.pending = None;
                }
                rig.restore_zoom();
                continue;
            }
            PlayerCommand::Confirm => {
                fire(&mut state, &mut selection, &mut playback, &mut log);
                continue;
            }
            PlayerCommand::ConfirmOrEndTurn => {
                if selection.aim.is_some() {
                    fire(&mut state, &mut selection, &mut playback, &mut log);
                } else if selection.pending.is_none() {
                    let _ = state.apply(Action::EndTurn, &mut playback);
                }
                continue;
            }
            PlayerCommand::EndTurn => {
                selection.pending = None;
                selection.aim = None;
                let _ = state.apply(Action::EndTurn, &mut playback);
                continue;
            }
            // Battle (re)starts are handled by `missions` (they work even when the battle is over).
            PlayerCommand::Restart | PlayerCommand::StartMission(_) | PlayerCommand::Skirmish => continue,
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
            // Pressing it again fires at the aimed target, or cancels.
            if selection.aim.is_some() {
                fire(&mut state, &mut selection, &mut playback, &mut log);
            } else {
                selection.pending = None;
            }
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
                    // Keep aiming at the same target if the new ability can use it.
                    let keep = selection.aim.filter(|t| {
                        state.battle.validate(&Action::UseAbility { unit, ability: ability.clone(), target: *t }).is_ok()
                    });
                    if keep.is_none() {
                        log.push(format!("{}: choose a target", def.name));
                    }
                    selection.aim = keep;
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

/// The previous friendly unit (before `current`, wrapping) that still has AP.
pub fn previous_ready_unit(battle: &Battle, current: Option<UnitId>) -> Option<UnitId> {
    let ready: Vec<UnitId> = battle.living(Team::Player).filter(|u| u.can_act()).map(|u| u.id).collect();
    let idx = current.and_then(|c| ready.iter().rposition(|id| *id < c)).unwrap_or(ready.len().checked_sub(1)?);
    Some(ready[idx])
}

/// Keeps a sensible unit selected: if the current one is spent or dead, pick
/// the next ready one (only when it's the player's turn and nothing animates).
fn auto_select(
    state: Res<BattleState>,
    playback: Res<Playback>,
    director: Res<CameraDirector>,
    mut selection: ResMut<Selection>,
    mut rig: Single<&mut CameraRig>,
    mut focused_round: Local<u32>,
) {
    if playback.is_busy() || director.is_active() || state.battle.active_team() != Team::Player || state.battle.is_over() {
        return;
    }
    // Things have calmed down: give the player their zoom back (unless
    // they're lining up a shot, which framed the camera on purpose).
    if selection.aim.is_none() {
        rig.restore_zoom();
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
            rig.focus_at(crate::coords::tile_top(state.battle.map(), u.pos));
        }
    }
    selection.refresh_range(&state);
}

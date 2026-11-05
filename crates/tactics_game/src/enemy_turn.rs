//! Drives the enemy team with the core AI.
//!
//! One action per decision. Pacing keeps the alien turn readable without
//! dragging:
//! * actions the player can't see (hidden by fog) and units that just wait
//!   (dormant pods) resolve instantly, without moving the camera, so nothing
//!   is given away and big maps don't stall;
//! * visible actions get a short "thinking" beat and the camera follows the
//!   actor lazily (only if it's off-centre);
//! * **hordes move at once**: every horde unit (zombies) that wants to walk
//!   gets its move from the AI, all the moves are applied, then their walks
//!   play together ([`horde_moves`]). Bites and anything that interrupts a
//!   walk (overwatch fire, burning) still play one at a time.

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::camera_director::CameraDirector;
use crate::playback::Playback;
use crate::state::BattleState;

pub struct EnemyTurnPlugin;

impl Plugin for EnemyTurnPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EnemyBrain {
            config: AiConfig::default(),
            delay: Timer::from_seconds(0.3, TimerMode::Once),
        })
        .add_systems(Update, enemy_think.in_set(FrameSet::Logic));
    }
}

/// AI settings and pacing.
#[derive(Resource)]
pub struct EnemyBrain {
    pub config: AiConfig,
    /// Pause before each decision.
    pub delay: Timer,
}

fn enemy_think(
    time: Res<Time>,
    mut brain: ResMut<EnemyBrain>,
    mut state: ResMut<BattleState>,
    mut playback: ResMut<Playback>,
    director: Res<CameraDirector>,
    settings: Res<crate::settings::GameSettings>,
    mut rig: Single<&mut CameraRig>,
) {
    if playback.is_busy() || director.is_active() || state.battle.is_over() || state.battle.active_team() != Team::Enemy {
        brain.delay.reset();
        return;
    }
    // Modules can retune the AI by registering an AI profile called "default".
    let config = state.battle.rules().ai_profile("default").map_or(brain.config, |p| p.config);
    let config = settings.ai_config(config);
    // The horde shuffles forward as one.
    if horde_moves(&mut state, &mut playback, &config) {
        return;
    }
    // Quiet actions (units waiting, or acting unseen in the fog) resolve
    // back to back within a few milliseconds per frame: a city full of
    // dormant zombies passes its turn at once.
    let started = bevy::platform::time::Instant::now();
    let mut action = ai::next_team_action(&state.battle, &config);
    loop {
        let actor = action.actor().and_then(|id| state.battle.unit(id));
        let seen = actor.is_some_and(|u| state.battle.is_revealed(u.id));
        let quiet = !seen || matches!(action, Action::Wait { .. });
        if !quiet || matches!(action, Action::EndTurn) || started.elapsed().as_secs_f32() > 0.012 {
            break;
        }
        if state.apply(action, &mut playback).is_err() {
            let _ = state.apply(Action::EndTurn, &mut playback);
            return;
        }
        if playback.is_busy() || state.battle.is_over() || state.battle.active_team() != Team::Enemy {
            return;
        }
        action = ai::next_team_action(&state.battle, &config);
    }
    let actor = action.actor().and_then(|id| state.battle.unit(id));
    let seen = actor.is_some_and(|u| state.battle.is_revealed(u.id));
    let quiet = !seen || matches!(action, Action::Wait { .. } | Action::EndTurn);
    if !quiet {
        // Look at the unit about to act during the "thinking" pause.
        if let Some(pos) = actor.map(|u| u.pos) {
            let p = crate::coords::tile_top(state.battle.map(), pos);
            if !director.is_active() {
                rig.follow(p);
            }
        }
        if !brain.delay.tick(time.delta()).is_finished() {
            return;
        }
    }
    brain.delay.reset();
    if state.apply(action, &mut playback).is_err() {
        // Should never happen (the AI only returns legal actions), but never
        // let a bug soft-lock the game: hand the turn back.
        let _ = state.apply(Action::EndTurn, &mut playback);
    }
}

/// Moves every horde unit that wants to walk (until none does, so units
/// with action points left walk again), then queues all the walks to play
/// together. A move that does more than walk (draws overwatch fire, walks
/// into fire, ends the turn...) closes the batch: what came before plays
/// together, then its events play normally. Returns whether anything moved.
fn horde_moves(state: &mut BattleState, playback: &mut Playback, config: &AiConfig) -> bool {
    let simple = |e: &GameEvent| matches!(e, GameEvent::UnitMoved { .. } | GameEvent::ActionPointsChanged { .. });
    let mut together: Vec<GameEvent> = Vec::new();
    let mut moved = false;
    for _pass in 0..4 {
        let walkers: Vec<UnitId> = state.battle.living(Team::Enemy).filter(|u| u.ai.horde && !u.ai.dormant && u.can_act()).map(|u| u.id).collect();
        let mut any = false;
        for id in walkers {
            if state.battle.is_over() || state.battle.active_team() != Team::Enemy {
                break;
            }
            if !state.battle.unit(id).is_some_and(|u| u.can_act()) {
                continue;
            }
            let action = ai::choose_action(&state.battle, id, config);
            if !matches!(action, Action::Move { .. }) {
                continue;
            }
            let Ok(events) = state.apply_quietly(action) else { continue };
            any = true;
            moved = true;
            if events.iter().all(simple) {
                together.extend(events);
            } else {
                playback.enqueue_together(std::mem::take(&mut together));
                playback.enqueue(events);
                return true;
            }
        }
        if !any {
            break;
        }
    }
    playback.enqueue_together(together);
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use tactics_core::grid::{GridPos, Map};

    /// Every zombie that can walk does, in one batch, before any bites.
    #[test]
    fn the_horde_moves_in_one_batch() {
        let (rules, _) = crate::content::build_ruleset();
        let mut battle = Battle::new(Map::from_ascii(&"..............................\n".repeat(10)), rules, 4);
        battle.spawn("rifleman", "A", Team::Player, GridPos::new(0, 5)).unwrap();
        let zombies: Vec<UnitId> = (0..8).map(|i| battle.spawn("zombie", format!("Z{i}"), Team::Enemy, GridPos::new(20 + i % 4, 2 + i))).collect::<Result<_, _>>().unwrap();
        battle.apply(Action::EndTurn).unwrap();
        let mut state = BattleState::new(battle);
        let mut playback = Playback::default();
        let before: Vec<GridPos> = zombies.iter().map(|z| state.battle.unit(*z).unwrap().pos).collect();
        assert!(horde_moves(&mut state, &mut playback, &AiConfig::default()));
        let moved = zombies.iter().zip(&before).filter(|(z, b)| state.battle.unit(**z).unwrap().pos != **b).count();
        assert_eq!(moved, zombies.len(), "all of them shuffle forward");
        assert!(playback.is_busy(), "their walks are queued");
        assert!(zombies.iter().all(|z| !state.battle.unit(*z).unwrap().can_act()), "both moves used");
    }
}

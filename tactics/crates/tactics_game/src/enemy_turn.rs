//! Drives the enemy team with the core AI.
//!
//! One action per decision; a short "thinking" delay between decisions plus
//! waiting for playback to finish keeps the alien turn readable.

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
            delay: Timer::from_seconds(0.45, TimerMode::Once),
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
    mut rig: Single<&mut CameraRig>,
) {
    if playback.is_busy() || director.is_active() || state.battle.is_over() || state.battle.active_team() != Team::Enemy {
        brain.delay.reset();
        return;
    }
    // Look at the unit about to act during the "thinking" pause.
    // Modules can retune the AI by registering an AI profile called "default".
    let config = state.battle.rules().ai_profile("default").map_or(brain.config, |p| p.config);
    let action = ai::next_team_action(&state.battle, &config);
    if let Some(pos) = action.actor().and_then(|id| state.battle.unit(id)).map(|u| u.pos) {
        rig.focus_at(crate::coords::tile_top(state.battle.map(), pos));
    }
    if !brain.delay.tick(time.delta()).is_finished() {
        return;
    }
    brain.delay.reset();
    if state.apply(action, &mut playback).is_err() {
        // Should never happen (the AI only returns legal actions), but never
        // let a bug soft-lock the game: hand the turn back.
        let _ = state.apply(Action::EndTurn, &mut playback);
    }
}

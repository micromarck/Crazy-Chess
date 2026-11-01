//! Bevy resources wrapping the core battle, plus the "current selection".
//!
//! [`BattleState`] is the single owner of the [`Battle`]. Its
//! [`apply`](BattleState::apply) method is the one place where actions enter
//! the rules engine; it forwards the resulting events to
//! [`Playback`](crate::playback::Playback) and bumps a revision counter that
//! other systems use to invalidate caches.

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::playback::Playback;

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BattleState::new(scenario::demo(DEFAULT_SEED)))
            .init_resource::<Selection>()
            .add_message::<PlayerCommand>()
            .add_message::<BattleReset>();
    }
}

/// Seed of the first battle; restarts pick new seeds.
pub const DEFAULT_SEED: u64 = 0x5EED;

/// The authoritative battle and a change counter.
#[derive(Resource)]
pub struct BattleState {
    pub battle: Battle,
    /// Incremented every time the battle changes. Cheap cache invalidation.
    pub revision: u64,
    /// Seed of the current battle (for restarts and bug reports).
    pub seed: u64,
}

impl BattleState {
    pub fn new(battle: Battle) -> Self {
        Self { battle, revision: 0, seed: DEFAULT_SEED }
    }

    /// Applies an action and queues its events for animation.
    /// Returns the error (also logged) if the action was illegal.
    pub fn apply(&mut self, action: Action, playback: &mut Playback) -> Result<(), ActionError> {
        match self.battle.apply(action.clone()) {
            Ok(events) => {
                self.revision += 1;
                playback.enqueue(events);
                Ok(())
            }
            Err(err) => {
                warn!("rejected {action:?}: {err}");
                Err(err)
            }
        }
    }
}

/// What the player is currently looking at / controlling.
#[derive(Resource, Default)]
pub struct Selection {
    /// The friendly unit receiving orders.
    pub unit: Option<UnitId>,
    /// The tile under the mouse/finger, if it's on the map.
    pub hovered: Option<GridPos>,
    /// Cached movement range of `unit`, valid for `range_revision`.
    pub range: Option<ReachableSet>,
    range_key: Option<(UnitId, u64)>,
}

impl Selection {
    /// Recomputes the cached movement range if the unit or battle changed.
    pub fn refresh_range(&mut self, state: &BattleState) {
        let key = self.unit.map(|u| (u, state.revision));
        if key != self.range_key {
            self.range_key = key;
            self.range = self.unit.map(|u| state.battle.movement_range(u));
        }
    }
}

/// High-level player intents coming from keyboard shortcuts or HUD buttons.
/// Routing both through one message keeps desktop and touch controls identical.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerCommand {
    NextUnit,
    Overwatch,
    Hunker,
    Reload,
    EndTurn,
    Restart,
}

/// Sent after the battle has been replaced, so visuals can be rebuilt.
#[derive(Message, Debug, Clone, Copy)]
pub struct BattleReset;

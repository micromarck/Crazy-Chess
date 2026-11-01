//! Bevy resources wrapping the core battle, plus the "current selection".
//!
//! [`BattleState`] is the single owner of the [`Battle`]. Its
//! [`apply`](BattleState::apply) method is the one place where actions enter
//! the rules engine; it forwards the resulting events to
//! [`Playback`](crate::playback::Playback) and bumps a revision counter that
//! other systems use to invalidate caches. Live content edits (the F1 editor)
//! also bump the revision.

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::content::build_ruleset;
use crate::playback::Playback;

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        let (rules, load_log) = build_ruleset();
        for line in &load_log {
            info!("{line}");
        }
        app.insert_resource(BattleState::new(scenario::demo(&rules, DEFAULT_SEED)))
            .insert_resource(ModuleLoadLog(load_log))
            .init_resource::<Selection>()
            .add_message::<PlayerCommand>()
            .add_message::<BattleReset>();
    }
}

/// Seed of the first battle; restarts pick new seeds.
pub const DEFAULT_SEED: u64 = 0x5EED;

/// Messages produced while loading modules (shown in the combat log at start).
#[derive(Resource, Default)]
pub struct ModuleLoadLog(pub Vec<String>);

/// The authoritative battle and a change counter.
#[derive(Resource)]
pub struct BattleState {
    pub battle: Battle,
    /// Incremented every time the battle or its content changes.
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

    /// Mutable access to the content for live tuning; marks the state changed.
    pub fn rules_mut(&mut self) -> &mut Ruleset {
        self.revision += 1;
        self.battle.rules_mut()
    }
}

/// What the player is currently looking at / controlling.
#[derive(Resource, Default)]
pub struct Selection {
    /// The friendly unit receiving orders.
    pub unit: Option<UnitId>,
    /// The tile under the mouse/finger, if it's on the map.
    pub hovered: Option<GridPos>,
    /// Ability waiting for a target (targeting mode), if any.
    pub pending: Option<AbilityId>,
    /// Cached movement range of `unit`.
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

    /// Selects a unit (cancelling any targeting in progress).
    pub fn select(&mut self, unit: Option<UnitId>) {
        if unit != self.unit {
            self.pending = None;
        }
        self.unit = unit;
    }
}

/// High-level player intents from keyboard shortcuts or HUD buttons. Routing
/// both through one message keeps desktop and touch controls identical.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum PlayerCommand {
    NextUnit,
    /// Use (self-targeted) or start targeting (others) an ability of the
    /// selected unit.
    UseAbility(AbilityId),
    /// Use the selected unit's n-th ability (hotkeys 1-9).
    AbilitySlot(usize),
    CancelTargeting,
    EndTurn,
    Restart,
}

/// Sent after the battle has been replaced, so visuals can be rebuilt.
#[derive(Message, Debug, Clone, Copy)]
pub struct BattleReset;

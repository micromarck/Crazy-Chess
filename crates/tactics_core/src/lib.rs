//! # tactics_core
//!
//! Engine-agnostic rules for a turn-based squad tactics game in the spirit of
//! XCOM: grid movement with action points, directional cover and flanking,
//! line of sight, percentage-based shooting, overwatch reaction fire and a
//! utility AI.
//!
//! All content (abilities, weapons, classes, maps...) comes from **modules**;
//! see [`content`] and `MODULES.md`.
//!
//! This crate knows nothing about rendering, input or time, and with
//! `default-features = false` has **no dependencies** (the default `data`
//! feature adds serde + RON for data-file modules). That makes it:
//! * **portable** — it compiles for every target Rust supports (desktop, Android,
//!   WebAssembly, a headless dedicated server...);
//! * **deterministic** — same seed + same actions = same battle;
//! * **testable** — `cargo test -p tactics_core` exercises the whole ruleset in
//!   milliseconds.
//!
//! ## Architecture at a glance
//!
//! ```text
//!   frontend (Bevy, CLI, server...)          tactics_core
//!   ───────────────────────────────          ─────────────────────────────
//!   input ──► builds Action ───────────────► Battle::validate / apply
//!                                                 │ mutates state, rolls dice
//!   animation ◄── plays GameEvents ◄──────────────┘ returns Vec<GameEvent>
//!   UI ◄──────── queries (movement_range, shot_preview, ...) ◄── &Battle
//! ```
//!
//! ## Module map
//! | module | responsibility |
//! |---|---|
//! | [`grid`] | tiles, terrain, coordinates |
//! | [`pathfinding`] | movement range (Dijkstra) and A* |
//! | [`visibility`] | line of sight |
//! | [`cover`] | directional cover and flanking |
//! | [`content`] | the module system: `Ruleset`, `GameModule`, ability definitions |
//! | [`modules`] | built-in modules (base game + example add-on) |
//! | [`unit`] | units, weapons, classes |
//! | [`combat`] | hit / crit / damage math |
//! | [`action`] | commands (`Move`, `UseAbility`...) and validation errors |
//! | [`event`] | the event stream frontends animate |
//! | [`battle`] | authoritative state + rules |
//! | [`ai`] | enemy decision making |
//! | [`scenario`] | demo and procedural battles |
//! | [`mission`] | XML mission files: map, heights, units, props, zones, scripts |
//! | [`script`] | Rhai scripting attached to missions, tiles, props and units |
//! | [`rng`] | deterministic randomness |
//!
//! ## Minimal example
//! ```
//! use tactics_core::prelude::*;
//!
//! let rules = Ruleset::standard(); // or add your own modules
//! let mut battle = scenario::demo(&rules, 42);
//! let soldier = battle.living(Team::Player).next().unwrap().id;
//! let dest = battle.movement_range(soldier).iter().next().unwrap().0;
//! let events = battle.apply(Action::Move { unit: soldier, to: dest }).unwrap();
//! assert!(matches!(events.last(), Some(GameEvent::UnitMoved { .. })));
//! ```

pub mod action;
pub mod ai;
#[cfg(test)]
mod ai_legacy;
pub mod battle;
pub mod combat;
pub mod content;
pub mod cover;
pub mod event;
pub mod grid;
pub mod mission;
pub mod modules;
pub mod pathfinding;
pub mod rng;
pub mod scenario;
pub mod script;
pub mod unit;
pub mod visibility;

/// Everything a frontend typically needs, in one import.
pub mod prelude {
    pub use crate::action::{Action, ActionError, Target};
    pub use crate::ai::{self, AiConfig};
    pub use crate::battle::Battle;
    pub use crate::combat::ShotPreview;
    pub use crate::content::{
        AbilityDef, AbilityEffect, AbilityId, AbilityRange, ApCost, AttackProfile, DamageSource, GameModule,
        ModuleInfo, Ruleset, Targeting,
    };
    pub use crate::cover::{cover_against, cover_around, CoverResult};
    pub use crate::event::{GameEvent, ShotOutcome};
    pub use crate::grid::{Climb, CoverLevel, Direction, GridPos, Map, Terrain};
    pub use crate::mission::MissionDef;
    pub use crate::pathfinding::ReachableSet;
    pub use crate::scenario;
    pub use crate::modules;
    pub use crate::unit::{Team, Unit, UnitClass, UnitId, UnitStats, Weapon};
}

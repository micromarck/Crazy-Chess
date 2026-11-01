//! # tactics_core
//!
//! Engine-agnostic rules for a turn-based squad tactics game in the spirit of
//! XCOM: grid movement with action points, directional cover and flanking,
//! line of sight, percentage-based shooting, overwatch reaction fire and a
//! utility AI.
//!
//! This crate has **no dependencies** and knows nothing about rendering, input
//! or time. That makes it:
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
//! | [`unit`] | unit/weapon data and templates |
//! | [`combat`] | hit / crit / damage math |
//! | [`action`] | commands and validation errors |
//! | [`event`] | the event stream frontends animate |
//! | [`battle`] | authoritative state + rules |
//! | [`ai`] | enemy decision making |
//! | [`scenario`] | demo and procedural battles |
//! | [`rng`] | deterministic randomness |
//!
//! ## Minimal example
//! ```
//! use tactics_core::prelude::*;
//!
//! let mut battle = scenario::demo(42);
//! let soldier = battle.living(Team::Player).next().unwrap().id;
//! let dest = battle.movement_range(soldier).iter().next().unwrap().0;
//! let events = battle.apply(Action::Move { unit: soldier, to: dest }).unwrap();
//! assert!(matches!(events.last(), Some(GameEvent::UnitMoved { .. })));
//! ```

pub mod action;
pub mod ai;
pub mod battle;
pub mod combat;
pub mod cover;
pub mod event;
pub mod grid;
pub mod pathfinding;
pub mod rng;
pub mod scenario;
pub mod unit;
pub mod visibility;

/// Everything a frontend typically needs, in one import.
pub mod prelude {
    pub use crate::action::{Action, ActionError};
    pub use crate::ai::{self, AiConfig};
    pub use crate::battle::Battle;
    pub use crate::combat::ShotPreview;
    pub use crate::cover::{cover_against, cover_around, CoverResult};
    pub use crate::event::{GameEvent, ShotOutcome};
    pub use crate::grid::{CoverLevel, Direction, GridPos, Map, Terrain};
    pub use crate::pathfinding::ReachableSet;
    pub use crate::scenario;
    pub use crate::unit::{Team, Unit, UnitId, UnitStats, UnitTemplate, Weapon};
}

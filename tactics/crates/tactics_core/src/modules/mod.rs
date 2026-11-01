//! Built-in content modules.
//!
//! * [`BaseModule`] — the core game: shoot, overwatch, hunker, reload, four
//!   weapons, four classes, default squads, the demo map and AI profiles.
//! * [`GrenadierModule`] — an **example** of an add-on module written in Rust:
//!   a new class with an area-of-effect grenade and a medkit, and a rebalanced
//!   player squad. Copy it as a template for your own modules.
//!
//! To add a module: create a file here (or in any crate), implement
//! [`GameModule`](crate::content::GameModule), and add it to the module list
//! the game builds its [`Ruleset`](crate::content::Ruleset) from
//! (`tactics_game/src/content.rs`).

mod base;
mod grenadier;

pub use base::BaseModule;
pub use grenadier::GrenadierModule;

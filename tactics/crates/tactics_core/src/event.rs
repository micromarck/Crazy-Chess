//! Events: the battle's output stream.
//!
//! [`Battle::apply`](crate::battle::Battle::apply) returns the *consequences* of an
//! action as an ordered list of [`GameEvent`]s. The rules engine resolves
//! everything instantly; the frontend then plays the events back one by one
//! with animations, sounds and camera moves. This split ("simulate, then
//! present") is the key to keeping game logic deterministic and testable while
//! still letting the presentation take as long as it likes.
//!
//! Example: a move that triggers overwatch yields
//! `UnitMoved(first half) -> ShotFired(reaction) -> UnitMoved(second half)`,
//! which the renderer animates in exactly that order.

use crate::grid::GridPos;
use crate::unit::{Team, UnitId};

/// What happened when a shot was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotOutcome {
    Miss,
    Hit { damage: i32, critical: bool },
}

/// Something observable that happened in the battle.
#[derive(Debug, Clone, PartialEq)]
pub enum GameEvent {
    /// A team's turn began; its units had their action points restored.
    TurnStarted { team: Team, round: u32 },
    /// A unit walked along `path` (excluding its starting tile `from`).
    UnitMoved {
        unit: UnitId,
        from: GridPos,
        path: Vec<GridPos>,
    },
    /// A weapon was fired.
    ShotFired {
        shooter: UnitId,
        target: UnitId,
        hit_chance: i32,
        outcome: ShotOutcome,
        /// `true` for overwatch reaction fire.
        reaction: bool,
    },
    /// A unit's hit points dropped to zero.
    UnitKilled { unit: UnitId },
    /// A unit entered overwatch.
    OverwatchSet { unit: UnitId },
    /// A unit hunkered down.
    Hunkered { unit: UnitId },
    /// A unit reloaded its weapon.
    Reloaded { unit: UnitId },
    /// A unit's remaining action points changed.
    ActionPointsChanged { unit: UnitId, remaining: u8 },
    /// One team has been wiped out.
    BattleEnded { winner: Team },
}

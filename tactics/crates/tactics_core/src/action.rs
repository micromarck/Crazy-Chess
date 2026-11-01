//! Player/AI commands and why they can fail.
//!
//! **Every** state change in a battle goes through an [`Action`]. Frontends never
//! mutate units directly; they build an `Action`, optionally call
//! [`Battle::validate`](crate::battle::Battle::validate) to grey out buttons, and
//! then [`Battle::apply`](crate::battle::Battle::apply) it. This single choke point
//! is what makes networking, replays and undo straightforward: record the actions,
//! replay them against the same seed.

use std::fmt;

use crate::grid::GridPos;
use crate::unit::UnitId;

/// A command issued by a player or the AI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Walk to `to`. Costs 1 action point per `mobility` movement points used
    /// (so a long "dash" costs 2).
    Move { unit: UnitId, to: GridPos },
    /// Fire the equipped weapon. Ends the unit's turn.
    Shoot { unit: UnitId, target: UnitId },
    /// Prepare a reaction shot against the next enemy that moves in sight.
    /// Ends the unit's turn.
    Overwatch { unit: UnitId },
    /// Take a defensive stance (+defense). Ends the unit's turn.
    Hunker { unit: UnitId },
    /// Refill the magazine. Costs 1 action point.
    Reload { unit: UnitId },
    /// Do nothing for the rest of this turn.
    Wait { unit: UnitId },
    /// Hand control to the other team.
    EndTurn,
}

impl Action {
    /// The unit performing the action, if any.
    pub fn actor(&self) -> Option<UnitId> {
        match self {
            Action::Move { unit, .. }
            | Action::Shoot { unit, .. }
            | Action::Overwatch { unit }
            | Action::Hunker { unit }
            | Action::Reload { unit }
            | Action::Wait { unit } => Some(*unit),
            Action::EndTurn => None,
        }
    }
}

/// Reasons an action is rejected. The battle state is unchanged when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    BattleOver,
    UnknownUnit(UnitId),
    UnitDead(UnitId),
    NotYourTurn(UnitId),
    NoActionPoints(UnitId),
    Unreachable(GridPos),
    InvalidTarget(UnitId),
    NoLineOfSight,
    OutOfRange,
    OutOfAmmo,
    AlreadyFullAmmo,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::BattleOver => write!(f, "the battle is over"),
            ActionError::UnknownUnit(id) => write!(f, "unit {} does not exist", id.0),
            ActionError::UnitDead(id) => write!(f, "unit {} is dead", id.0),
            ActionError::NotYourTurn(id) => write!(f, "unit {} cannot act this turn", id.0),
            ActionError::NoActionPoints(id) => write!(f, "unit {} has no action points left", id.0),
            ActionError::Unreachable(p) => write!(f, "tile {p} is not reachable"),
            ActionError::InvalidTarget(id) => write!(f, "unit {} is not a valid target", id.0),
            ActionError::NoLineOfSight => write!(f, "no line of sight"),
            ActionError::OutOfRange => write!(f, "target is out of range"),
            ActionError::OutOfAmmo => write!(f, "weapon is empty, reload first"),
            ActionError::AlreadyFullAmmo => write!(f, "magazine is already full"),
        }
    }
}

impl std::error::Error for ActionError {}

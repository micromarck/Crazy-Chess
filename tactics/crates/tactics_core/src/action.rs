//! Player/AI commands and why they can fail.
//!
//! **Every** state change in a battle goes through an [`Action`]. Frontends never
//! mutate units directly; they build an `Action`, optionally call
//! [`Battle::validate`](crate::battle::Battle::validate) to grey out buttons, and
//! then [`Battle::apply`](crate::battle::Battle::apply) it. This single choke point
//! is what makes networking, replays and undo straightforward: record the actions,
//! replay them against the same seed and ruleset.
//!
//! Apart from walking, waiting and ending the turn, everything is
//! [`Action::UseAbility`] — new moves are content, not new action variants.

use std::fmt;

use crate::content::AbilityId;
use crate::grid::GridPos;
use crate::unit::UnitId;

/// What an ability is aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Target {
    /// Self-targeted abilities.
    #[default]
    None,
    Unit(UnitId),
    Tile(GridPos),
}

/// A command issued by a player or the AI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Walk to `to`. Costs 1 action point per `mobility` movement points used
    /// (so a long "dash" costs 2).
    Move { unit: UnitId, to: GridPos },
    /// Use one of the unit's abilities (shoot, overwatch, grenade, heal...).
    UseAbility { unit: UnitId, ability: AbilityId, target: Target },
    /// Do nothing for the rest of this turn.
    Wait { unit: UnitId },
    /// Hand control to the other team.
    EndTurn,
}

impl Action {
    /// Convenience constructor: `Action::ability(unit, "shoot", Target::Unit(enemy))`.
    pub fn ability(unit: UnitId, ability: impl Into<String>, target: Target) -> Self {
        Action::UseAbility { unit, ability: AbilityId::new(ability), target }
    }

    /// The unit performing the action, if any.
    pub fn actor(&self) -> Option<UnitId> {
        match self {
            Action::Move { unit, .. } | Action::UseAbility { unit, .. } | Action::Wait { unit } => Some(*unit),
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
    UnknownAbility(AbilityId),
    AbilityNotKnown(AbilityId),
    OnCooldown { ability: AbilityId, turns: u32 },
    NoCharges(AbilityId),
    WrongTarget,
    InvalidTarget(UnitId),
    NoLineOfSight,
    OutOfRange,
    OutOfAmmo,
    AlreadyFullAmmo,
    NothingToHeal,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::BattleOver => write!(f, "the battle is over"),
            ActionError::UnknownUnit(id) => write!(f, "unit {} does not exist", id.0),
            ActionError::UnitDead(id) => write!(f, "unit {} is dead", id.0),
            ActionError::NotYourTurn(id) => write!(f, "unit {} cannot act this turn", id.0),
            ActionError::NoActionPoints(_) => write!(f, "not enough action points"),
            ActionError::Unreachable(p) => write!(f, "tile {p} is not reachable"),
            ActionError::UnknownAbility(a) => write!(f, "no ability called '{a}' is loaded"),
            ActionError::AbilityNotKnown(a) => write!(f, "this unit doesn't know '{a}'"),
            ActionError::OnCooldown { turns, .. } => write!(f, "on cooldown ({turns} turns)"),
            ActionError::NoCharges(_) => write!(f, "no uses left"),
            ActionError::WrongTarget => write!(f, "wrong kind of target"),
            ActionError::InvalidTarget(_) => write!(f, "not a valid target"),
            ActionError::NoLineOfSight => write!(f, "no line of sight"),
            ActionError::OutOfRange => write!(f, "out of range"),
            ActionError::OutOfAmmo => write!(f, "out of ammo, reload first"),
            ActionError::AlreadyFullAmmo => write!(f, "magazine is already full"),
            ActionError::NothingToHeal => write!(f, "target is at full health"),
        }
    }
}

impl std::error::Error for ActionError {}

//! Events: the battle's output stream.
//!
//! [`Battle::apply`](crate::battle::Battle::apply) returns the *consequences* of an
//! action as an ordered list of [`GameEvent`]s. The rules engine resolves
//! everything instantly; the frontend then plays the events back one by one
//! with animations, sounds and camera moves. This split ("simulate, then
//! present") keeps game logic deterministic and testable while letting the
//! presentation take as long as it likes.
//!
//! Example: a move that triggers overwatch yields
//! `UnitMoved(first half) -> AttackResolved(reaction) -> UnitMoved(second half)`.

use crate::action::Target;
use crate::battle::PropId;
use crate::content::AbilityId;
use crate::grid::GridPos;
use crate::unit::{Team, UnitId};

/// What happened when an attack was resolved against one target.
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
    UnitMoved { unit: UnitId, from: GridPos, path: Vec<GridPos> },
    /// A unit activated an ability. Always precedes the ability's results, so
    /// frontends can show its name / play its animation.
    AbilityUsed { unit: UnitId, ability: AbilityId, target: Target },
    /// An area attack detonated.
    Explosion { center: GridPos, radius: f32 },
    /// One attack roll against one target.
    AttackResolved {
        attacker: UnitId,
        ability: AbilityId,
        target: UnitId,
        /// Where the attack came from (attacker tile, or blast centre).
        origin: GridPos,
        hit_chance: i32,
        outcome: ShotOutcome,
        /// `true` for overwatch reaction fire.
        reaction: bool,
    },
    /// A unit regained hit points.
    UnitHealed { unit: UnitId, amount: i32 },
    /// A unit's hit points dropped to zero.
    UnitKilled { unit: UnitId },
    /// A unit's remaining action points changed.
    ActionPointsChanged { unit: UnitId, remaining: u8 },
    /// The battle is decided (a team was wiped out, or a script called win/lose).
    BattleEnded { winner: Team },
    /// A unit appeared mid-battle (script `spawn_unit`).
    UnitSpawned { unit: UnitId },
    /// A unit took damage outside an attack roll (explosions, scripts).
    UnitDamaged { unit: UnitId, amount: i32 },
    /// A destructible prop was hit; `hp` is what's left.
    PropDamaged { prop: PropId, hp: i32 },
    /// A prop was destroyed (its tile becomes open floor).
    PropDestroyed { prop: PropId },
    /// A tile's terrain, height or ladder changed; redraw it.
    TerrainChanged { pos: GridPos },
    /// A message from a mission script.
    Message { text: String },
    /// A mission script failed (syntax/runtime error, limit reached).
    ScriptError { message: String },
}

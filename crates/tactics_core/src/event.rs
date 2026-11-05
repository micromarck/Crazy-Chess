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

/// One traced projectile of a [`GameEvent::Volley`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bullet {
    /// Where it stopped: grid x, grid y (tiles, fractional) and height above
    /// level 0 in **metres** (a level is 0.5 m).
    pub end: [f32; 3],
    /// The unit it hit, if any (friend or foe).
    pub hit: Option<UnitId>,
    /// A destructible prop it hit, if any.
    pub prop: Option<PropId>,
    /// Damage dealt (0 = no unit hit).
    pub damage: i32,
    pub critical: bool,
    /// Stopped by the intended target's cover.
    pub blocked: bool,
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
    /// A ballistic attack: every projectile traced as a ray (see
    /// `battle::ballistics`). Damage is applied right after this event.
    Volley {
        attacker: UnitId,
        ability: AbilityId,
        /// The unit aimed at.
        target: UnitId,
        /// Muzzle position: grid x, y and height in metres.
        origin: [f32; 3],
        /// Per-projectile hit chance against the target (for logs).
        hit_chance: i32,
        /// Overwatch / suppression reaction fire.
        reaction: bool,
        /// Suppressive fire (pins the target).
        suppress: bool,
        bullets: Vec<Bullet>,
    },
    /// A unit regained hit points.
    UnitHealed { unit: UnitId, amount: i32 },
    /// A unit that runs on heat got hotter (`delta` > 0: shooting, moving)
    /// or cooler (`delta` < 0: cooling, venting). `heat` / `max` = now.
    /// (Damage adds heat too, reported as `UnitDamaged`.)
    HeatChanged { unit: UnitId, delta: i32, heat: i32, max: i32 },
    /// A robot powered down (`shut_down`) or rebooted.
    Shutdown { unit: UnitId, shut_down: bool },
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
    /// A soldier walked over a supply cache and took it (`text` says what,
    /// e.g. "+1 magazine"). The prop is gone.
    SupplyFound { unit: UnitId, prop: PropId, text: String },
    /// A blast damaged a wall (or other terrain cover); `hp` is what's left.
    WallDamaged { pos: GridPos, hp: i32 },
    /// A blast broke a wall: high walls collapse into rubble (half cover),
    /// low cover is flattened. A `TerrainChanged` follows.
    WallDestroyed { pos: GridPos },
    /// Grenade fragments flying out of a blast at `center` (grid x, grid y,
    /// height in metres), one `Bullet` per fragment. Damage is applied
    /// after this event, like a `Volley`.
    Shrapnel { attacker: Option<UnitId>, center: [f32; 3], fragments: Vec<Bullet> },
    /// A tile's terrain, height or ladder changed; redraw it.
    TerrainChanged { pos: GridPos },
    /// A message from a mission script.
    Message { text: String },
    /// A short callout over a unit from a script (`say`), e.g. "SYSTEM FAULT".
    Callout { unit: UnitId, text: String },
    /// A mission script failed (syntax/runtime error, limit reached).
    ScriptError { message: String },
    /// The alarm was raised: the squad lost concealment and every enemy is
    /// now active. `by` is the unit that noticed the squad, if any.
    Alarm { by: Option<UnitId> },
    /// A dormant unit became active (its pod was spotted, it was hit...).
    UnitAlerted { unit: UnitId },
    /// A door panel opened or closed (its tile's terrain changed too).
    DoorChanged { prop: PropId, open: bool },
    /// The mission objective text changed (script `objective`).
    ObjectiveChanged { text: String },
    /// A marked zone was shown, hidden or relabelled (scripts).
    ZoneChanged { tag: String },
    /// A unit's fear crossed a threshold (Steady / Shaken / Panicked).
    MoraleChanged { unit: UnitId, level: crate::content::FearLevel, fear: i32 },
}

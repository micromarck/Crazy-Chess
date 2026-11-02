//! Units and the data that describes them.
//!
//! * [`Weapon`] and [`UnitClass`] are **content** — registered in a
//!   [`Ruleset`](crate::content::Ruleset) by modules.
//! * [`Unit`] is a live **instance** on the battlefield, created from a class
//!   by [`Battle::spawn`](crate::battle::Battle::spawn).
//!
//! Units are plain data; all behaviour lives in [`crate::battle`].

use std::collections::BTreeMap;

use crate::content::AbilityId;
use crate::grid::GridPos;

/// Stable identifier for a unit within a battle. Never reused, even after death,
/// so frontends can safely key visuals by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(pub u32);

/// Which side a unit fights for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    Player,
    Enemy,
}

impl Team {
    /// The opposing team.
    pub const fn opponent(self) -> Team {
        match self {
            Team::Player => Team::Enemy,
            Team::Enemy => Team::Player,
        }
    }
}

/// A weapon profile. Abilities with `DamageSource::Weapon` / `AbilityRange::Weapon`
/// read their numbers from here, so one "shoot" ability works with every gun.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Weapon {
    /// Unique id used by [`UnitClass::weapon`].
    pub id: String,
    pub name: String,
    pub damage_min: i32,
    pub damage_max: i32,
    /// Base critical-hit chance (percent).
    pub crit_chance: i32,
    /// Extra damage on a critical hit.
    pub crit_bonus: i32,
    /// Ideal engagement distance in tiles; accuracy falls off away from it.
    pub optimal_range: f32,
    /// Hard maximum distance in tiles.
    pub max_range: f32,
    /// Magazine size.
    pub clip_size: i32,
    /// Rounds currently loaded (instance state: set to `clip_size` on spawn).
    #[cfg_attr(feature = "data", serde(skip))]
    pub ammo: i32,
    /// Classes allowed to carry it in a loadout (empty = everyone).
    pub classes: Vec<String>,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            damage_min: 3,
            damage_max: 5,
            crit_chance: 10,
            crit_bonus: 2,
            optimal_range: 6.0,
            max_range: 18.0,
            clip_size: 4,
            ammo: 0,
            classes: Vec::new(),
        }
    }
}

/// Base statistics of a unit.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct UnitStats {
    pub max_hp: i32,
    /// Movement points per action point (see [`crate::pathfinding`]; 2 per tile).
    pub mobility: u32,
    /// Base accuracy (percent), used by attacks without a fixed `base_accuracy`.
    pub aim: i32,
    /// Subtracted from attackers' hit chance.
    pub defense: i32,
    /// Vision radius in tiles.
    pub sight: f32,
    /// Levels the unit can step **up** in one move (1 = normal infantry,
    /// higher = jump jets / flyers). Ladders allow any climb.
    pub climb: i8,
    /// Fear resistance: 50 = normal, 100 = unshakeable, 0 = fragile
    /// (see [`MoraleRules`](crate::content::MoraleRules)).
    pub bravery: i32,
}

impl Default for UnitStats {
    fn default() -> Self {
        Self {
            max_hp: 8,
            mobility: 12,
            aim: 70,
            defense: 0,
            sight: 14.0,
            climb: 1,
            bravery: 50,
        }
    }
}

/// A unit archetype: stats, weapon and the abilities it knows.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct UnitClass {
    /// Unique id (`"rifleman"`, `"sectoid"`...).
    pub id: String,
    /// Display name of the class.
    pub name: String,
    pub stats: UnitStats,
    /// Weapon id.
    pub weapon: String,
    /// Abilities in hotkey order (1, 2, 3...).
    pub abilities: Vec<AbilityId>,
    /// Hint for the renderer (`"soldier"`, `"alien"`...); the core ignores it.
    pub model: String,
    /// Never afraid (robots, drones, constructs): fear stays at 0.
    pub fearless: bool,
    /// Default loadout preset (see [`LoadoutDef`](crate::content::LoadoutDef)).
    pub loadout: Option<String>,
}

/// Temporary conditions on a unit. A future "status effect" module type will
/// generalise this into a list of timed effects (see `MODULES.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitStatus {
    /// Armed overwatch ability: fires at the first enemy that moves in sight.
    pub overwatch: Option<AbilityId>,
    /// Extra defence from a Defend ability (hunker) until the unit's next turn.
    pub defense_bonus: i32,
}

/// Behaviour state of an AI-controlled unit (mission mechanics).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitAi {
    /// Not yet active: holds position (or walks its patrol) until woken by
    /// being spotted, being hurt, its pod waking, or the alarm.
    pub dormant: bool,
    /// Units sharing a pod name wake together.
    pub pod: Option<String>,
    /// Waypoints walked in a loop while dormant.
    pub patrol: Vec<GridPos>,
    /// Index of the next patrol waypoint.
    pub patrol_index: usize,
}

/// A single combatant on the battlefield.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: UnitId,
    pub name: String,
    /// Class id this unit was created from.
    pub class: String,
    pub team: Team,
    pub pos: GridPos,
    pub hp: i32,
    /// Action points remaining this turn.
    pub action_points: u8,
    pub stats: UnitStats,
    pub weapon: Weapon,
    /// Abilities this unit can use (copied from its class at spawn).
    pub abilities: Vec<AbilityId>,
    /// Remaining cooldown turns per ability (absent = ready).
    pub cooldowns: BTreeMap<AbilityId, u32>,
    /// Remaining uses per limited ability (absent = unlimited / untouched).
    pub charges: BTreeMap<AbilityId, u32>,
    pub status: UnitStatus,
    /// Renderer hint copied from the class.
    pub model: String,
    /// AI behaviour state (dormant pods, patrols).
    pub ai: UnitAi,
    /// 0-100, see [`MoraleRules`](crate::content::MoraleRules).
    pub fear: i32,
    /// Copied from the class: never afraid.
    pub fearless: bool,
    /// What the unit carries (resolved at spawn).
    pub gear: crate::content::Loadout,
}

impl Unit {
    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    /// Can this unit still do something this turn?
    pub fn can_act(&self) -> bool {
        self.is_alive() && self.action_points > 0
    }

    /// Does the unit know this ability?
    pub fn has_ability(&self, id: &AbilityId) -> bool {
        self.abilities.contains(id)
    }

    /// Remaining cooldown of an ability (0 = ready).
    pub fn cooldown(&self, id: &AbilityId) -> u32 {
        self.cooldowns.get(id).copied().unwrap_or(0)
    }
}

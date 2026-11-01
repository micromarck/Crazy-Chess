//! Units (soldiers, aliens, robots...) and the data that describes them.
//!
//! Units are plain data. All behaviour lives in [`crate::battle`], which keeps
//! the rules in one place and makes units trivial to serialize, clone for AI
//! look-ahead, or display in a UI.

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

/// A weapon profile. Copy-able template plus the per-instance ammo counter.
#[derive(Debug, Clone, PartialEq)]
pub struct Weapon {
    pub name: String,
    pub damage_min: i32,
    pub damage_max: i32,
    /// Base critical-hit chance (percent) before flanking bonuses.
    pub crit_chance: i32,
    /// Extra damage on a critical hit.
    pub crit_bonus: i32,
    /// Ideal engagement distance in tiles; accuracy falls off away from it.
    pub optimal_range: f32,
    /// Hard maximum distance in tiles.
    pub max_range: f32,
    /// Magazine size.
    pub clip_size: i32,
    /// Rounds currently loaded.
    pub ammo: i32,
}

impl Weapon {
    /// Standard-issue assault rifle.
    pub fn rifle() -> Self {
        Self {
            name: "Assault Rifle".into(),
            damage_min: 3,
            damage_max: 5,
            crit_chance: 10,
            crit_bonus: 2,
            optimal_range: 6.0,
            max_range: 18.0,
            clip_size: 4,
            ammo: 4,
        }
    }

    /// Close-range, high-damage weapon.
    pub fn shotgun() -> Self {
        Self {
            name: "Shotgun".into(),
            damage_min: 4,
            damage_max: 7,
            crit_chance: 20,
            crit_bonus: 3,
            optimal_range: 2.0,
            max_range: 10.0,
            clip_size: 3,
            ammo: 3,
        }
    }

    /// Long-range precision weapon.
    pub fn sniper() -> Self {
        Self {
            name: "Sniper Rifle".into(),
            damage_min: 4,
            damage_max: 6,
            crit_chance: 25,
            crit_bonus: 3,
            optimal_range: 14.0,
            max_range: 25.0,
            clip_size: 3,
            ammo: 3,
        }
    }

    /// Alien plasma pistol.
    pub fn plasma_pistol() -> Self {
        Self {
            name: "Plasma Pistol".into(),
            damage_min: 2,
            damage_max: 4,
            crit_chance: 5,
            crit_bonus: 2,
            optimal_range: 5.0,
            max_range: 14.0,
            clip_size: 3,
            ammo: 3,
        }
    }
}

/// Base statistics of a unit. Modified by gear, abilities or difficulty in a
/// full game; here they are flat values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitStats {
    pub max_hp: i32,
    /// Movement points per action point (see [`crate::pathfinding`]).
    pub mobility: u32,
    /// Base accuracy (percent).
    pub aim: i32,
    /// Subtracted from attackers' hit chance.
    pub defense: i32,
    /// Vision radius in tiles.
    pub sight: f32,
}

impl Default for UnitStats {
    fn default() -> Self {
        Self {
            max_hp: 8,
            mobility: 12,
            aim: 70,
            defense: 0,
            sight: 14.0,
        }
    }
}

/// Temporary conditions on a unit. Kept as simple flags for clarity; a larger
/// game would turn this into a list of timed effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitStatus {
    /// Will shoot at the first enemy that moves within sight during their turn.
    pub overwatch: bool,
    /// Hunkered down: doubles cover bonus until the unit's next turn.
    pub hunkered: bool,
}

/// A single combatant on the battlefield.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: UnitId,
    pub name: String,
    pub team: Team,
    pub pos: GridPos,
    pub hp: i32,
    /// Action points remaining this turn (XCOM-style: usually 2).
    pub action_points: u8,
    pub stats: UnitStats,
    pub weapon: Weapon,
    pub status: UnitStatus,
}

impl Unit {
    /// Action points every unit receives at the start of its team's turn.
    pub const MAX_ACTION_POINTS: u8 = 2;

    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    /// Can this unit still do something this turn?
    pub fn can_act(&self) -> bool {
        self.is_alive() && self.action_points > 0
    }
}

/// Builder-style helper for creating units without listing every field.
#[derive(Debug, Clone)]
pub struct UnitTemplate {
    pub name: String,
    pub stats: UnitStats,
    pub weapon: Weapon,
}

impl UnitTemplate {
    pub fn new(name: impl Into<String>, stats: UnitStats, weapon: Weapon) -> Self {
        Self {
            name: name.into(),
            stats,
            weapon,
        }
    }

    pub fn rifleman(name: impl Into<String>) -> Self {
        Self::new(name, UnitStats::default(), Weapon::rifle())
    }

    pub fn assault(name: impl Into<String>) -> Self {
        Self::new(
            name,
            UnitStats {
                max_hp: 10,
                mobility: 14,
                aim: 65,
                ..UnitStats::default()
            },
            Weapon::shotgun(),
        )
    }

    pub fn sniper(name: impl Into<String>) -> Self {
        Self::new(
            name,
            UnitStats {
                max_hp: 6,
                mobility: 10,
                aim: 80,
                sight: 18.0,
                ..UnitStats::default()
            },
            Weapon::sniper(),
        )
    }

    pub fn alien_grunt(name: impl Into<String>) -> Self {
        Self::new(
            name,
            UnitStats {
                max_hp: 6,
                mobility: 12,
                aim: 65,
                defense: 5,
                sight: 13.0,
            },
            Weapon::plasma_pistol(),
        )
    }
}

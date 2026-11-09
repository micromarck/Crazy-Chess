//! Equipment: items and loadouts.
//!
//! A unit's gear is a **weapon** (any [`Weapon`](crate::unit::Weapon)), an
//! optional **armor** item and up to [`UTILITY_SLOTS`] **utility** items.
//! Items change stats and grant abilities (a frag grenade item grants the
//! `frag_grenade` ability with its own number of uses). Everything is data:
//! modules register [`ItemDef`]s and named [`LoadoutDef`] presets, squads and
//! missions pick presets or individual pieces.
//!
//! How a unit's gear is decided (later wins, see [`Ruleset::gear_for`]):
//! 1. the class's `loadout` preset (if any), else just its `weapon`;
//! 2. the squad member's / mission unit's `loadout` preset;
//! 3. explicit `weapon` / `armor` / `items` overrides.
//!
//! [`Ruleset::gear_for`]: super::Ruleset::gear_for

use crate::content::AbilityId;

/// How many utility items a unit can carry.
pub const UTILITY_SLOTS: usize = 2;

/// How many augments an augmented soldier can have.
pub const AUGMENT_SLOTS: usize = 3;

/// Where an item is worn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum ItemSlot {
    Armor,
    #[default]
    Utility,
    /// Cybernetic augments (augmented soldiers): up to [`AUGMENT_SLOTS`],
    /// each a trade-off - or something stranger.
    Augment,
}

/// Stat changes from an item (added to the class's stats).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct StatModifiers {
    pub max_hp: i32,
    /// Movement points (2 per tile).
    pub mobility: i32,
    pub aim: i32,
    pub defense: i32,
    pub sight: f32,
    /// Sight range bonus in percent of the total (a scanner: 50 = half as far again).
    pub sight_percent: i32,
    /// Fear resistance (see `MoraleRules`).
    pub bravery: i32,
    /// Extra spare magazines (ammo pouches).
    pub magazines: i32,
}

/// An equippable item.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub slot: ItemSlot,
    pub stats: StatModifiers,
    /// Abilities the wearer gains.
    pub abilities: Vec<AbilityId>,
    /// Uses this item adds to each granted ability that has limited charges
    /// (`None` = the ability's own `charges`). Two grenade items = two uses.
    pub charges: Option<u32>,
    /// Classes allowed to use it (empty = everyone).
    pub classes: Vec<String>,
    /// Model id drawn on the wearer (e.g. a glTF helmet or pouch). Without
    /// one, the frontend draws built-in gear for known items.
    pub model: Option<String>,
    /// Socket the model attaches to (default: `chest` for armour, `hip_r`
    /// for utility items).
    pub socket: Option<String>,
    /// Scripts the wearer runs, like a class's `hooks` (e.g. bursting into
    /// fire on `death`).
    pub hooks: Vec<crate::unit::ClassHook>,
    /// The wearer runs on **heat** instead of health (like a robot frame).
    pub heat: Option<crate::unit::HeatRules>,
    /// The wearer also counts as these classes for gear (`["frame_light"]`:
    /// can carry a Strider's weapons and items).
    pub gear_as: Vec<String>,
    /// The wearer builds rage (see `RageRules`).
    pub rage: Option<crate::unit::RageRules>,
    /// The wearer feels no fear and no insanity.
    pub fearless: bool,
}

/// A set of equipment. Empty fields mean "keep what you'd otherwise have".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Loadout {
    pub weapon: Option<String>,
    /// `Some("")` = explicitly no armor (overrides a preset's).
    pub armor: Option<String>,
    /// Utility item ids (an id may repeat: two grenades). `[""]` = explicitly
    /// no items.
    pub items: Vec<String>,
    /// Augment ids (augmented soldiers). `[""]` = explicitly none.
    pub augments: Vec<String>,
}

impl Loadout {
    /// `self` with every field `over` sets replaced.
    pub fn overridden_by(&self, over: &Loadout) -> Loadout {
        Loadout {
            weapon: over.weapon.clone().or_else(|| self.weapon.clone()),
            armor: over.armor.clone().or_else(|| self.armor.clone()),
            items: if over.items.is_empty() { self.items.clone() } else { over.items.clone() },
            augments: if over.augments.is_empty() { self.augments.clone() } else { over.augments.clone() },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.weapon.is_none() && self.armor.is_none() && self.items.is_empty() && self.augments.is_empty()
    }
}

/// A named, reusable loadout ("Breacher", "Medic"...).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct LoadoutDef {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Classes it's meant for (empty = any). Shown as a hint; the pieces'
    /// own class restrictions are what's enforced.
    pub classes: Vec<String>,
    pub gear: Loadout,
}

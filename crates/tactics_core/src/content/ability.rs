//! Ability ("move") definitions.
//!
//! Every thing a unit can do besides walking — shooting, overwatch, hunkering,
//! reloading, throwing a grenade, healing — is an [`AbilityDef`]. Abilities are
//! **data**: the battle engine reads these numbers, so adding a new move means
//! adding a new definition, not new rules code.
//!
//! An ability is made of four independent parts:
//!
//! | part | question it answers | type |
//! |---|---|---|
//! | cost       | what does it consume?          | [`ApCost`], `ammo_cost`, `cooldown`, `charges` |
//! | targeting  | what can it be aimed at?       | [`Targeting`], [`AbilityRange`], `requires_los` |
//! | effect     | what does it do?               | [`AbilityEffect`] |
//! | attack     | how accurate / deadly is it?   | [`AttackProfile`] (inside attack effects) |
//!
//! Every field has a sensible default, so a data file only lists what differs:
//!
//! ```ron
//! (
//!     id: "aimed_shot",
//!     name: "Aimed Shot",
//!     cost: Points(2),
//!     effect: Attack((
//!         accuracy_bonus: 15,
//!         half_cover_penalty: 15,
//!         full_cover_penalty: 30,
//!     )),
//! )
//! ```

use std::fmt;

/// Stable string identifier of an ability, e.g. `"shoot"` or `"frag_grenade"`.
///
/// Strings (rather than enums) let data files and third-party modules add
/// abilities without touching this crate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct AbilityId(pub String);

impl AbilityId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AbilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for AbilityId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// How many action points an ability consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum ApCost {
    /// Costs exactly this many AP (the unit may keep acting afterwards).
    Points(u8),
    /// Needs at least 1 AP and consumes all remaining AP (XCOM-style "ends turn").
    #[default]
    EndsTurn,
}

impl ApCost {
    /// Minimum AP the unit must have to use the ability.
    pub fn required(self) -> u8 {
        match self {
            ApCost::Points(n) => n.max(1),
            ApCost::EndsTurn => 1,
        }
    }
}

/// What an ability may be aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Targeting {
    /// Affects the user only (overwatch, hunker, reload).
    #[default]
    SelfOnly,
    /// A living enemy unit.
    Enemy,
    /// A living friendly unit, the user included.
    Ally,
    /// Any tile on the map (area attacks, smoke, deployables...).
    Tile,
}

/// Maximum distance (tiles) between the user and the target.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum AbilityRange {
    /// Use the equipped weapon's `max_range` (also capped by the unit's sight).
    #[default]
    Weapon,
    /// A fixed distance, e.g. a grenade throw or a melee strike.
    Fixed(f32),
}

/// Where the damage numbers of an attack come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum DamageSource {
    /// The equipped weapon's `damage_min..=damage_max`.
    #[default]
    Weapon,
    /// Fixed numbers independent of the weapon (grenades, melee...).
    Fixed { min: i32, max: i32 },
}

/// Accuracy and damage rules of one attack. This is where "how good is this
/// move" lives: base accuracy, cover penalties, crit and damage modifiers.
///
/// Hit chance formula (see [`crate::combat`]):
/// ```text
/// hit = (base_accuracy or attacker.aim) + accuracy_bonus
///       - target.defense - cover penalty - target's defend bonus
///       ± range modifier (if range_falloff)
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AttackProfile {
    /// Fixed base accuracy in percent. `None` = use the attacker's `aim` stat.
    pub base_accuracy: Option<i32>,
    /// Added to the base accuracy (negative for snap/reaction shots).
    pub accuracy_bonus: i32,
    /// Hit-chance penalty when the target is in half cover.
    pub half_cover_penalty: i32,
    /// Hit-chance penalty when the target is in full cover.
    pub full_cover_penalty: i32,
    /// Apply the weapon's optimal-range falloff / close-range bonus.
    pub range_falloff: bool,
    /// `false` = always hits (grenades). Cover still matters if penalties are set.
    pub can_miss: bool,
    /// Added to the weapon's crit chance.
    pub crit_bonus: i32,
    /// Extra crit chance against flanked targets.
    pub flank_crit_bonus: i32,
    /// Base damage numbers.
    pub damage: DamageSource,
    /// Flat damage added on every hit.
    pub damage_bonus: i32,
    /// `0` = single target. `> 0` = hits every unit within this many tiles of
    /// the target point (friendly fire included). Cover is then evaluated from
    /// the blast centre.
    pub area_radius: f32,
}

impl Default for AttackProfile {
    /// The classic XCOM rifle shot: unit aim, 20/40 cover penalties, range
    /// falloff, +40 crit on flanks, weapon damage.
    fn default() -> Self {
        Self {
            base_accuracy: None,
            accuracy_bonus: 0,
            half_cover_penalty: 20,
            full_cover_penalty: 40,
            range_falloff: true,
            can_miss: true,
            crit_bonus: 0,
            flank_crit_bonus: 40,
            damage: DamageSource::Weapon,
            damage_bonus: 0,
            area_radius: 0.0,
        }
    }
}

/// What happens when the ability resolves.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum AbilityEffect {
    /// Roll an attack against the target (or everyone in `area_radius`).
    Attack(AttackProfile),
    /// Arm a reaction shot using this profile against the first enemy that
    /// moves into range and sight during the opponent's turn.
    Overwatch(AttackProfile),
    /// Raise the user's defence until its next turn (hunker down).
    Defend { defense_bonus: i32 },
    /// Refill the user's magazine.
    Reload,
    /// Restore hit points to the target (never above max).
    Heal { amount: i32 },
}

impl Default for AbilityEffect {
    fn default() -> Self {
        AbilityEffect::Attack(AttackProfile::default())
    }
}

impl AbilityEffect {
    /// The attack profile, for effects that roll attacks.
    pub fn attack_profile(&self) -> Option<&AttackProfile> {
        match self {
            AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) => Some(p),
            _ => None,
        }
    }

    pub fn attack_profile_mut(&mut self) -> Option<&mut AttackProfile> {
        match self {
            AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) => Some(p),
            _ => None,
        }
    }

    /// Short label for UIs and logs.
    pub fn kind_name(&self) -> &'static str {
        match self {
            AbilityEffect::Attack(p) if p.area_radius > 0.0 => "Area attack",
            AbilityEffect::Attack(_) => "Attack",
            AbilityEffect::Overwatch(_) => "Overwatch",
            AbilityEffect::Defend { .. } => "Defend",
            AbilityEffect::Reload => "Reload",
            AbilityEffect::Heal { .. } => "Heal",
        }
    }
}

/// A complete ability ("move") definition.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AbilityDef {
    /// Unique id. Registering another ability with the same id replaces it,
    /// which is how modules tweak existing moves.
    pub id: AbilityId,
    /// Display name.
    pub name: String,
    /// Tooltip text.
    pub description: String,
    /// Action point cost.
    pub cost: ApCost,
    /// Rounds of ammunition consumed (0 for non-weapon abilities).
    pub ammo_cost: i32,
    /// Own turns before the ability can be used again (0 = no cooldown).
    pub cooldown: u32,
    /// Uses per battle (`None` = unlimited).
    pub charges: Option<u32>,
    /// What it can be aimed at.
    pub targeting: Targeting,
    /// How far.
    pub range: AbilityRange,
    /// Must the user see the target (tile)?
    pub requires_los: bool,
    /// What it does.
    pub effect: AbilityEffect,
}

impl Default for AbilityDef {
    fn default() -> Self {
        Self {
            id: AbilityId::default(),
            name: String::new(),
            description: String::new(),
            cost: ApCost::default(),
            ammo_cost: 0,
            cooldown: 0,
            charges: None,
            targeting: Targeting::default(),
            range: AbilityRange::default(),
            // Most abilities need to see their target; opt out explicitly.
            requires_los: true,
            effect: AbilityEffect::default(),
        }
    }
}

impl AbilityDef {
    /// Starts a definition with the given id and name; chain the `with_*`
    /// helpers or set fields directly.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: AbilityId::new(id),
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn with_description(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    pub fn with_cost(mut self, cost: ApCost) -> Self {
        self.cost = cost;
        self
    }

    pub fn with_ammo_cost(mut self, ammo: i32) -> Self {
        self.ammo_cost = ammo;
        self
    }

    pub fn with_cooldown(mut self, turns: u32) -> Self {
        self.cooldown = turns;
        self
    }

    pub fn with_charges(mut self, charges: u32) -> Self {
        self.charges = Some(charges);
        self
    }

    pub fn with_targeting(mut self, targeting: Targeting, range: AbilityRange) -> Self {
        self.targeting = targeting;
        self.range = range;
        self
    }

    pub fn with_los(mut self, requires_los: bool) -> Self {
        self.requires_los = requires_los;
        self
    }

    pub fn with_effect(mut self, effect: AbilityEffect) -> Self {
        self.effect = effect;
        self
    }

    /// Is this a single-target attack on enemies (a "primary" shot)?
    pub fn is_direct_attack(&self) -> bool {
        self.targeting == Targeting::Enemy
            && matches!(&self.effect, AbilityEffect::Attack(p) if p.area_radius <= 0.0)
    }
}

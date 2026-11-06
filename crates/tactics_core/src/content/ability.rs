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
    /// Rounds fired per use (overrides the weapon's `burst`; each round of a
    /// shotgun is still `pellets` pellets). Single-target attacks that can
    /// miss are **ballistic**: every round is traced as a ray (see
    /// [`crate::battle`] `ballistics`).
    pub rounds: Option<u32>,
    /// A strike in arm's reach (claws, bites, blades): rolled, not traced,
    /// so nothing in between can be hit.
    pub melee: bool,
    /// Area attacks: fragments thrown out of the blast in every direction,
    /// each traced like a bullet (1 damage; friend or foe; walls stop them).
    pub shrapnel: u32,
    /// `> 0` = a **cone** this many degrees wide from the user towards the
    /// target tile, reaching as far as that tile (a flamethrower). Hits
    /// everyone in it the user can see, friend or foe; cover counts from
    /// the user.
    pub cone: f32,
    /// Sets the area (blast or cone) **on fire** for this many rounds
    /// (see `CombatRules::fire_damage`). 0 = no fire.
    pub ignite: u32,
    /// Area attacks: flies **straight** from the user (a rocket) and
    /// detonates at the first wall or unit in the way, not over them.
    pub direct: bool,
    /// Blasts: a **square** reaching `area_radius` tiles out from the centre
    /// on every side (radius 2 = 5x5) instead of a circle.
    pub square: bool,
    /// Blasts: goes off this many of the thrower's turns later (at the start
    /// of its turn), not at once. 0 = immediately.
    pub fuse: u32,
    /// A **shock** (taser): `Light` and `Medium` machines hit by it are
    /// shorted out (destroyed) outright; `Heavy` machines take this much
    /// damage instead of the normal damage. Anyone else takes the normal
    /// damage. `None` = not a shock attack.
    pub shock: Option<i32>,
    /// Stuns flesh-and-blood targets it hits for this many of their turns
    /// (they lose them). Machines shrug it off. 0 = no stun.
    pub stun: u32,
    /// A **slam**: aimed at an adjacent enemy, but hits everyone within
    /// `area_radius` of the user (friend or foe, the user excepted) and
    /// damages props and walls around it.
    pub slam: bool,
    /// Area attacks: fills the area with **gas** for this many rounds. Living
    /// units in gas can't aim (their attacks that can miss have no chance to
    /// hit; bullets fly wild) and stagger to a random tile when they move.
    /// Machines don't breathe. 0 = no gas.
    pub gas: u32,
    /// **Blind fire**: aimed at a tile, not a unit you can see. The shots go
    /// at whoever is there (if anyone); previews show nothing for units the
    /// user's team can't see, so it gives nothing away (or the AI any cheats).
    pub blind: bool,
}

impl AttackProfile {
    /// Traced as bullets (single target, can miss, not melee) rather than rolled.
    pub fn is_ballistic(&self) -> bool {
        !self.is_area() && self.can_miss && !self.melee
    }

    /// Hits an area (a blast or a cone) rather than one target.
    pub fn is_area(&self) -> bool {
        self.area_radius > 0.0 || self.cone > 0.0
    }
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
            rounds: None,
            melee: false,
            shrapnel: 0,
            cone: 0.0,
            ignite: 0,
            direct: false,
            square: false,
            fuse: 0,
            shock: None,
            stun: 0,
            slam: false,
            gas: 0,
            blind: false,
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
    /// Suppressive fire: a long, inaccurate volley at the target (bullets can
    /// still hit anything in the way). The target is **suppressed** until the
    /// user's next turn: worse aim, more fear, and if it moves the user fires
    /// at it again.
    Suppress(AttackProfile),
    /// Robot frames: power down for `turns` of the user's own turns (no
    /// actions), then reboot with all heat vented. Any damage taken while
    /// shut down destroys the unit.
    Shutdown { turns: u32 },
    /// Shed `amount` heat (coolant flush). Only for units that run on heat.
    Cool { amount: i32 },
    /// Overclock: for the rest of the user's turn its next turn-ending
    /// attack costs only 1 AP (an extra shot), and all heat it generates is
    /// doubled.
    Overclock,
    /// Jump/boost straight to the target tile, up to `reach` times the
    /// unit's normal move (tiles per action point). `over_walls`: flies over
    /// anything (jump pack); otherwise walls, holes, units and steep steps
    /// on the line stop it (boost thrusters). Overwatch can fire at the
    /// landing; landing in fire burns.
    Leap { reach: f32, over_walls: bool },
    /// Do nothing and end the turn (robots idling to cool off: no heat).
    Idle,
    /// An adrenaline shot: the user loses half its remaining health (rounded
    /// down, so nothing at 1 HP) and gains this many action points.
    Adrenaline { action_points: u8 },
    /// **Mind control** a living enemy (not a machine): it fights for the
    /// user's side for `turns` of the user's turns (acting at once if it's
    /// their turn), and its insanity rises by `insanity`. Freed early if the
    /// user dies.
    MindControl { turns: u32, insanity: i32 },
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
            AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) | AbilityEffect::Suppress(p) => Some(p),
            _ => None,
        }
    }

    pub fn attack_profile_mut(&mut self) -> Option<&mut AttackProfile> {
        match self {
            AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) | AbilityEffect::Suppress(p) => Some(p),
            _ => None,
        }
    }

    /// Short label for UIs and logs.
    pub fn kind_name(&self) -> &'static str {
        match self {
            AbilityEffect::Attack(p) if p.cone > 0.0 => "Cone attack",
            AbilityEffect::Attack(p) if p.area_radius > 0.0 => "Area attack",
            AbilityEffect::Attack(_) => "Attack",
            AbilityEffect::Overwatch(_) => "Overwatch",
            AbilityEffect::Defend { .. } => "Defend",
            AbilityEffect::Reload => "Reload",
            AbilityEffect::Heal { .. } => "Heal",
            AbilityEffect::Suppress(_) => "Suppress",
            AbilityEffect::Shutdown { .. } => "Shutdown",
            AbilityEffect::Cool { .. } => "Cool down",
            AbilityEffect::Overclock => "Overclock",
            AbilityEffect::Leap { .. } => "Leap",
            AbilityEffect::Idle => "Idle",
            AbilityEffect::Adrenaline { .. } => "Adrenaline",
            AbilityEffect::MindControl { .. } => "Mind control",
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
    /// Heat it adds to a user that runs on heat, on top of the weapon's
    /// heat for weapon attacks (see [`HeatRules`](crate::unit::HeatRules)).
    pub heat: i32,
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
            heat: 0,
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

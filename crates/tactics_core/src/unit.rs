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

/// What a weapon looks like when held (the frontend builds a model for each;
/// a weapon's `model` replaces it with a custom one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum WeaponLook {
    #[default]
    Rifle,
    Shotgun,
    Sniper,
    Smg,
    Lmg,
    Pistol,
    Plasma,
    /// A long emitter with glowing coils; fires beams.
    Laser,
    /// Rotary multi-barrel gun.
    Minigun,
    /// A heavy, short, fat-barrelled cannon (fires beams).
    Cannon,
    /// Fuel tank and a long nozzle (flamethrowers).
    Flamer,
    /// Nothing held (built-in weapons of drones, claws...).
    None,
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
    /// Rounds fired per shot action (attacks may override with `rounds`).
    pub burst: u32,
    /// Projectiles per round (shotgun pellets).
    pub pellets: u32,
    /// Mechanical spread of each projectile, degrees (adds to aim error).
    pub spread: f32,
    /// Its shots can't be stopped by the target's cover (no cover penalty,
    /// no blocked rounds): e.g. a shotgun blast that wraps around a wall.
    /// Walls and units in the way still stop projectiles.
    pub ignores_cover: bool,
    /// Closer than this (tiles) the weapon is awkward (a scoped rifle):
    /// no close-range bonus, and `close_penalty` aim lost per tile inside it.
    pub min_range: f32,
    /// Aim lost per tile closer than `min_range`.
    pub close_penalty: i32,
    /// Spare magazines carried into battle (each Reload uses one).
    pub magazines: u32,
    /// Spare magazines left (instance state: set from `magazines` on spawn).
    #[cfg_attr(feature = "data", serde(skip))]
    pub spare: u32,
    /// How it's drawn in the hand.
    pub look: WeaponLook,
    /// Model id (a `ModelDef`, e.g. a glTF) drawn at the `weapon` socket
    /// instead of the built-in look.
    pub model: Option<String>,
    /// Heat each attack with it adds to a unit that runs on heat (see
    /// [`HeatRules`]); 0 = the unit's `per_shot`. Ignored by everyone else.
    pub heat: i32,
    /// `> 0`: a **flame weapon**: its attacks (Shoot, overwatch) sweep a
    /// cone this many degrees wide out to the target instead of firing
    /// bullets, never miss, ignore cover and burn everyone in it.
    pub cone: f32,
    /// Rounds of fire its cone leaves behind (flame weapons).
    pub ignite: u32,
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
            burst: 1,
            pellets: 1,
            spread: 0.6,
            ignores_cover: false,
            min_range: 0.0,
            close_penalty: 0,
            magazines: 3,
            spare: 0,
            look: WeaponLook::Rifle,
            model: None,
            heat: 0,
            cone: 0.0,
            ignite: 0,
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

/// The physical size of a unit for traced shots, in metres above the ground
/// of its tile. Flying units (drones) hover: their hitbox, muzzle and the
/// point enemies aim at all sit higher, so they shoot over and are shot over
/// low cover and the heads of units in between.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Body {
    /// Gap between the ground and the bottom of the hitbox (0 = standing).
    pub hover: f32,
    /// Height of the hitbox.
    pub height: f32,
    /// Gun height above the ground, if not the usual 72% up the hitbox
    /// (e.g. a drone's gun slung under its body).
    pub muzzle: Option<f32>,
}

impl Default for Body {
    fn default() -> Self {
        Self { hover: 0.0, height: 1.25, muzzle: None }
    }
}

impl Body {
    /// Bottom and top of the hitbox above the tile's ground.
    pub fn span(&self) -> (f32, f32) {
        (self.hover.max(0.0), self.hover.max(0.0) + self.height.max(0.1))
    }
    /// Where shots come from (gun height).
    pub fn muzzle(&self) -> f32 {
        self.muzzle.unwrap_or(self.hover.max(0.0) + 0.72 * self.height.max(0.1))
    }
    /// Where enemies aim (centre of mass).
    pub fn aim_point(&self) -> f32 {
        self.hover.max(0.0) + 0.56 * self.height.max(0.1)
    }
    /// Flying: high enough that cover of `cover_height` metres is beneath it.
    pub fn above(&self, cover_height: f32) -> bool {
        self.hover >= cover_height
    }
}

/// **Heat** instead of health (robot frames). The unit's `max_hp` is its heat
/// limit and its HP is the headroom left: `heat = max_hp - hp`. Damage adds
/// heat; so do its own attacks (the weapon's `heat`, or `per_shot`) and
/// moving (`per_move` per action point). At the start of each of its turns
/// it sheds `cooling`. Reaching the limit destroys it. The Shutdown ability
/// vents everything, at a price (see `AbilityEffect::Shutdown`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct HeatRules {
    /// Heat shed at the start of each of the unit's turns.
    pub cooling: i32,
    /// Heat per action point spent moving.
    pub per_move: i32,
    /// Heat per attack with a weapon that has no `heat` of its own.
    pub per_shot: i32,
}

/// A script every unit of a class carries, wherever it is spawned (missions,
/// generated maps, `spawn_unit`). Same language and API as mission unit
/// scripts; `unit` is the unit itself. Events: `turn_start` (its team's
/// turn), `damaged` (`amount` = damage), `death`, `spotted`, `enter`.
///
/// ```ron
/// hooks: [(on: "turn_start", code: "add_stat(unit, \"defense\", -5);")]
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ClassHook {
    /// Event name (`turn_start`, `damaged`, `death`, `spotted`, `enter`).
    pub on: String,
    /// Run at most once per unit.
    pub once: bool,
    /// Rhai source code.
    pub code: String,
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
    /// Mindless swarm (zombies): the AI skips tactics and simply closes in
    /// on the nearest enemy and strikes. Cheap enough for hundreds of units.
    pub horde: bool,
    /// While dormant, gunfire within this many tiles wakes it (0 = deaf).
    pub hearing: f32,
    /// Hitbox and hover height (see [`Body`]); e.g. drones
    /// `body: (hover: 0.8, height: 0.6)`.
    pub body: Body,
    /// Scripts every unit of this class runs (see [`ClassHook`]).
    pub hooks: Vec<ClassHook>,
    /// Default loadout preset (see [`LoadoutDef`](crate::content::LoadoutDef)).
    pub loadout: Option<String>,
    /// Runs on heat instead of health (see [`HeatRules`]).
    pub heat: Option<HeatRules>,
    /// Too big to hide behind low cover: half cover gives it nothing (full
    /// cover still does). Robot frames.
    pub no_half_cover: bool,
    /// Tiles it takes up: `size` x `size` (0/1 = one tile, 2 = 2x2, 3 =
    /// 3x3: big monsters). The unit's `pos` is the footprint's corner with
    /// the lowest x and y.
    pub size: u8,
    /// May only use weapons and items that name this class in their
    /// `classes` (gear open to everyone is off limits): robot frames have
    /// their own arsenal.
    pub restricted_gear: bool,
    /// A robot, and how big (`None` = flesh and blood). Shock weapons
    /// (tasers) short out `Light` and `Medium` machines outright and hurt
    /// `Heavy` ones badly.
    pub machine: Machine,
}

/// What kind of machine a unit is, if any (see [`UnitClass::machine`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Machine {
    /// Not a machine.
    #[default]
    None,
    /// Drones, sentry bots.
    Light,
    /// Man-sized robots (a Strider frame).
    Medium,
    /// Big walkers (a Bastion frame, wardens).
    Heavy,
}

/// Temporary conditions on a unit. A future "status effect" module type will
/// generalise this into a list of timed effects (see `MODULES.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitStatus {
    /// Armed overwatch ability: fires at the first enemy that moves in sight.
    pub overwatch: Option<AbilityId>,
    /// Extra defence from a Defend ability (hunker) until the unit's next turn.
    pub defense_bonus: i32,
    /// Pinned by suppressive fire: (suppressor, its ability). Aim penalty;
    /// moving draws fire. Lasts until the suppressor's next turn.
    pub suppressed_by: Option<(UnitId, AbilityId)>,
    /// Shut down (Shutdown ability): counts down at the start of each of
    /// the unit's turns; offline (no AP) while above 1, and on reaching 0 it
    /// reboots with all heat vented. Any damage while shut down destroys it.
    pub shutdown: u32,
    /// Overclocked this turn: all heat it generates is doubled.
    pub overclocked: bool,
    /// Overclock's extra shot: the next turn-ending attack costs 1 AP.
    pub bonus_shot: bool,
    /// Knocked out (script `stun`): the unit's team's next this-many turns
    /// give it no action points.
    pub stunned: u32,
    /// Standing in gas (see `Battle::gas`): can't aim (hit chance 0, shots
    /// fly wild) and moves stagger to random tiles. Never set on machines.
    pub gassed: bool,
}

/// Behaviour state of an AI-controlled unit (mission mechanics).
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// Copied from the class: mindless swarm behaviour (see [`UnitClass::horde`]).
    pub horde: bool,
    /// Copied from the class: gunfire within this many tiles wakes it.
    pub hearing: f32,
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
    /// Copied from the class: hitbox and hover height.
    pub body: Body,
    /// Copied from the class: runs on heat instead of health.
    pub heat: Option<HeatRules>,
    /// Copied from the class: half cover doesn't protect it.
    pub no_half_cover: bool,
    /// Copied from the class: footprint `size` x `size` from `pos`.
    pub size: u8,
    /// Copied from the class: a robot, and how big.
    pub machine: Machine,
    /// What the unit carries (resolved at spawn).
    pub gear: crate::content::Loadout,
    /// Looks (`None` = derived from the name by the frontend).
    pub appearance: Option<crate::content::Appearance>,
}

impl Unit {
    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    /// Can this unit still do something this turn?
    pub fn can_act(&self) -> bool {
        self.is_alive() && self.action_points > 0
    }

    /// Footprint side in tiles (at least 1).
    pub fn side(&self) -> i32 {
        self.size.max(1) as i32
    }

    /// Every tile the unit stands on.
    pub fn tiles(&self) -> impl Iterator<Item = GridPos> + '_ {
        let s = self.side();
        (0..s).flat_map(move |dy| (0..s).map(move |dx| GridPos::new(self.pos.x + dx, self.pos.y + dy)))
    }

    /// Does the unit stand on `p`?
    pub fn occupies(&self, p: GridPos) -> bool {
        let s = self.side();
        p.x >= self.pos.x && p.x < self.pos.x + s && p.y >= self.pos.y && p.y < self.pos.y + s
    }

    /// The tile of its footprint closest to `p`.
    pub fn nearest_tile(&self, p: GridPos) -> GridPos {
        let s = self.side();
        GridPos::new(p.x.clamp(self.pos.x, self.pos.x + s - 1), p.y.clamp(self.pos.y, self.pos.y + s - 1))
    }

    /// Centre of the footprint in grid coordinates.
    pub fn center(&self) -> (f32, f32) {
        let k = (self.side() - 1) as f32 / 2.0;
        (self.pos.x as f32 + k, self.pos.y as f32 + k)
    }

    /// Current heat (0 for units that don't run on heat).
    pub fn heat(&self) -> i32 {
        if self.heat.is_some() { self.stats.max_hp - self.hp.max(0) } else { 0 }
    }

    /// Shut down (offline, see [`UnitStatus::shutdown`]).
    pub fn is_shut_down(&self) -> bool {
        self.status.shutdown > 0
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

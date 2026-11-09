//! The battle: authoritative game state and the rules that change it.
//!
//! [`Battle`] owns the map, the [`Ruleset`] (all content), every unit, the
//! turn order and the RNG. Its public surface is intentionally small:
//!
//! * **Queries** (`unit`, `movement_range`, `preview`, `visible_enemies`, ...)
//!   are read-only and cheap enough to call every frame from a UI.
//! * [`Battle::validate`] says whether an [`Action`] is legal without changing
//!   anything.
//! * [`Battle::apply`] performs an action and returns the resulting
//!   [`GameEvent`]s. It is the **only** way to mutate the battle state
//!   (content can be tuned live through [`Battle::rules_mut`]).
//!
//! Turn structure (XCOM-like):
//! 1. All units of the active team get `rules.combat.action_points` AP; their
//!    overwatch/defend states expire and ability cooldowns tick down.
//! 2. Units act in any order: move (1 AP per `mobility` worth of movement) or
//!    use abilities (cost defined per ability).
//! 3. The turn ends on [`Action::EndTurn`] or automatically when no unit of the
//!    active team can act any more.
//!
//! ## How an ability resolves
//! [`Action::UseAbility`] → look up the [`AbilityDef`] → check AP, ammo,
//! cooldown, charges and the target (kind, range, line of sight) → emit
//! [`GameEvent::AbilityUsed`] → pay costs → run the [`AbilityEffect`] → spend AP.
//! Adding a new *kind* of effect means adding an `AbilityEffect` variant and
//! one match arm in [`Battle::run_effect`].

use crate::action::{Action, ActionError, Target};
use crate::combat::{ShotPreview, preview_attack};
use crate::content::{AbilityDef, AbilityEffect, AbilityId, AbilityRange, AttackProfile, ItemDef, Loadout, Ruleset, Targeting};
use crate::event::{GameEvent, ShotOutcome};
use crate::grid::{GridPos, Map};
use crate::pathfinding::{self, ReachableSet};
use crate::rng::GameRng;
use crate::unit::{Team, Unit, UnitId};
use crate::visibility;

mod ballistics;
#[cfg(test)]
mod big_tests;
#[cfg(test)]
mod melee_tests;
mod fire;
mod gas;
mod symbiote;
mod heat;
mod morale;
mod stealth;
mod world;
pub use stealth::{DoorState, MarkedZone, Stealth};
pub use world::{MAX_SCRIPT_DEPTH, Prop, PropId};

/// Authoritative state of one tactical engagement.
#[derive(Debug, Clone)]
pub struct Battle {
    map: Map,
    rules: Ruleset,
    /// Indexed by `UnitId.0`. Dead units stay in the list (hp <= 0) so ids remain stable.
    units: Vec<Unit>,
    active_team: Team,
    round: u32,
    rng: GameRng,
    winner: Option<Team>,
    // --- mission layer (see world.rs) ---
    props: Vec<Prop>,
    hooks: Vec<world::ActiveHook>,
    /// Shared script functions from the mission.
    library: String,
    flags: std::collections::BTreeMap<String, i64>,
    /// Enemies the player has already seen (for `spotted` hooks).
    spotted: std::collections::BTreeSet<UnitId>,
    unit_tags: std::collections::BTreeMap<UnitId, String>,
    /// Labels scripts keep over units (`mark_unit`), e.g. a boss.
    unit_marks: std::collections::BTreeMap<UnitId, String>,
    /// Killing every enemy wins (otherwise only scripts can win).
    eliminate_to_win: bool,
    mission_id: Option<String>,
    script_depth: u32,
    // --- mission mechanics (see stealth.rs) ---
    stealth: Stealth,
    /// Enemies are hidden from the player unless seen (presentation hint;
    /// the rules already work from each side's own sight where it matters).
    fog: bool,
    /// Where each team's AI heads while no enemy is in sight (index: Team).
    ai_goals: [Option<GridPos>; 2],
    objective: String,
    /// Zones drawn on the board.
    zones: Vec<stealth::MarkedZone>,
    /// Damage taken so far by walls and other terrain cover (blasts).
    wall_damage: std::collections::HashMap<GridPos, i32>,
    /// Bumped whenever terrain changes (renderers redraw maps on change).
    map_version: u32,
    /// The mission's background (for renderers).
    sky: crate::mission::SkyDef,
    /// Burning tiles: rounds of fire left (see fire.rs).
    fires: std::collections::BTreeMap<GridPos, u32>,
    /// Charges waiting to go off (delayed grenades, see fire.rs).
    charges_armed: Vec<fire::ArmedCharge>,
    /// Gassed tiles: rounds of gas left (see gas.rs).
    gas: std::collections::BTreeMap<GridPos, u32>,
    /// Symbiote vines on the floor (see symbiote.rs).
    vines: std::collections::BTreeSet<GridPos>,
    /// Chance (percent) per round for vines to spread to each free side.
    vine_growth: u32,
}

impl Battle {
    /// Creates an empty battle on `map` using `rules`. The player team moves first.
    pub fn new(map: Map, rules: Ruleset, seed: u64) -> Self {
        Self {
            map,
            rules,
            units: Vec::new(),
            active_team: Team::Player,
            round: 1,
            rng: GameRng::new(seed),
            winner: None,
            props: Vec::new(),
            hooks: Vec::new(),
            library: String::new(),
            flags: Default::default(),
            spotted: Default::default(),
            unit_tags: Default::default(),
            unit_marks: Default::default(),
            eliminate_to_win: true,
            mission_id: None,
            script_depth: 0,
            stealth: Stealth::default(),
            fog: false,
            ai_goals: [None, None],
            objective: String::new(),
            zones: Vec::new(),
            wall_damage: Default::default(),
            map_version: 0,
            sky: Default::default(),
            fires: Default::default(),
            charges_armed: Vec::new(),
            gas: Default::default(),
            vines: Default::default(),
            vine_growth: 0,
        }
    }

    /// The mission's sky (`<sky>`; default = the game's own colour).
    pub fn sky(&self) -> &crate::mission::SkyDef {
        &self.sky
    }

    /// Changes every time terrain changes (walls broken, props destroyed).
    pub fn map_version(&self) -> u32 {
        self.map_version
    }

    /// Places a new unit of class `class` with its default gear on the map
    /// and returns its id.
    ///
    /// Fails if the class (or its weapon) isn't registered, or the tile is not
    /// free — data modules can be wrong, so this reports instead of panicking.
    pub fn spawn(&mut self, class: &str, name: impl Into<String>, team: Team, pos: GridPos) -> Result<UnitId, String> {
        self.spawn_equipped(class, name, team, pos, None, &Loadout::default())
    }

    /// Like [`Battle::spawn`] with a loadout: `preset` (a [`LoadoutDef`] id)
    /// and individual `overrides` on top of the class's default gear.
    /// Illegal or unknown gear is an error.
    pub fn spawn_equipped(
        &mut self,
        class: &str,
        name: impl Into<String>,
        team: Team,
        pos: GridPos,
        preset: Option<&str>,
        overrides: &Loadout,
    ) -> Result<UnitId, String> {
        let class = self.rules.class(class).ok_or_else(|| format!("unknown unit class '{class}'"))?.clone();
        if let Some(p) = preset
            && self.rules.loadout(p).is_none()
        {
            return Err(format!("unknown loadout '{p}'"));
        }
        let gear = self.rules.gear_for(&class.id, preset, overrides);
        if let Some(problem) = self.rules.check_gear(&class.id, &gear).into_iter().next() {
            return Err(problem);
        }
        let weapon_id = gear.weapon.clone().unwrap_or_else(|| class.weapon.clone());
        let mut weapon = self
            .rules
            .weapon(&weapon_id)
            .ok_or_else(|| format!("class '{}' uses unknown weapon '{weapon_id}'", class.id))?
            .clone();
        // Every tile of the footprint must be free.
        if !self.footprint_free(pos, class.size.max(1) as i32, None) {
            return Err(format!("no room for a {} ({}x{}) at {pos}", class.name, class.size.max(1), class.size.max(1)));
        }
        weapon.ammo = weapon.clip_size;
        weapon.spare = weapon.magazines;

        // Items: stats, abilities and their uses.
        let mut stats = class.stats;
        let mut abilities = class.abilities.clone();
        let mut charges: std::collections::BTreeMap<AbilityId, u32> = Default::default();
        let items: Vec<ItemDef> =
            gear.armor.iter().chain(gear.items.iter()).chain(gear.augments.iter()).filter_map(|id| self.rules.item(id).cloned()).collect();
        let mut sight_percent = 0;
        for item in &items {
            let m = item.stats;
            sight_percent += m.sight_percent;
            stats.max_hp = (stats.max_hp + m.max_hp).max(1);
            stats.mobility = (stats.mobility as i32 + m.mobility).max(2) as u32;
            stats.aim += m.aim;
            stats.defense += m.defense;
            stats.sight = (stats.sight + m.sight).max(1.0);
            stats.bravery += m.bravery;
            weapon.spare = (weapon.spare as i32 + m.magazines).max(0) as u32;
            for a in &item.abilities {
                let Some(def) = self.rules.ability(a) else { continue };
                if let Some(per_item) = item.charges.or(def.charges)
                    && def.charges.is_some()
                {
                    // Classes that know the ability natively keep its own uses.
                    let base = if class.abilities.contains(a) { def.charges.unwrap_or(0) } else { 0 };
                    *charges.entry(a.clone()).or_insert(base) += per_item;
                }
                if !abilities.contains(a) {
                    abilities.push(a.clone());
                }
            }
        }
        if sight_percent != 0 {
            stats.sight = (stats.sight * (1.0 + sight_percent as f32 / 100.0)).max(1.0);
        }

        let id = UnitId(self.units.len() as u32);
        let unit = Unit {
            id,
            name: name.into(),
            class: class.id.clone(),
            team,
            pos,
            hp: stats.max_hp,
            action_points: if team == self.active_team { self.rules.combat.action_points } else { 0 },
            stats,
            weapon,
            abilities,
            cooldowns: Default::default(),
            charges,
            status: Default::default(),
            model: class.model.clone(),
            ai: crate::unit::UnitAi { horde: class.horde, hearing: class.hearing, ..Default::default() },
            fear: 0,
            fearless: class.fearless,
            body: class.body,
            heat: class.heat,
            no_half_cover: class.no_half_cover,
            size: class.size.max(1),
            machine: class.machine,
            rage_rules: class.rage,
            rage: 0,
            insanity: 0,
            flammable: class.flammable,
            vine_kin: class.vine_kin,
            gear: Loadout { weapon: Some(weapon_id), ..gear },
            appearance: None,
        };
        // Augments that change what the soldier *is*.
        let mut unit = unit;
        for item in &items {
            if item.heat.is_some() {
                unit.heat = item.heat;
            }
            if item.rage.is_some() {
                unit.rage_rules = item.rage;
            }
            unit.fearless |= item.fearless;
        }
        // The class's scripts ride along with every unit of it.
        let hooks: Vec<crate::mission::ScriptHook> = class
            .hooks
            .iter()
            .chain(items.iter().flat_map(|i| i.hooks.iter()))
            .filter_map(|h| {
                crate::mission::HookEvent::parse(&h.on).map(|on| crate::mission::ScriptHook { on, team: None, once: h.once, code: h.code.clone() })
            })
            .collect();
        self.units.push(unit);
        if !hooks.is_empty() {
            self.add_hooks(world::HookOwner::Unit(id), &hooks);
        }
        Ok(id)
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn map(&self) -> &Map {
        &self.map
    }

    /// All content (abilities, weapons, classes...) this battle uses.
    pub fn rules(&self) -> &Ruleset {
        &self.rules
    }

    /// Live content tuning (in-game editor). Changes apply from the next action.
    pub fn rules_mut(&mut self) -> &mut Ruleset {
        &mut self.rules
    }

    pub fn active_team(&self) -> Team {
        self.active_team
    }

    pub fn round(&self) -> u32 {
        self.round
    }

    /// The winning team, once the battle is decided.
    pub fn winner(&self) -> Option<Team> {
        self.winner
    }

    pub fn is_over(&self) -> bool {
        self.winner.is_some()
    }

    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        self.units.get(id.0 as usize)
    }

    /// Changes which model draws a unit (presentation only; no effect on the
    /// rules). Used by the Character Lab to preview models.
    pub fn set_unit_model(&mut self, id: UnitId, model: impl Into<String>) {
        if let Some(u) = self.units.get_mut(id.0 as usize) {
            u.model = model.into();
        }
    }

    /// Every unit ever spawned, dead ones included.
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// Sets how a unit looks (presentation only; saved soldiers carry theirs).
    pub fn set_unit_appearance(&mut self, id: UnitId, appearance: crate::content::Appearance) {
        if let Some(u) = self.units.get_mut(id.0 as usize) {
            u.appearance = Some(appearance);
        }
    }

    /// Living units of a team.
    pub fn living(&self, team: Team) -> impl Iterator<Item = &Unit> + '_ {
        self.units.iter().filter(move |u| u.team == team && u.is_alive())
    }

    /// The living unit standing on `pos` (any tile of its footprint), if any.
    pub fn unit_at(&self, pos: GridPos) -> Option<&Unit> {
        self.units.iter().find(|u| u.is_alive() && u.occupies(pos))
    }

    /// Can a `side` x `side` unit stand with its corner on `pos`: every tile
    /// walkable, free (but for `ignore`) and roughly level?
    pub fn footprint_free(&self, pos: GridPos, side: i32, ignore: Option<UnitId>) -> bool {
        let base = self.map.elevation(pos);
        (0..side).all(|dy| {
            (0..side).all(|dx| {
                let p = GridPos::new(pos.x + dx, pos.y + dy);
                self.map.is_walkable(p)
                    && (self.map.elevation(p) - base).abs() <= 1
                    && self.unit_at(p).is_none_or(|u| Some(u.id) == ignore)
            })
        })
    }

    /// The closest pair of tiles (one of each footprint) between `a` and
    /// `b` that can see each other, or with `need_sight` false just the
    /// closest pair. Big units are seen, shot and reached at their nearest
    /// visible tile.
    pub fn sight_between(&self, a: &Unit, b: &Unit, need_sight: bool) -> Option<(GridPos, GridPos)> {
        if a.side() == 1 && b.side() == 1 {
            return (!need_sight || visibility::has_sight(&self.map, a.pos, b.pos)).then_some((a.pos, b.pos));
        }
        let mut best: Option<(f32, GridPos, GridPos)> = None;
        for from in a.tiles() {
            for to in b.tiles() {
                let d = from.distance(to);
                if best.is_some_and(|(bd, ..)| bd <= d) {
                    continue;
                }
                if !need_sight || visibility::has_sight(&self.map, from, to) {
                    best = Some((d, from, to));
                }
            }
        }
        best.map(|(_, f, t)| (f, t))
    }

    /// Is any tile of `u` in sight of `team`?
    pub fn team_sees_unit(&self, team: Team, u: &Unit) -> bool {
        if u.side() == 1 {
            return self.team_can_see(team, u.pos);
        }
        u.tiles().any(|p| self.team_can_see(team, p))
    }

    /// Every tile `unit` could move to with its remaining action points.
    /// Use [`Battle::move_cost_ap`] to split it into "1 AP" and "dash" zones.
    pub fn movement_range(&self, id: UnitId) -> ReachableSet {
        let Some(unit) = self.unit(id) else {
            return ReachableSet::default();
        };
        let budget = unit.mobility() * unit.action_points as u32;
        let climb = crate::grid::Climb { max_up: unit.stats.climb };
        // Occupied tiles once (not a scan of every unit per neighbour: big
        // maps have hundreds of units).
        let occupied: std::collections::HashSet<GridPos> =
            self.units.iter().filter(|u| u.is_alive() && u.id != id).flat_map(|u| u.tiles().collect::<Vec<_>>()).collect();
        let side = unit.side();
        if side == 1 {
            return pathfinding::reachable(&self.map, unit.pos, budget, climb, |p| !occupied.contains(&p));
        }
        // A big unit: its whole footprint has to fit at every step.
        pathfinding::reachable(&self.map, unit.pos, budget, climb, |p| {
            let base = self.map.elevation(p);
            (0..side).all(|dy| {
                (0..side).all(|dx| {
                    let q = GridPos::new(p.x + dx, p.y + dy);
                    self.map.is_walkable(q) && !occupied.contains(&q) && (self.map.elevation(q) - base).abs() <= 1
                })
            })
        })
    }

    /// How many action points moving `movement_cost` points costs this unit.
    pub fn move_cost_ap(&self, id: UnitId, movement_cost: u32) -> u8 {
        let mobility = self.unit(id).map_or(1, |u| u.mobility().max(1));
        movement_cost.div_ceil(mobility) as u8
    }

    /// Can `viewer` see the tile `pos` (range + line of sight)?
    pub fn can_unit_see(&self, viewer: &Unit, pos: GridPos) -> bool {
        visibility::can_see(&self.map, viewer.pos, pos, viewer.stats.sight)
    }

    /// Is any living member of `team` able to see `pos`? (Squad sight / fog of war.)
    pub fn team_can_see(&self, team: Team, pos: GridPos) -> bool {
        self.living(team).any(|u| self.can_unit_see(u, pos))
    }

    /// Living enemies within weapon range and sight of `id`.
    pub fn visible_enemies(&self, id: UnitId) -> Vec<UnitId> {
        let Some(u) = self.unit(id) else {
            return Vec::new();
        };
        let range = u.weapon.max_range.min(u.stats.sight);
        self.living(u.team.opponent())
            .filter(|t| self.sight_between(u, t, true).is_some_and(|(a, b)| a.distance(b) <= range))
            .map(|t| t.id)
            .collect()
    }

    /// Looks up an ability definition.
    pub fn ability(&self, id: &AbilityId) -> Option<&AbilityDef> {
        self.rules.ability(id)
    }

    /// Maximum reach of `def` when used by `unit`, in tiles.
    pub fn ability_range(&self, unit: &Unit, def: &AbilityDef) -> f32 {
        // Leaps reach a multiple of the unit's normal move.
        if let AbilityEffect::Leap { reach, .. } = def.effect {
            return reach * (unit.stats.mobility / 2) as f32;
        }
        // Blind fire reaches as far as the gun, not as far as the eye.
        if def.effect.attack_profile().is_some_and(|p| p.blind) && def.range == AbilityRange::Weapon {
            return unit.weapon.max_range;
        }
        match def.range {
            AbilityRange::Weapon => unit.weapon.max_range.min(unit.stats.sight),
            AbilityRange::Fixed(r) => r,
        }
    }

    /// Remaining uses of a limited ability (`None` = unlimited).
    pub fn charges_left(&self, unit: &Unit, def: &AbilityDef) -> Option<u32> {
        def.charges.map(|max| unit.charges.get(&def.id).copied().unwrap_or(max))
    }

    /// The first single-target enemy attack the unit knows ("primary shot").
    pub fn primary_attack(&self, unit: &Unit) -> Option<&AbilityDef> {
        unit.abilities
            .iter()
            .filter_map(|a| self.rules.ability(a))
            .find(|d| d.is_direct_attack())
    }

    /// Who an ability would hit and with what odds, without rolling.
    /// Returns one entry per affected unit (several for area attacks).
    pub fn preview(&self, unit: UnitId, ability: &AbilityId, target: Target) -> Result<Vec<(UnitId, ShotPreview)>, ActionError> {
        let user = self.living_unit(unit)?;
        let def = self.rules.ability(ability).ok_or_else(|| ActionError::UnknownAbility(ability.clone()))?;
        let point = self.check_target(user, def, target)?;
        let def = &self.with_weapon(user, def);
        let AbilityEffect::Attack(profile) = &def.effect else {
            return Ok(Vec::new());
        };
        if profile.spray > 0.0 {
            return Ok(Vec::new()); // wild fire: no odds to show
        }
        let point = self.blast_point(user.pos, profile, point);
        Ok(self
            .attack_victims(user, profile, point, target)
            .into_iter()
            // Blind fire: no hints about who's out there.
            .filter(|(victim, _)| !profile.blind || self.team_sees_unit(user.team, &self.units[victim.0 as usize]))
            .map(|(victim, origin)| {
                let v = &self.units[victim.0 as usize];
                (victim, preview_attack(&self.rules.combat, &self.map, origin, user, v, profile))
            })
            .collect())
    }

    /// Does any unit of the active team still have action points?
    pub fn active_team_can_act(&self) -> bool {
        self.living(self.active_team).any(Unit::can_act)
    }

    // ------------------------------------------------------------------
    // Validation
    // ------------------------------------------------------------------

    fn living_unit(&self, id: UnitId) -> Result<&Unit, ActionError> {
        let unit = self.unit(id).ok_or(ActionError::UnknownUnit(id))?;
        if !unit.is_alive() {
            return Err(ActionError::UnitDead(id));
        }
        Ok(unit)
    }

    /// Checks that `id` exists, is alive, belongs to the active team and has AP.
    fn ready_actor(&self, id: UnitId) -> Result<&Unit, ActionError> {
        let unit = self.living_unit(id)?;
        if unit.team != self.active_team {
            return Err(ActionError::NotYourTurn(id));
        }
        if unit.action_points == 0 {
            return Err(ActionError::NoActionPoints(id));
        }
        Ok(unit)
    }

    /// Validates the target of an ability and returns the point it is aimed at.
    fn check_target(&self, user: &Unit, def: &AbilityDef, target: Target) -> Result<GridPos, ActionError> {
        let point = match (def.targeting, target) {
            (Targeting::SelfOnly, Target::None) => return Ok(user.pos),
            (Targeting::SelfOnly, Target::Unit(id)) if id == user.id => return Ok(user.pos),
            (Targeting::Enemy, Target::Unit(id)) | (Targeting::Ally, Target::Unit(id)) => {
                let t = self.living_unit(id).map_err(|_| ActionError::InvalidTarget(id))?;
                let wants_enemy = def.targeting == Targeting::Enemy;
                if (t.team != user.team) != wants_enemy {
                    return Err(ActionError::InvalidTarget(id));
                }
                // Big units (either side): range and sight between the
                // nearest tiles that can see each other.
                if user.side() > 1 || t.side() > 1 {
                    let pair = self.sight_between(user, t, def.requires_los);
                    let Some((from, to)) = pair else { return Err(ActionError::NoLineOfSight) };
                    if from.distance(to) > self.ability_range(user, def) {
                        return Err(ActionError::OutOfRange);
                    }
                    return Ok(t.pos);
                }
                t.pos
            }
            (Targeting::Tile, Target::Tile(p)) if self.map.in_bounds(p) => p,
            _ => return Err(ActionError::WrongTarget),
        };
        if user.pos.distance(point) > self.ability_range(user, def) {
            return Err(ActionError::OutOfRange);
        }
        if def.requires_los && !visibility::has_sight(&self.map, user.pos, point) {
            return Err(ActionError::NoLineOfSight);
        }
        Ok(point)
    }

    /// Returns `Ok(())` if `action` could be applied right now.
    pub fn validate(&self, action: &Action) -> Result<(), ActionError> {
        if self.is_over() {
            return Err(ActionError::BattleOver);
        }
        match action {
            Action::Move { unit, to } => {
                let u = self.ready_actor(*unit)?;
                if *to == u.pos || !self.movement_range(*unit).contains(*to) {
                    return Err(ActionError::Unreachable(*to));
                }
                Ok(())
            }
            Action::UseAbility { unit, ability, target } => {
                let u = self.ready_actor(*unit)?;
                let def = self.rules.ability(ability).ok_or_else(|| ActionError::UnknownAbility(ability.clone()))?;
                if !u.has_ability(ability) {
                    return Err(ActionError::AbilityNotKnown(ability.clone()));
                }
                if u.action_points < def.cost.required() {
                    return Err(ActionError::NoActionPoints(*unit));
                }
                let cd = u.cooldown(ability);
                if cd > 0 {
                    return Err(ActionError::OnCooldown { ability: ability.clone(), turns: cd });
                }
                if self.charges_left(u, def) == Some(0) {
                    return Err(ActionError::NoCharges(ability.clone()));
                }
                if u.weapon.ammo < def.ammo_cost {
                    return Err(ActionError::OutOfAmmo);
                }
                // Effect-specific requirements.
                match &def.effect {
                    AbilityEffect::Overwatch(_) if u.weapon.ammo <= 0 => return Err(ActionError::OutOfAmmo),
                    AbilityEffect::Reload if u.weapon.ammo >= u.weapon.clip_size => {
                        return Err(ActionError::AlreadyFullAmmo);
                    }
                    AbilityEffect::Reload if u.weapon.spare == 0 => return Err(ActionError::OutOfMagazines),
                    AbilityEffect::Suppress(_) if u.weapon.ammo <= 0 => return Err(ActionError::OutOfAmmo),
                    AbilityEffect::Suppress(_) if u.weapon.cone > 0.0 => return Err(ActionError::NotWithThisWeapon),
                    AbilityEffect::Overclock if u.status.overclocked => return Err(ActionError::AlreadyActive),
                    // Flesh and blood only.
                    AbilityEffect::Adrenaline { .. } if u.heat.is_some() => return Err(ActionError::CantHealMachine),
                    // Only minds can be taken: no machines, no monsters, no one already held.
                    AbilityEffect::MindControl { .. } => {
                        let Target::Unit(t) = target else { return Err(ActionError::WrongTarget) };
                        let v = self.living_unit(*t).map_err(|_| ActionError::InvalidTarget(*t))?;
                        if !v.can_go_insane() || v.status.controlled > 0 || v.side() > 1 {
                            return Err(ActionError::InvalidTarget(*t));
                        }
                    }
                    _ => {}
                }
                let point = self.check_target(u, def, *target)?;
                if let AbilityEffect::Heal { .. } = def.effect {
                    let healee = self.unit_at(point).ok_or(ActionError::WrongTarget)?;
                    if healee.heat.is_some() {
                        return Err(ActionError::CantHealMachine);
                    }
                    if healee.hp >= healee.stats.max_hp {
                        return Err(ActionError::NothingToHeal);
                    }
                }
                if let AbilityEffect::Cool { .. } = def.effect
                    && u.heat() <= 0
                {
                    return Err(ActionError::NoHeat);
                }
                if let AbilityEffect::Shutdown { .. } = def.effect
                    && u.heat.is_none()
                {
                    return Err(ActionError::NoHeat);
                }
                if let AbilityEffect::Leap { over_walls, .. } = def.effect {
                    self.check_leap(u, point, over_walls)?;
                }
                Ok(())
            }
            Action::Wait { unit } => self.ready_actor(*unit).map(|_| ()),
            Action::EndTurn => Ok(()),
        }
    }

    // ------------------------------------------------------------------
    // Mutation
    // ------------------------------------------------------------------

    fn unit_mut(&mut self, id: UnitId) -> &mut Unit {
        &mut self.units[id.0 as usize]
    }

    /// Validates and performs `action`, returning everything that happened.
    ///
    /// On error the battle is left untouched.
    pub fn apply(&mut self, action: Action) -> Result<Vec<GameEvent>, ActionError> {
        self.validate(&action)?;
        let mut events = Vec::new();

        match action {
            Action::Move { unit, to } => self.do_move(unit, to, &mut events),
            Action::UseAbility { unit, ability, target } => self.use_ability(unit, ability, target, &mut events),
            Action::Wait { unit } => self.spend_all_ap(unit, &mut events),
            Action::EndTurn => {
                self.end_turn(&mut events);
                self.refresh_gassed();
                return Ok(events);
            }
        }

        self.check_victory(&mut events);
        if !self.is_over() && !self.active_team_can_act() {
            self.end_turn(&mut events);
        }
        self.refresh_gassed();
        Ok(events)
    }

    fn spend_ap(&mut self, id: UnitId, amount: u8, events: &mut Vec<GameEvent>) {
        let u = self.unit_mut(id);
        u.action_points = u.action_points.saturating_sub(amount);
        let remaining = u.action_points;
        events.push(GameEvent::ActionPointsChanged { unit: id, remaining });
    }

    fn spend_all_ap(&mut self, id: UnitId, events: &mut Vec<GameEvent>) {
        let ap = self.unit_mut(id).action_points;
        self.spend_ap(id, ap, events);
    }

    /// Pays costs, runs the effect, spends AP. Assumes `validate` passed.
    fn use_ability(&mut self, unit: UnitId, ability: AbilityId, target: Target, events: &mut Vec<GameEvent>) {
        let def = self.rules.ability(&ability).cloned().expect("validated");
        let point = self.check_target(&self.units[unit.0 as usize], &def, target).expect("validated");
        // A flame weapon turns its shots into cones of fire.
        let def = self.with_weapon(&self.units[unit.0 as usize], &def);
        events.push(GameEvent::AbilityUsed { unit, ability: ability.clone(), target });
        // Attacking gives a concealed squad away.
        if self.stealth.concealed
            && self.units[unit.0 as usize].team == Team::Player
            && matches!(def.effect, AbilityEffect::Attack(_) | AbilityEffect::Suppress(_))
        {
            self.raise_alarm(None, events);
        }

        // Costs.
        let charges = self.charges_left(&self.units[unit.0 as usize], &def);
        // Ballistic weapon fire spends one round per round fired (in the volley).
        let volley_ammo = match &def.effect {
            AbilityEffect::Attack(p) => p.is_ballistic() && def.ammo_cost > 0,
            AbilityEffect::Suppress(_) => true,
            _ => false,
        };
        let u = self.unit_mut(unit);
        if !volley_ammo {
            u.weapon.ammo -= def.ammo_cost;
        }
        if def.cooldown > 0 {
            u.cooldowns.insert(ability.clone(), def.cooldown);
        }
        if let Some(left) = charges {
            u.charges.insert(ability.clone(), left.saturating_sub(1));
        }

        let heat = self.ability_heat(&self.units[unit.0 as usize], &def);
        self.run_effect(unit, &def, point, target, events);
        // Firing (or straining) a machine heats it up.
        if heat > 0 && self.units[unit.0 as usize].is_alive() {
            self.add_heat(unit, heat, events);
        }
        if !self.units[unit.0 as usize].is_alive() {
            return;
        }

        let attack = matches!(def.effect, AbilityEffect::Attack(_) | AbilityEffect::Suppress(_));
        match def.cost {
            // Overclock's extra shot: a turn-ending attack costs just 1 AP.
            crate::content::ApCost::EndsTurn if attack && self.units[unit.0 as usize].status.bonus_shot => {
                self.unit_mut(unit).status.bonus_shot = false;
                self.spend_ap(unit, 1, events);
            }
            crate::content::ApCost::EndsTurn => self.spend_all_ap(unit, events),
            crate::content::ApCost::Points(n) => self.spend_ap(unit, n, events),
        }
    }

    /// Can `u` leap (jump / boost) to `to`? Lands on a free walkable tile;
    /// without `over_walls` the straight line must be clear (no walls,
    /// holes, units or steps higher than it can climb).
    fn check_leap(&self, u: &Unit, to: GridPos, over_walls: bool) -> Result<(), ActionError> {
        if to == u.pos || !self.map.is_walkable(to) || self.unit_at(to).is_some() {
            return Err(ActionError::Unreachable(to));
        }
        if over_walls {
            return Ok(());
        }
        let (dx, dy) = ((to.x - u.pos.x) as f32, (to.y - u.pos.y) as f32);
        let steps = ((dx * dx + dy * dy).sqrt() * 4.0).ceil() as i32;
        let mut last = u.pos;
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            let p = GridPos::new((u.pos.x as f32 + dx * k).round() as i32, (u.pos.y as f32 + dy * k).round() as i32);
            if p == last {
                continue;
            }
            let blocked = !self.map.is_walkable(p)
                || self.unit_at(p).is_some_and(|o| o.id != u.id)
                || self.map.elevation(p) - self.map.elevation(last) > u.stats.climb.max(1);
            if blocked {
                return Err(ActionError::Unreachable(to));
            }
            last = p;
        }
        Ok(())
    }

    /// Executes an ability's effect. **Add new effect kinds here.**
    fn run_effect(&mut self, unit: UnitId, def: &AbilityDef, point: GridPos, target: Target, events: &mut Vec<GameEvent>) {
        match &def.effect {
            AbilityEffect::Attack(profile) if profile.spray > 0.0 => {
                // Half a magazine (whatever's left of it, at least one round).
                let u = &self.units[unit.0 as usize];
                let rounds = (u.weapon.clip_size / 2).max(1).min(u.weapon.ammo.max(0)) as u32;
                if rounds > 0 {
                    self.unit_mut(unit).weapon.ammo -= rounds as i32;
                    self.spray_fire(unit, point, profile.spray, rounds, events);
                }
            }
            AbilityEffect::Attack(profile) if profile.is_ballistic() => {
                let victim = match target {
                    Target::Unit(id) => Some(id),
                    _ => self.unit_at(point).map(|u| u.id),
                };
                if victim.is_none() && profile.blind {
                    // Blind fire into nothing: the burst is spent all the same.
                    let u = self.unit_mut(unit);
                    let burst = if profile.rounds.is_some() { profile.rounds.unwrap_or(1) as i32 } else { u.weapon.burst.max(1) as i32 };
                    u.weapon.ammo = (u.weapon.ammo - burst).max(0);
                    events.push(GameEvent::Callout { unit, text: "BLIND FIRE".into() });
                }
                if let Some(victim) = victim {
                    let uses_ammo = def.ammo_cost > 0;
                    let cap = uses_ammo.then(|| self.units[unit.0 as usize].weapon.ammo.max(0) as u32);
                    let fired = self.fire_volley(unit, victim, &def.id, profile, cap, false, false, events);
                    if uses_ammo {
                        let u = self.unit_mut(unit);
                        u.weapon.ammo = (u.weapon.ammo - fired as i32).max(0);
                    }
                }
            }
            AbilityEffect::Suppress(profile) => {
                if let Some(victim) = self.unit_at(point).map(|u| u.id) {
                    let cap = Some(self.units[unit.0 as usize].weapon.ammo.max(0) as u32);
                    let fired = self.fire_volley(unit, victim, &def.id, profile, cap, false, true, events);
                    let u = self.unit_mut(unit);
                    u.weapon.ammo = (u.weapon.ammo - fired as i32).max(0);
                    if self.units[victim.0 as usize].is_alive() {
                        self.units[victim.0 as usize].status.suppressed_by = Some((unit, def.id.clone()));
                        let fear = self.rules.combat.suppression_fear;
                        self.add_fear(victim, fear, events);
                    }
                }
            }
            AbilityEffect::Attack(profile) if profile.fuse > 0 && profile.is_area() => {
                // A delayed charge: it lands now and goes off later.
                let team = self.units[unit.0 as usize].team;
                self.charges_armed.push(fire::ArmedCharge { unit, team, ability: def.id.clone(), pos: point, turns: profile.fuse });
                events.push(GameEvent::ChargeArmed { unit, pos: point, turns: profile.fuse });
            }
            AbilityEffect::Attack(profile) => {
                let user = self.units[unit.0 as usize].clone();
                // Rockets go off at the first thing in their way.
                let point = self.blast_point(user.pos, profile, point);
                let tiles = if profile.is_area() { self.area_tiles(user.pos, profile, point) } else { Vec::new() };
                // A pure gas grenade hurts nobody: no attack rolls, no blast.
                let harmless = profile.damage == crate::content::DamageSource::Fixed { min: 0, max: 0 } && profile.damage_bonus <= 0;
                if profile.cone > 0.0 {
                    events.push(GameEvent::Flames { unit, from: user.pos, tiles: tiles.clone() });
                } else if profile.area_radius > 0.0 && !harmless {
                    events.push(GameEvent::Explosion { center: point, radius: profile.area_radius });
                }
                if !harmless {
                    for (victim, origin) in self.attack_victims(&user, profile, point, target) {
                        self.resolve_attack(unit, victim, &def.id, origin, profile, false, events);
                    }
                }
                if profile.gas > 0 && !self.is_over() {
                    self.release_gas(&tiles, profile.gas, events);
                }
                // Cones scorch the props they sweep.
                if profile.cone > 0.0 {
                    let props: Vec<_> = self.props.iter().filter(|p| p.alive && p.hp.is_some() && tiles.contains(&p.pos)).map(|p| p.id).collect();
                    for id in props {
                        self.damage_prop(id, 1, events);
                    }
                }
                if profile.ignite > 0 && !self.is_over() {
                    self.ignite(&tiles, profile.ignite, events);
                }
                // Blasts also hit destructible props.
                if profile.area_radius > 0.0 && !harmless {
                    let (lo, hi) = match profile.damage {
                        crate::content::DamageSource::Weapon => (user.weapon.damage_min, user.weapon.damage_max),
                        crate::content::DamageSource::Fixed { min, max } => (min, max),
                    };
                    let damage = self.rng.range_inclusive(lo, hi) + profile.damage_bonus;
                    self.damage_props_in_area(point, profile.area_radius, damage, events);
                    self.damage_walls_in_area(point, profile.area_radius, damage, events);
                    if profile.shrapnel > 0 && !self.is_over() {
                        self.shrapnel_with(
                            Some(unit),
                            point,
                            profile.area_radius,
                            profile.shrapnel,
                            profile.shrapnel_damage,
                            profile.fragmentation,
                            events,
                        );
                    }
                }
            }
            AbilityEffect::Overwatch(_) => self.unit_mut(unit).status.overwatch = Some(def.id.clone()),
            AbilityEffect::Defend { defense_bonus } => {
                self.unit_mut(unit).status.defense_bonus = *defense_bonus;
                let relief = self.rules.combat.morale.hunker_relief;
                self.add_fear(unit, -relief, events);
                // Steady breathing: a little sanity back.
                self.add_insanity(unit, -symbiote::HUNKER_CALM, events);
            }
            AbilityEffect::MindControl { turns, insanity } => {
                if let Some(victim) = self.unit_at(point).map(|u| u.id) {
                    let team = self.units[unit.0 as usize].team;
                    self.add_insanity(victim, *insanity, events);
                    if self.units[victim.0 as usize].team != team {
                        self.mind_control(victim, Some(unit), team, *turns, events);
                    }
                }
            }
            AbilityEffect::Reload => {
                let u = self.unit_mut(unit);
                u.weapon.ammo = u.weapon.clip_size;
                u.weapon.spare = u.weapon.spare.saturating_sub(1);
            }
            AbilityEffect::Shutdown { turns } => {
                let u = self.unit_mut(unit);
                // +1: the countdown's last step is the reboot (see UnitStatus::shutdown).
                u.status.shutdown = turns + 1;
                u.status.overwatch = None;
                u.status.defense_bonus = 0;
                events.push(GameEvent::Shutdown { unit, shut_down: true });
            }
            AbilityEffect::Cool { amount } => self.add_heat(unit, -amount, events),
            AbilityEffect::Idle => {}
            AbilityEffect::Adrenaline { action_points } => {
                let cost = self.units[unit.0 as usize].hp / 2;
                if cost > 0 {
                    self.apply_damage(unit, cost, true, events);
                }
                let u = self.unit_mut(unit);
                if u.is_alive() {
                    u.action_points = u.action_points.saturating_add(*action_points);
                    let remaining = u.action_points;
                    events.push(GameEvent::Callout { unit, text: format!("ADRENALINE +{action_points} AP") });
                    events.push(GameEvent::ActionPointsChanged { unit, remaining });
                }
            }
            AbilityEffect::Overclock => {
                let u = self.unit_mut(unit);
                u.status.overclocked = true;
                u.status.bonus_shot = true;
                events.push(GameEvent::Callout { unit, text: "OVERCLOCKED".into() });
            }
            AbilityEffect::Leap { over_walls, .. } => {
                let from = self.units[unit.0 as usize].pos;
                self.unit_mut(unit).pos = point;
                events.push(GameEvent::UnitLeaped { unit, from, to: point, over_walls: *over_walls });
                if self.on_fire(point) && !self.burn_step(unit, events) {
                    return;
                }
                if let Some((watcher, ability)) = self.find_overwatcher(unit) {
                    self.unit_mut(watcher).status.overwatch = None;
                    let profile = self.rules.ability(&ability).and_then(|d| d.effect.attack_profile().cloned()).unwrap_or_default();
                    self.reaction_volley(watcher, unit, &ability, &profile, events);
                    if !self.units[unit.0 as usize].is_alive() || self.is_over() {
                        return;
                    }
                }
                self.on_enter(unit, point, events);
            }
            AbilityEffect::Heal { amount } => {
                if let Some(healee) = self.unit_at(point).map(|u| u.id) {
                    let h = self.unit_mut(healee);
                    let before = h.hp;
                    h.hp = (h.hp + amount).min(h.stats.max_hp);
                    let healed = h.hp - before;
                    events.push(GameEvent::UnitHealed { unit: healee, amount: healed });
                }
            }
        }
    }

    /// Units an attack affects, each with the origin used for cover/range.
    fn attack_victims(&self, user: &Unit, profile: &AttackProfile, point: GridPos, target: Target) -> Vec<(UnitId, GridPos)> {
        if profile.is_area() {
            // Cones (flamethrowers) are aimed from the user; blasts from their centre.
            let origin = if profile.cone > 0.0 { user.pos } else { point };
            let tiles = self.area_tiles(user.pos, profile, point);
            self.units
                .iter()
                .filter(|u| u.is_alive() && u.tiles().any(|p| tiles.contains(&p)))
                .filter(|u| !profile.slam || u.id != user.id)
                .map(|u| (u.id, origin))
                .collect()
        } else if let Target::Unit(id) = target {
            vec![(id, self.shot_line(user.pos, point).0)]
        } else {
            self.unit_at(point).map(|u| vec![(u.id, self.shot_line(user.pos, point).0)]).unwrap_or_default()
        }
    }

    /// `def` as `unit` would use it: with a **flame weapon** (`Weapon::cone`)
    /// its weapon attacks become cones of fire out to the target (never
    /// missing, ignoring cover, setting it alight).
    pub fn with_weapon(&self, unit: &Unit, def: &AbilityDef) -> AbilityDef {
        let mut def = def.clone();
        if unit.weapon.cone > 0.0
            && let AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) = &mut def.effect
            && p.damage == crate::content::DamageSource::Weapon
            && !p.melee
            && !p.is_area()
        {
            p.cone = unit.weapon.cone;
            p.ignite = unit.weapon.ignite;
            p.can_miss = false;
            p.half_cover_penalty = 0;
            p.full_cover_penalty = 0;
            p.range_falloff = false;
            p.flank_crit_bonus = 0;
            p.rounds = None;
        }
        def
    }

    /// Where a shot from `from` at `to` is fired from and aimed at: the
    /// tiles themselves, or lean tiles when one of them peeks around a
    /// corner ([`visibility::sight_line`]).
    pub fn shot_line(&self, from: GridPos, to: GridPos) -> (GridPos, GridPos) {
        visibility::sight_line(&self.map, from, to).unwrap_or((from, to))
    }

    /// Moves step by step so overwatchers can interrupt mid-path.
    fn do_move(&mut self, id: UnitId, to: GridPos, events: &mut Vec<GameEvent>) {
        let range = self.movement_range(id);
        // Choking on gas: the unit staggers somewhere at random (as far as it meant to go).
        let to = self.stagger(id, to, &range, events);
        let path = range.path_to(to).unwrap_or_default();
        let cost = range.cost(to).unwrap_or(0);
        let ap_cost = self.move_cost_ap(id, cost);
        // AP is paid up front: being interrupted does not refund it (XCOM rule).
        self.spend_ap(id, ap_cost, events);
        // So is a machine's heat for moving.
        let heat = self.move_heat(&self.units[id.0 as usize], ap_cost);
        if heat > 0 {
            self.add_heat(id, heat, events);
            if !self.units[id.0 as usize].is_alive() {
                return;
            }
        }

        let mut segment_start = self.units[id.0 as usize].pos;
        let mut segment: Vec<GridPos> = Vec::new();
        for step in path {
            // A script may have spawned something in the way: stop there.
            let side = self.units[id.0 as usize].side();
            if !self.footprint_free(step, side, Some(id)) && side > 1 || self.unit_at(step).is_some_and(|u| u.id != id) {
                break;
            }
            self.unit_mut(id).pos = step;
            segment.push(step);

            // Walking into fire burns.
            if self.units[id.0 as usize].tiles().any(|p| self.on_fire(p)) {
                events.push(GameEvent::UnitMoved { unit: id, from: segment_start, path: std::mem::take(&mut segment) });
                segment_start = step;
                if !self.burn_step(id, events) || self.is_over() {
                    return;
                }
            }

            if let Some((watcher, ability)) = self.find_overwatcher(id) {
                // Flush the walked part first so the frontend animates in order.
                events.push(GameEvent::UnitMoved { unit: id, from: segment_start, path: std::mem::take(&mut segment) });
                segment_start = step;
                self.unit_mut(watcher).status.overwatch = None;
                let profile = self
                    .rules
                    .ability(&ability)
                    .and_then(|d| d.effect.attack_profile().cloned())
                    .unwrap_or_default();
                self.reaction_volley(watcher, id, &ability, &profile, events);
                if !self.units[id.0 as usize].is_alive() || self.is_over() {
                    return;
                }
            }

            // Moving while suppressed draws the suppressor's fire (once).
            if let Some((gunner, ability)) = self.units[id.0 as usize].status.suppressed_by.clone() {
                let g = &self.units[gunner.0 as usize];
                if g.is_alive() && g.weapon.ammo > 0 && visibility::has_line_of_sight(&self.map, g.pos, step) {
                    if !segment.is_empty() {
                        events.push(GameEvent::UnitMoved { unit: id, from: segment_start, path: std::mem::take(&mut segment) });
                        segment_start = step;
                    }
                    self.unit_mut(id).status.suppressed_by = None;
                    let mut profile = self
                        .rules
                        .ability(&ability)
                        .and_then(|d| d.effect.attack_profile().cloned())
                        .unwrap_or_default();
                    profile.rounds = None; // a normal burst, not the full suppression belt
                    self.reaction_volley(gunner, id, &ability, &profile, events);
                    if !self.units[id.0 as usize].is_alive() || self.is_over() {
                        return;
                    }
                }
            }

            // Zone/tile scripts and first sightings. Flush the walk first if
            // they did anything, so the frontend shows them at the right moment.
            let mut script_events = Vec::new();
            self.on_enter(id, step, &mut script_events);
            if !script_events.is_empty() {
                if !segment.is_empty() {
                    events.push(GameEvent::UnitMoved { unit: id, from: segment_start, path: std::mem::take(&mut segment) });
                    segment_start = step;
                }
                events.extend(script_events);
                if !self.units[id.0 as usize].is_alive() || self.is_over() {
                    return;
                }
            }
        }
        if !segment.is_empty() {
            events.push(GameEvent::UnitMoved { unit: id, from: segment_start, path: segment });
        }
        self.after_dormant_move(id, events);
    }

    /// First enemy on overwatch that can see and reach `mover` at its current tile.
    fn find_overwatcher(&self, mover: UnitId) -> Option<(UnitId, AbilityId)> {
        let m = &self.units[mover.0 as usize];
        self.living(m.team.opponent()).find_map(|w| {
            let ability = w.status.overwatch.clone()?;
            let def = self.rules.ability(&ability)?;
            let reach = self.ability_range(w, def).min(w.stats.sight);
            let in_sight = self.sight_between(w, m, true).is_some_and(|(a, b)| a.distance(b) <= reach);
            (w.weapon.ammo > 0 && in_sight).then_some((w.id, ability))
        })
    }

    /// Reaction fire (overwatch, suppression): a ballistic volley that spends
    /// ammo, or a rolled attack for non-ballistic profiles.
    fn reaction_volley(&mut self, shooter: UnitId, target: UnitId, ability: &AbilityId, profile: &AttackProfile, events: &mut Vec<GameEvent>) {
        self.reaction_fire(shooter, target, ability, profile, events);
        // Reaction fire heats a machine like any other shot.
        let heat = self.weapon_heat(&self.units[shooter.0 as usize]);
        if heat > 0 && self.units[shooter.0 as usize].is_alive() {
            self.add_heat(shooter, heat, events);
        }
    }

    fn reaction_fire(&mut self, shooter: UnitId, target: UnitId, ability: &AbilityId, profile: &AttackProfile, events: &mut Vec<GameEvent>) {
        // A flame weapon on overwatch: a cone of fire at whoever moved in.
        if self.units[shooter.0 as usize].weapon.cone > 0.0 {
            let base = AbilityDef { id: ability.clone(), effect: AbilityEffect::Attack(profile.clone()), ..Default::default() };
            let def = self.with_weapon(&self.units[shooter.0 as usize], &base);
            let point = self.units[target.0 as usize].pos;
            self.unit_mut(shooter).weapon.ammo -= 1;
            self.run_effect(shooter, &def, point, Target::Unit(target), events);
            return;
        }
        if profile.is_ballistic() {
            let cap = Some(self.units[shooter.0 as usize].weapon.ammo.max(0) as u32);
            let fired = self.fire_volley(shooter, target, ability, profile, cap, true, false, events);
            let u = self.unit_mut(shooter);
            u.weapon.ammo = (u.weapon.ammo - fired as i32).max(0);
        } else {
            let pos = self.shot_line(self.units[shooter.0 as usize].pos, self.units[target.0 as usize].pos).0;
            self.unit_mut(shooter).weapon.ammo -= 1;
            self.resolve_attack(shooter, target, ability, pos, profile, true, events);
        }
    }

    /// Rolls one attack and applies damage. Does not touch AP or ammo.
    #[allow(clippy::too_many_arguments)]
    fn resolve_attack(
        &mut self,
        attacker: UnitId,
        target: UnitId,
        ability: &AbilityId,
        origin: GridPos,
        profile: &AttackProfile,
        reaction: bool,
        events: &mut Vec<GameEvent>,
    ) {
        let preview = preview_attack(
            &self.rules.combat,
            &self.map,
            origin,
            &self.units[attacker.0 as usize],
            &self.units[target.0 as usize],
            profile,
        );
        let hit = preview.hit_chance >= 100 || self.rng.roll_percent(preview.hit_chance);
        let outcome = if hit {
            let critical = self.rng.roll_percent(preview.crit_chance);
            let (lo, hi) = preview.damage;
            let damage = (self.rng.range_inclusive(lo, hi) + if critical { preview.crit_damage } else { 0 }).max(0);
            ShotOutcome::Hit { damage, critical }
        } else {
            ShotOutcome::Miss
        };

        events.push(GameEvent::AttackResolved {
            attacker,
            ability: ability.clone(),
            target,
            origin,
            hit_chance: preview.hit_chance,
            outcome,
            reaction,
        });

        if let ShotOutcome::Hit { damage, .. } = outcome {
            // The attack event already shows the damage; this applies it and
            // runs damaged/death scripts.
            self.apply_damage_from(target, damage, false, Some(attacker), events);
            // Tasers knock flesh and blood out for a while.
            let t = &mut self.units[target.0 as usize];
            if profile.stun > 0 && t.is_alive() && t.machine == crate::unit::Machine::None {
                t.status.stunned = t.status.stunned.max(profile.stun);
                if t.team == self.active_team {
                    t.action_points = 0;
                }
                t.status.overwatch = None;
                events.push(GameEvent::UnitStunned { unit: target, turns: profile.stun });
            }
        } else {
            let shot_at = self.rules.combat.morale.shot_at;
            self.add_fear(target, shot_at, events);
        }
    }

    fn check_victory(&mut self, events: &mut Vec<GameEvent>) {
        if self.winner.is_some() {
            return;
        }
        // Sides by allegiance: a mind-controlled soldier is still the player's.
        let any = |team: Team| self.units.iter().any(|u| u.is_alive() && u.home_team() == team);
        if !any(Team::Player) {
            self.decide(Team::Enemy, events);
        } else if self.eliminate_to_win && !any(Team::Enemy) {
            self.decide(Team::Player, events);
        }
    }

    fn end_turn(&mut self, events: &mut Vec<GameEvent>) {
        for u in self.units.iter_mut().filter(|u| u.team == self.active_team) {
            u.action_points = 0;
            // Overclock lasts the user's turn.
            u.status.overclocked = false;
            u.status.bonus_shot = false;
        }
        self.active_team = self.active_team.opponent();
        if self.active_team == Team::Player {
            self.round += 1;
        }
        // Refresh the incoming team: AP back, reactive stances expire (they
        // protected the unit during the opponent's turn), cooldowns tick.
        let team = self.active_team;
        // Suppression lasts until the suppressor's next turn.
        let suppressors: Vec<bool> = self.units.iter().map(|u| u.team == team).collect();
        for u in self.units.iter_mut() {
            if u.status.suppressed_by.as_ref().is_some_and(|(g, _)| suppressors[g.0 as usize]) {
                u.status.suppressed_by = None;
            }
        }
        // Minds held by this team tick down; spent ones go home (without AP).
        self.control_turn_start(team, events);
        let ap = self.rules.combat.action_points;
        for u in self.units.iter_mut().filter(|u| u.team == team && u.is_alive()) {
            // Dormant units with no patrol have nothing to do: they sit the
            // turn out without taking actions (hundreds of sleeping zombies
            // cost nothing). Woken during the turn, they act from the next.
            u.action_points = if u.ai.dormant && u.ai.patrol.is_empty() { 0 } else { ap };
            if u.status.stunned > 0 {
                // Knocked out: sits this turn out.
                u.status.stunned -= 1;
                u.action_points = 0;
            }

            u.status.overwatch = None;
            u.status.defense_bonus = 0;
            u.cooldowns.retain(|_, turns| {
                *turns = turns.saturating_sub(1);
                *turns > 0
            });
        }
        events.push(GameEvent::TurnStarted { team, round: self.round });
        self.heat_turn_start(team, events);
        if team == Team::Player {
            // Round start: burning vines burn away and spread the fire.
            self.vines_burn(events);
        }
        self.fire_turn_start(team, events);
        self.gas_turn_start(team, events);
        self.rage_turn_start(team, events);
        if team == Team::Enemy {
            self.vines_grow(events);
        }
        self.vines_turn_start(team, events);
        self.insanity_turn_start(team, events);
        self.refresh_gassed();
        self.charges_turn_start(team, events);
        self.morale_turn_start(team, events);
        self.on_turn_start(events);
        self.check_victory(events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ApCost, Ruleset};
    use crate::modules::GrenadierModule;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    /// One player unit of `class` at `A`, one sectoid at `E`.
    fn duel(art: &str, class: &str) -> (Battle, UnitId, UnitId) {
        let mut b = Battle::new(Map::from_ascii(art), rules(), 1234);
        let a = b.spawn(class, "A", Team::Player, Map::find_markers(art, 'A')[0]).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, Map::find_markers(art, 'E')[0]).unwrap();
        (b, a, e)
    }

    fn shoot(a: UnitId, e: UnitId) -> Action {
        Action::ability(a, "shoot", Target::Unit(e))
    }

    const OPEN: &str = "
        ..E..
        .....
        ..A..
    ";

    #[test]
    fn move_costs_ap_and_dash_costs_two() {
        let art = "
            E.........
            ..........
            A.........
        ";
        let (mut b, a, _) = duel(art, "rifleman");
        b.apply(Action::Move { unit: a, to: GridPos::new(3, 0) }).unwrap();
        assert_eq!(b.unit(a).unwrap().action_points, 1);

        let (mut b, a, _) = duel(art, "rifleman");
        b.apply(Action::Move { unit: a, to: GridPos::new(8, 0) }).unwrap();
        assert_eq!(b.active_team(), Team::Enemy, "dash used both AP, turn passed");
    }

    #[test]
    fn cannot_shoot_through_walls() {
        let (b, a, e) = duel(
            "
            ..E..
            HHHHH
            ..A..
            ",
            "rifleman",
        );
        assert_eq!(b.validate(&shoot(a, e)), Err(ActionError::NoLineOfSight));
    }

    #[test]
    fn shooting_uses_ammo_and_ends_unit_turn() {
        let (mut b, a, e) = duel(OPEN, "rifleman");
        let events = b.apply(shoot(a, e)).unwrap();
        assert!(matches!(events[0], GameEvent::AbilityUsed { .. }));
        let fired = events.iter().find_map(|ev| match ev {
            GameEvent::Volley { bullets, .. } => Some(bullets.len() as i32),
            _ => None,
        });
        let u = b.unit(a).unwrap();
        assert_eq!(fired, Some(u.weapon.burst as i32), "a burst of traced rounds");
        assert_eq!(u.weapon.ammo, u.weapon.clip_size - u.weapon.burst as i32);
        assert!(b.is_over() || b.active_team() == Team::Enemy);
    }

    #[test]
    fn overwatch_interrupts_movement() {
        let art = "
            ....E....
            .........
            .........
            .........
            A........
        ";
        let (mut b, a, e) = duel(art, "rifleman");
        b.apply(Action::ability(a, "overwatch", Target::None)).unwrap();
        assert_eq!(b.active_team(), Team::Enemy);
        let events = b.apply(Action::Move { unit: e, to: GridPos::new(4, 2) }).unwrap();
        assert!(
            events.iter().any(|ev| matches!(ev, GameEvent::Volley { reaction: true, .. })),
            "overwatch should fire: {events:?}"
        );
        assert!(b.unit(a).unwrap().status.overwatch.is_none(), "overwatch is consumed");
    }

    #[test]
    fn unknown_and_unlearned_abilities_are_rejected() {
        let (b, a, _) = duel(OPEN, "assault");
        assert_eq!(
            b.validate(&Action::ability(a, "nope", Target::None)),
            Err(ActionError::UnknownAbility("nope".into()))
        );
        // Assaults don't get the grenadier's grenade.
        assert_eq!(
            b.validate(&Action::ability(a, "frag_grenade", Target::Tile(GridPos::new(2, 2)))),
            Err(ActionError::AbilityNotKnown("frag_grenade".into()))
        );
    }

    #[test]
    fn grenade_hits_area_and_uses_its_charge() {
        let art = "
            .......
            ..EE...
            .......
            .......
            A......
        ";
        let mut b = Battle::new(Map::from_ascii(art), rules(), 7);
        let a = b.spawn("grenadier", "G", Team::Player, GridPos::new(0, 0)).unwrap();
        let e1 = b.spawn("sectoid", "E1", Team::Enemy, GridPos::new(2, 3)).unwrap();
        let e2 = b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(3, 3)).unwrap();
        let throw = Action::ability(a, "frag_grenade", Target::Tile(GridPos::new(2, 3)));

        let preview = b.preview(a, &"frag_grenade".into(), Target::Tile(GridPos::new(2, 3))).unwrap();
        assert_eq!(preview.len(), 2);
        assert!(preview.iter().all(|(_, p)| p.hit_chance == 100));

        let events = b.apply(throw.clone()).unwrap();
        let hits: Vec<_> = events
            .iter()
            .filter_map(|ev| match ev {
                GameEvent::AttackResolved { target, outcome: ShotOutcome::Hit { .. }, .. } => Some(*target),
                _ => None,
            })
            .collect();
        assert_eq!(hits, vec![e1, e2]);
        assert_eq!(b.validate(&throw), Err(ActionError::NoCharges("frag_grenade".into())));
    }

    #[test]
    fn heal_and_cooldowns() {
        let mut r = rules();
        // Give the medkit a cooldown to exercise that path too.
        r.ability_mut(&"medkit".into()).unwrap().cooldown = 2;
        r.ability_mut(&"medkit".into()).unwrap().cost = ApCost::Points(1);
        let art = "
            E....
            .....
            A....
        ";
        let mut b = Battle::new(Map::from_ascii(art), r, 3);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(0, 2)).unwrap();
        let heal = Action::ability(a, "medkit", Target::Unit(a));
        assert_eq!(b.validate(&heal), Err(ActionError::NothingToHeal));
        b.units[a.0 as usize].hp = 2;
        let events = b.apply(heal.clone()).unwrap();
        assert!(events.contains(&GameEvent::UnitHealed { unit: a, amount: 4 }));
        assert!(matches!(b.validate(&heal), Err(ActionError::OnCooldown { turns: 2, .. })));
    }

    #[test]
    fn same_seed_same_battle() {
        let run = || {
            let (mut b, a, e) = duel(OPEN, "rifleman");
            let mut log = Vec::new();
            for _ in 0..6 {
                if b.is_over() {
                    break;
                }
                let (me, them) = if b.active_team() == Team::Player { (a, e) } else { (e, a) };
                let action = if b.validate(&shoot(me, them)) == Err(ActionError::OutOfAmmo) {
                    Action::ability(me, "reload", Target::None)
                } else {
                    shoot(me, them)
                };
                log.extend(b.apply(action).unwrap());
            }
            log
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn live_tuning_changes_odds() {
        let (mut b, a, e) = duel(OPEN, "rifleman");
        let before = b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.aim_chance;
        if let AbilityEffect::Attack(p) = &mut b.rules_mut().ability_mut(&"shoot".into()).unwrap().effect {
            p.accuracy_bonus = -30;
        }
        let after = b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.aim_chance;
        let expected = 30.0 * b.rules().combat.accuracy_multiplier;
        assert!(((before - after) as f32 - expected).abs() <= 1.0 || before == b.rules().combat.max_hit, "{before} -> {after}");
        assert!(after < before - 25);
    }
}

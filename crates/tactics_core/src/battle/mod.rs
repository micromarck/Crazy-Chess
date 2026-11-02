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
            eliminate_to_win: true,
            mission_id: None,
            script_depth: 0,
            stealth: Stealth::default(),
            fog: false,
            ai_goals: [None, None],
            objective: String::new(),
            zones: Vec::new(),
        }
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
        if !self.map.is_walkable(pos) {
            return Err(format!("cannot spawn on non-walkable tile {pos}"));
        }
        if self.unit_at(pos).is_some() {
            return Err(format!("tile {pos} already occupied"));
        }
        weapon.ammo = weapon.clip_size;
        weapon.spare = weapon.magazines;

        // Items: stats, abilities and their uses.
        let mut stats = class.stats;
        let mut abilities = class.abilities.clone();
        let mut charges: std::collections::BTreeMap<AbilityId, u32> = Default::default();
        let items: Vec<ItemDef> =
            gear.armor.iter().chain(gear.items.iter()).filter_map(|id| self.rules.item(id).cloned()).collect();
        for item in &items {
            let m = item.stats;
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
            ai: Default::default(),
            fear: 0,
            fearless: class.fearless,
            gear: Loadout { weapon: Some(weapon_id), ..gear },
            appearance: None,
        };
        self.units.push(unit);
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

    /// The living unit standing on `pos`, if any.
    pub fn unit_at(&self, pos: GridPos) -> Option<&Unit> {
        self.units.iter().find(|u| u.is_alive() && u.pos == pos)
    }

    /// Every tile `unit` could move to with its remaining action points.
    /// Use [`Battle::move_cost_ap`] to split it into "1 AP" and "dash" zones.
    pub fn movement_range(&self, id: UnitId) -> ReachableSet {
        let Some(unit) = self.unit(id) else {
            return ReachableSet::default();
        };
        let budget = unit.stats.mobility * unit.action_points as u32;
        let climb = crate::grid::Climb { max_up: unit.stats.climb };
        pathfinding::reachable(&self.map, unit.pos, budget, climb, |p| self.unit_at(p).is_none())
    }

    /// How many action points moving `movement_cost` points costs this unit.
    pub fn move_cost_ap(&self, id: UnitId, movement_cost: u32) -> u8 {
        let mobility = self.unit(id).map_or(1, |u| u.stats.mobility.max(1));
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
            .filter(|t| u.pos.distance(t.pos) <= range && visibility::has_line_of_sight(&self.map, u.pos, t.pos))
            .map(|t| t.id)
            .collect()
    }

    /// Looks up an ability definition.
    pub fn ability(&self, id: &AbilityId) -> Option<&AbilityDef> {
        self.rules.ability(id)
    }

    /// Maximum reach of `def` when used by `unit`, in tiles.
    pub fn ability_range(&self, unit: &Unit, def: &AbilityDef) -> f32 {
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
        let AbilityEffect::Attack(profile) = &def.effect else {
            return Ok(Vec::new());
        };
        Ok(self
            .attack_victims(user, profile, point, target)
            .into_iter()
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
                t.pos
            }
            (Targeting::Tile, Target::Tile(p)) if self.map.in_bounds(p) => p,
            _ => return Err(ActionError::WrongTarget),
        };
        if user.pos.distance(point) > self.ability_range(user, def) {
            return Err(ActionError::OutOfRange);
        }
        if def.requires_los && !visibility::has_line_of_sight(&self.map, user.pos, point) {
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
                    _ => {}
                }
                let point = self.check_target(u, def, *target)?;
                if let AbilityEffect::Heal { .. } = def.effect {
                    let healee = self.unit_at(point).ok_or(ActionError::WrongTarget)?;
                    if healee.hp >= healee.stats.max_hp {
                        return Err(ActionError::NothingToHeal);
                    }
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
                return Ok(events);
            }
        }

        self.check_victory(&mut events);
        if !self.is_over() && !self.active_team_can_act() {
            self.end_turn(&mut events);
        }
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

        self.run_effect(unit, &def, point, target, events);

        match def.cost {
            crate::content::ApCost::EndsTurn => self.spend_all_ap(unit, events),
            crate::content::ApCost::Points(n) => self.spend_ap(unit, n, events),
        }
    }

    /// Executes an ability's effect. **Add new effect kinds here.**
    fn run_effect(&mut self, unit: UnitId, def: &AbilityDef, point: GridPos, target: Target, events: &mut Vec<GameEvent>) {
        match &def.effect {
            AbilityEffect::Attack(profile) if profile.is_ballistic() => {
                let victim = match target {
                    Target::Unit(id) => Some(id),
                    _ => self.unit_at(point).map(|u| u.id),
                };
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
            AbilityEffect::Attack(profile) => {
                if profile.area_radius > 0.0 {
                    events.push(GameEvent::Explosion { center: point, radius: profile.area_radius });
                }
                let user = self.units[unit.0 as usize].clone();
                for (victim, origin) in self.attack_victims(&user, profile, point, target) {
                    self.resolve_attack(unit, victim, &def.id, origin, profile, false, events);
                }
                // Blasts also hit destructible props.
                if profile.area_radius > 0.0 {
                    let (lo, hi) = match profile.damage {
                        crate::content::DamageSource::Weapon => (user.weapon.damage_min, user.weapon.damage_max),
                        crate::content::DamageSource::Fixed { min, max } => (min, max),
                    };
                    let damage = self.rng.range_inclusive(lo, hi) + profile.damage_bonus;
                    self.damage_props_in_area(point, profile.area_radius, damage, events);
                }
            }
            AbilityEffect::Overwatch(_) => self.unit_mut(unit).status.overwatch = Some(def.id.clone()),
            AbilityEffect::Defend { defense_bonus } => {
                self.unit_mut(unit).status.defense_bonus = *defense_bonus;
                let relief = self.rules.combat.morale.hunker_relief;
                self.add_fear(unit, -relief, events);
            }
            AbilityEffect::Reload => {
                let u = self.unit_mut(unit);
                u.weapon.ammo = u.weapon.clip_size;
                u.weapon.spare = u.weapon.spare.saturating_sub(1);
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
        if profile.area_radius > 0.0 {
            self.units
                .iter()
                .filter(|u| u.is_alive() && u.pos.distance(point) <= profile.area_radius)
                .map(|u| (u.id, point))
                .collect()
        } else if let Target::Unit(id) = target {
            vec![(id, user.pos)]
        } else {
            self.unit_at(point).map(|u| vec![(u.id, user.pos)]).unwrap_or_default()
        }
    }

    /// Moves step by step so overwatchers can interrupt mid-path.
    fn do_move(&mut self, id: UnitId, to: GridPos, events: &mut Vec<GameEvent>) {
        let range = self.movement_range(id);
        let path = range.path_to(to).unwrap_or_default();
        let cost = range.cost(to).unwrap_or(0);
        let ap_cost = self.move_cost_ap(id, cost);
        // AP is paid up front: being interrupted does not refund it (XCOM rule).
        self.spend_ap(id, ap_cost, events);

        let mut segment_start = self.units[id.0 as usize].pos;
        let mut segment: Vec<GridPos> = Vec::new();
        for step in path {
            // A script may have spawned something in the way: stop there.
            if self.unit_at(step).is_some_and(|u| u.id != id) {
                break;
            }
            self.unit_mut(id).pos = step;
            segment.push(step);

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
            let in_range = w.pos.distance(m.pos) <= self.ability_range(w, def).min(w.stats.sight);
            (w.weapon.ammo > 0 && in_range && visibility::has_line_of_sight(&self.map, w.pos, m.pos))
                .then_some((w.id, ability))
        })
    }

    /// Reaction fire (overwatch, suppression): a ballistic volley that spends
    /// ammo, or a rolled attack for non-ballistic profiles.
    fn reaction_volley(&mut self, shooter: UnitId, target: UnitId, ability: &AbilityId, profile: &AttackProfile, events: &mut Vec<GameEvent>) {
        if profile.is_ballistic() {
            let cap = Some(self.units[shooter.0 as usize].weapon.ammo.max(0) as u32);
            let fired = self.fire_volley(shooter, target, ability, profile, cap, true, false, events);
            let u = self.unit_mut(shooter);
            u.weapon.ammo = (u.weapon.ammo - fired as i32).max(0);
        } else {
            let pos = self.units[shooter.0 as usize].pos;
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
        } else {
            let shot_at = self.rules.combat.morale.shot_at;
            self.add_fear(target, shot_at, events);
        }
    }

    fn check_victory(&mut self, events: &mut Vec<GameEvent>) {
        if self.winner.is_some() {
            return;
        }
        if self.living(Team::Player).next().is_none() {
            self.decide(Team::Enemy, events);
        } else if self.eliminate_to_win && self.living(Team::Enemy).next().is_none() {
            self.decide(Team::Player, events);
        }
    }

    fn end_turn(&mut self, events: &mut Vec<GameEvent>) {
        for u in self.units.iter_mut().filter(|u| u.team == self.active_team) {
            u.action_points = 0;
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
        let ap = self.rules.combat.action_points;
        for u in self.units.iter_mut().filter(|u| u.team == team && u.is_alive()) {
            u.action_points = ap;
            u.status.overwatch = None;
            u.status.defense_bonus = 0;
            u.cooldowns.retain(|_, turns| {
                *turns = turns.saturating_sub(1);
                *turns > 0
            });
        }
        events.push(GameEvent::TurnStarted { team, round: self.round });
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
        assert_eq!(after, before - 30);
    }
}

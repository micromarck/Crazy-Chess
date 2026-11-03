//! The mission layer of a battle: props, script hooks, flags and everything
//! scripts can do.
//!
//! * [`Battle::from_mission`] builds a battle from a [`MissionDef`].
//! * [`Battle::begin`] runs the mission's `start` scripts; call it once and
//!   play back the events it returns.
//! * Hooks fire from inside the rules (`fire`): when a unit steps onto a zone,
//!   takes damage, dies or is first spotted, when a prop is damaged/destroyed
//!   and when a turn starts.
//! * Scripts return [`ScriptCommand`]s, applied by
//!   [`Battle::apply_script_command`], which emits normal [`GameEvent`]s, so
//!   the frontend animates script effects like any other.
//!
//! Re-entrancy (a script's explosion kills a unit whose `death` script
//! spawns more units...) is allowed up to [`MAX_SCRIPT_DEPTH`] levels.

use std::collections::BTreeMap;

use super::Battle;
use crate::event::GameEvent;
use crate::grid::{CoverLevel, GridPos, Surface, Terrain};
use crate::mission::{HookEvent, MissionDef, ScriptHook, ZoneDef};
use crate::script::{self, ScriptCommand, ScriptVars, ScriptWorld, UnitStat, WorldProp, WorldUnit};
use crate::unit::{Team, UnitId};
use super::stealth::team_index;

/// Maximum nesting of scripts triggering scripts.
pub const MAX_SCRIPT_DEPTH: u32 = 16;

/// Stable identifier of a prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropId(pub u32);

/// A placed object: crate, barrel, console, wall section...
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub id: PropId,
    pub tag: String,
    pub kind: String,
    pub pos: GridPos,
    pub cover: CoverLevel,
    /// `None` = indestructible.
    pub hp: Option<i32>,
    pub model: Option<String>,
    pub alive: bool,
    /// Set for door panels (see `stealth.rs`).
    pub door: Option<super::DoorState>,
}

/// Who a hook belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum HookOwner {
    Mission,
    Zone(ZoneDef),
    Unit(UnitId),
    Prop(PropId),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ActiveHook {
    pub owner: HookOwner,
    pub hook: ScriptHook,
    pub fired: bool,
}

/// What happened, for matching hooks.
#[derive(Debug, Clone, Copy)]
pub(super) struct Trigger {
    pub event: HookEvent,
    pub unit: Option<UnitId>,
    pub prop: Option<PropId>,
    pub pos: Option<GridPos>,
    pub amount: i64,
}

impl Trigger {
    pub fn new(event: HookEvent) -> Self {
        Self { event, unit: None, prop: None, pos: None, amount: 0 }
    }
}

impl Battle {
    /// Builds a battle from a mission. Problems that don't prevent playing
    /// (unknown class, occupied spawn...) are returned as warnings.
    pub fn from_mission(rules: crate::content::Ruleset, mission: &MissionDef, seed: u64) -> (Battle, Vec<String>) {
        let mut battle = Battle::new(mission.map.clone(), rules, mission.seed.unwrap_or(seed));
        let mut warnings = Vec::new();
        battle.eliminate_to_win = mission.eliminate_to_win;
        battle.library = mission.library.clone();
        battle.mission_id = Some(mission.id.clone());

        for def in &mission.props {
            let id = PropId(battle.props.len() as u32);
            // Props occupy their tile: any cover blocks movement.
            let terrain = match def.cover {
                CoverLevel::Full => Terrain::HighCover,
                CoverLevel::Half => Terrain::LowCover,
                CoverLevel::None => battle.map.terrain(def.pos),
            };
            battle.map.set_terrain(def.pos, terrain);
            battle.props.push(Prop {
                id,
                tag: def.tag.clone(),
                kind: def.kind.clone(),
                pos: def.pos,
                cover: def.cover,
                hp: def.hp,
                model: def.model.clone(),
                alive: true,
                door: None,
            });
            battle.add_hooks(HookOwner::Prop(id), &def.hooks);
        }
        battle.setup_mechanics(mission);
        battle.sky = mission.sky.clone();

        for d in &mission.deploys {
            let members = battle.rules.squad(&d.squad).map(|s| s.members.clone()).unwrap_or_default();
            if members.is_empty() {
                warnings.push(format!("deploy: squad '{}' is empty or unknown", d.squad));
            }
            for m in members {
                match battle.free_tile_near(d.pos) {
                    Some(p) => {
                        match battle.spawn_equipped(&m.class, m.name.clone(), d.team, p, m.loadout.as_deref(), &m.gear) {
                            Ok(id) => {
                                if let Some(look) = m.appearance {
                                    battle.set_unit_appearance(id, look);
                                }
                            }
                            Err(e) => warnings.push(format!("{}: {e}", m.name)),
                        }
                    }
                    None => warnings.push(format!("deploy: no room for {}", m.name)),
                }
            }
        }

        for u in &mission.units {
            match battle.spawn_equipped(&u.class, u.name.clone(), u.team, u.pos, u.loadout.as_deref(), &u.gear) {
                Ok(id) => {
                    if let Some(tag) = &u.tag {
                        battle.unit_tags.insert(id, tag.clone());
                    }
                    let ai = &mut battle.units[id.0 as usize].ai;
                    ai.dormant = u.dormant || !u.patrol.is_empty();
                    ai.pod = u.pod.clone();
                    ai.patrol = u.patrol.clone();
                    battle.add_hooks(HookOwner::Unit(id), &u.hooks);
                }
                Err(e) => warnings.push(format!("unit '{}': {e}", u.name)),
            }
        }

        for z in &mission.zones {
            battle.add_hooks(HookOwner::Zone(z.clone()), &z.hooks);
        }
        battle.add_hooks(HookOwner::Mission, &mission.hooks);
        (battle, warnings)
    }

    pub(super) fn add_hooks(&mut self, owner: HookOwner, hooks: &[ScriptHook]) {
        for h in hooks {
            self.hooks.push(ActiveHook { owner: owner.clone(), hook: h.clone(), fired: false });
        }
    }

    /// Starts the mission: runs `start` scripts and initial spotting. Call once
    /// after creating the battle and play back the returned events.
    pub fn begin(&mut self) -> Vec<GameEvent> {
        let mut events = Vec::new();
        self.fire(Trigger::new(HookEvent::Start), &mut events);
        self.check_spotted(&mut events);
        self.check_victory(&mut events);
        events
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn props(&self) -> &[Prop] {
        &self.props
    }

    pub fn prop(&self, id: PropId) -> Option<&Prop> {
        self.props.get(id.0 as usize)
    }

    /// Script flags (`set_flag` / `flag`).
    pub fn flags(&self) -> &BTreeMap<String, i64> {
        &self.flags
    }

    /// Id of the mission this battle was built from, if any.
    pub fn mission_id(&self) -> Option<&str> {
        self.mission_id.as_deref()
    }

    /// The script tag of a unit (`<unit tag="...">`).
    pub fn unit_tag(&self, id: UnitId) -> Option<&str> {
        self.unit_tags.get(&id).map(String::as_str)
    }

    /// Nearest walkable, unoccupied tile to `pos` (spiral search, radius 4).
    pub fn free_tile_near(&self, pos: GridPos) -> Option<GridPos> {
        (0..=4).find_map(|r| {
            let mut ring: Vec<GridPos> = (-r..=r)
                .flat_map(|dy| (-r..=r).map(move |dx| GridPos::new(pos.x + dx, pos.y + dy)))
                .filter(|p| p.chebyshev(pos) == r)
                .filter(|p| self.map.is_walkable(*p) && self.unit_at(*p).is_none())
                .collect();
            ring.sort_by_key(|p| (p.distance(pos) * 100.0) as i32);
            ring.first().copied()
        })
    }

    // ------------------------------------------------------------------
    // Hooks
    // ------------------------------------------------------------------

    /// Runs every hook matching `t`.
    pub(super) fn fire(&mut self, t: Trigger, events: &mut Vec<GameEvent>) {
        if self.hooks.is_empty() {
            return;
        }
        if self.script_depth >= MAX_SCRIPT_DEPTH {
            events.push(GameEvent::ScriptError { message: "scripts triggered each other too deeply; stopped".into() });
            return;
        }
        let unit_team = t.unit.and_then(|u| self.unit(u)).map(|u| u.team);
        let matching: Vec<usize> = (0..self.hooks.len())
            .filter(|&i| {
                let h = &self.hooks[i];
                if h.hook.on != t.event || (h.hook.once && h.fired) {
                    return false;
                }
                // Team filter: the unit's team, or for turn starts the active team.
                let team = if t.event == HookEvent::TurnStart { Some(self.active_team) } else { unit_team };
                if h.hook.team.is_some() && h.hook.team != team {
                    return false;
                }
                match &h.owner {
                    HookOwner::Mission => {
                        matches!(t.event, HookEvent::Start | HookEvent::TurnStart | HookEvent::UnitKilled | HookEvent::Alarm)
                    }
                    HookOwner::Zone(z) => t.event == HookEvent::Enter && t.pos.is_some_and(|p| z.contains(p)),
                    HookOwner::Unit(id) => match t.event {
                        HookEvent::TurnStart => self.unit(*id).is_some_and(|u| u.is_alive() && u.team == self.active_team),
                        _ => t.unit == Some(*id),
                    },
                    HookOwner::Prop(id) => t.prop == Some(*id),
                }
            })
            .collect();

        for i in matching {
            self.hooks[i].fired = true;
            let owner = self.hooks[i].owner.clone();
            let code = self.hooks[i].hook.code.clone();
            // For per-unit turn_start hooks, `unit` is the hook's own unit.
            let unit = match (&owner, t.event) {
                (HookOwner::Unit(id), HookEvent::TurnStart) => Some(*id),
                _ => t.unit,
            };
            let prop = match owner {
                HookOwner::Prop(id) => Some(id),
                _ => t.prop,
            };
            let pos = t.pos.or_else(|| unit.and_then(|u| self.unit(u)).map(|u| u.pos)).or_else(|| prop.and_then(|p| self.prop(p)).map(|p| p.pos));
            let vars = ScriptVars { unit: unit.map(|u| u.0), prop: prop.and_then(|p| self.prop(p)).map(|p| p.tag.clone()), pos, amount: t.amount };
            self.run_hook(&code, &vars, events);
            if self.is_over() {
                return;
            }
        }
    }

    /// Runs every hook once against the current state **without applying
    /// anything** and returns the errors (syntax errors, unknown functions,
    /// bad arguments). For tests and tools: scripts only fail at run time, so
    /// this catches mistakes in hooks a test game never triggers.
    pub fn dry_run_scripts(&self) -> Vec<String> {
        let world = self.snapshot();
        let unit = self.living(Team::Player).next().map(|u| u.id.0);
        self.hooks
            .iter()
            .filter_map(|h| {
                let vars = ScriptVars { unit, prop: None, pos: unit.and_then(|u| self.unit(UnitId(u))).map(|u| u.pos), amount: 1 };
                script::run(&h.hook.code, &self.script_library(), &vars, &world).err().map(|e| format!("{:?} hook: {e}", h.hook.on))
            })
            .collect()
    }

    fn snapshot(&self) -> ScriptWorld {
        ScriptWorld {
            round: self.round as i64,
            active_team: Some(self.active_team),
            units: self
                .units
                .iter()
                .map(|u| WorldUnit {
                    id: u.id.0,
                    team: u.team,
                    name: u.name.clone(),
                    tag: self.unit_tags.get(&u.id).cloned(),
                    pos: u.pos,
                    hp: u.hp,
                    max_hp: u.stats.max_hp,
                    alive: u.is_alive(),
                    dormant: u.ai.dormant,
                    fear: u.fear,
                    class: u.class.clone(),
                    aim: u.stats.aim,
                    defense: u.stats.defense,
                    mobility: (u.stats.mobility / 2) as i32,
                    sight: u.stats.sight.round() as i32,
                })
                .collect(),
            props: self.props.iter().map(|p| WorldProp { tag: p.tag.clone(), pos: p.pos, hp: p.hp, alive: p.alive }).collect(),
            map: Some(self.map.clone()),
            flags: self.flags.clone(),
            zones: {
                // Marked zones, plus unmarked ones that carry scripts.
                let mut zones: Vec<(String, GridPos, GridPos)> = self.zones.iter().map(|z| (z.tag.clone(), z.min, z.max)).collect();
                for h in &self.hooks {
                    if let HookOwner::Zone(z) = &h.owner
                        && !zones.iter().any(|(t, ..)| *t == z.tag)
                    {
                        zones.push((z.tag.clone(), z.min, z.max));
                    }
                }
                zones
            },
            seed: 0,
            alarm: self.stealth.alarm,
            concealed: self.stealth.concealed,
        }
    }

    /// Module script functions, then the mission's own library.
    fn script_library(&self) -> String {
        if self.rules.script_library.is_empty() { self.library.clone() } else { format!("{}\n{}", self.rules.script_library, self.library) }
    }

    fn run_hook(&mut self, code: &str, vars: &ScriptVars, events: &mut Vec<GameEvent>) {
        let mut world = self.snapshot();
        world.seed = self.rng.next_u64();
        let library = self.script_library();
        match script::run(code, &library, vars, &world) {
            Ok(commands) => {
                self.script_depth += 1;
                for c in commands {
                    self.apply_script_command(c, events);
                    if self.is_over() {
                        break;
                    }
                }
                self.script_depth -= 1;
            }
            Err(message) => events.push(GameEvent::ScriptError { message }),
        }
    }

    /// Applies one script command. **Handle new [`ScriptCommand`]s here.**
    pub fn apply_script_command(&mut self, command: ScriptCommand, events: &mut Vec<GameEvent>) {
        match command {
            ScriptCommand::Message(text) => events.push(GameEvent::Message { text }),
            ScriptCommand::Spawn { team, class, name, pos } => match self.free_tile_near(pos) {
                Some(p) => match self.spawn(&class, name, team, p) {
                    Ok(unit) => {
                        events.push(GameEvent::UnitSpawned { unit });
                        self.check_spotted(events);
                    }
                    Err(message) => events.push(GameEvent::ScriptError { message }),
                },
                None => events.push(GameEvent::ScriptError { message: format!("spawn_unit: no free tile near {pos}") }),
            },
            ScriptCommand::Damage { unit, amount } => self.apply_damage(UnitId(unit), amount, true, events),
            ScriptCommand::SetStat { unit, stat, value, relative } => {
                let Some(u) = self.units.get_mut(unit as usize).filter(|u| u.is_alive()) else { return };
                let s = &mut u.stats;
                let now = match stat {
                    UnitStat::Aim => s.aim,
                    UnitStat::Defense => s.defense,
                    UnitStat::Mobility => (s.mobility / 2) as i32,
                    UnitStat::Sight => s.sight.round() as i32,
                    UnitStat::MaxHp => s.max_hp,
                };
                let v = if relative { now + value } else { value };
                match stat {
                    UnitStat::Aim => s.aim = v.clamp(0, 200),
                    UnitStat::Defense => s.defense = v.clamp(-100, 100),
                    UnitStat::Mobility => s.mobility = (v.clamp(0, 30) * 2) as u32,
                    UnitStat::Sight => s.sight = v.clamp(1, 40) as f32,
                    UnitStat::MaxHp => {
                        s.max_hp = v.clamp(1, 999);
                        u.hp = u.hp.min(s.max_hp);
                    }
                }
            }
            ScriptCommand::Say { unit, text } => events.push(GameEvent::Callout { unit: UnitId(unit), text }),
            ScriptCommand::Kill { unit } => self.apply_damage(UnitId(unit), i32::MAX / 2, true, events),
            ScriptCommand::Heal { unit, amount } => {
                if let Some(u) = self.units.get_mut(unit as usize).filter(|u| u.is_alive()) {
                    let before = u.hp;
                    u.hp = (u.hp + amount.max(0)).min(u.stats.max_hp);
                    events.push(GameEvent::UnitHealed { unit: UnitId(unit), amount: u.hp - before });
                }
            }
            ScriptCommand::DamageProp { tag, amount } => {
                if let Some(id) = self.prop_by_tag(&tag) {
                    self.damage_prop(id, amount, events);
                }
            }
            ScriptCommand::DestroyProp { tag } => {
                if let Some(id) = self.prop_by_tag(&tag) {
                    self.destroy_prop(id, events);
                }
            }
            ScriptCommand::Explode { pos, radius, damage } => self.explode(pos, radius, damage, events),
            ScriptCommand::SetTerrain { pos, terrain } => {
                self.map.set_terrain(pos, terrain);
                events.push(GameEvent::TerrainChanged { pos });
            }
            ScriptCommand::SetHeight { pos, level } => {
                self.map.set_elevation(pos, level);
                events.push(GameEvent::TerrainChanged { pos });
            }
            ScriptCommand::SetLadder { pos, ladder } => {
                self.map.set_ladder(pos, ladder);
                events.push(GameEvent::TerrainChanged { pos });
            }
            ScriptCommand::GiveAp { unit, amount } => {
                if let Some(u) = self.units.get_mut(unit as usize).filter(|u| u.is_alive()) {
                    u.action_points = (u.action_points as i32 + amount).clamp(0, 9) as u8;
                    events.push(GameEvent::ActionPointsChanged { unit: UnitId(unit), remaining: u.action_points });
                }
            }
            ScriptCommand::SetFlag { name, value } => {
                self.flags.insert(name, value);
            }
            ScriptCommand::ShowZone { tag, show } => self.set_zone(&tag, Some(show), None, events),
            ScriptCommand::ZoneLabel { tag, text } => self.set_zone(&tag, None, Some(text), events),
            ScriptCommand::AddFear { unit, amount } => self.add_fear(UnitId(unit), amount, events),
            ScriptCommand::Objective(text) => self.set_objective(text, events),
            ScriptCommand::RaiseAlarm => self.raise_alarm(None, events),
            ScriptCommand::Wake { unit } => self.wake(UnitId(unit), events),
            ScriptCommand::WakeAll => {
                let ids: Vec<UnitId> = self.units.iter().filter(|u| u.ai.dormant).map(|u| u.id).collect();
                for id in ids {
                    self.wake(id, events);
                }
            }
            ScriptCommand::OpenDoor { tag } => self.set_door_open(&tag, true, events),
            ScriptCommand::CloseDoor { tag } => self.set_door_open(&tag, false, events),
            ScriptCommand::LockDoor { tag, locked } => self.lock_door(&tag, locked),
            ScriptCommand::AiGoal { team, pos } => self.ai_goals[team_index(team)] = Some(pos),
            ScriptCommand::ClearAiGoal { team } => self.ai_goals[team_index(team)] = None,
            ScriptCommand::Win => self.decide(Team::Player, events),
            ScriptCommand::Lose => self.decide(Team::Enemy, events),
        }
    }

    pub(super) fn decide(&mut self, winner: Team, events: &mut Vec<GameEvent>) {
        if self.winner.is_none() {
            self.winner = Some(winner);
            events.push(GameEvent::BattleEnded { winner });
        }
    }

    fn prop_by_tag(&self, tag: &str) -> Option<PropId> {
        self.props.iter().find(|p| p.tag == tag && p.alive).map(|p| p.id)
    }

    // ------------------------------------------------------------------
    // Damage, props, explosions
    // ------------------------------------------------------------------

    /// Applies damage to a unit (from an attack or a script) and runs the
    /// `damaged` / `death` / `unit_killed` hooks. `announce` emits
    /// [`GameEvent::UnitDamaged`] (attacks already reported their damage).
    pub(super) fn apply_damage(&mut self, target: UnitId, amount: i32, announce: bool, events: &mut Vec<GameEvent>) {
        self.apply_damage_from(target, amount, announce, None, events);
    }

    /// [`Battle::apply_damage`] knowing who did it (for morale).
    pub(super) fn apply_damage_from(
        &mut self,
        target: UnitId,
        amount: i32,
        announce: bool,
        attacker: Option<UnitId>,
        events: &mut Vec<GameEvent>,
    ) {
        let Some(t) = self.units.get_mut(target.0 as usize).filter(|u| u.is_alive()) else { return };
        let amount = amount.max(0).min(t.hp.max(0) + 1_000);
        t.hp -= amount;
        let died = t.hp <= 0;
        if died {
            t.hp = 0;
            t.action_points = 0;
            t.status = Default::default();
        }
        if announce {
            events.push(GameEvent::UnitDamaged { unit: target, amount });
        }
        let pos = self.units[target.0 as usize].pos;
        if !died {
            self.wake(target, events);
            let per = self.rules.combat.morale.per_damage;
            self.add_fear(target, per * amount, events);
        } else {
            self.fear_on_death(target, attacker, events);
        }
        self.fire(Trigger { unit: Some(target), pos: Some(pos), amount: amount as i64, ..Trigger::new(HookEvent::Damaged) }, events);
        if died {
            events.push(GameEvent::UnitKilled { unit: target });
            let t = Trigger { unit: Some(target), pos: Some(pos), ..Trigger::new(HookEvent::Death) };
            self.fire(t, events);
            self.fire(Trigger { event: HookEvent::UnitKilled, ..t }, events);
        }
    }

    pub(super) fn damage_prop(&mut self, id: PropId, amount: i32, events: &mut Vec<GameEvent>) {
        let Some(p) = self.props.get_mut(id.0 as usize).filter(|p| p.alive) else { return };
        let Some(hp) = p.hp.as_mut() else { return }; // indestructible
        *hp -= amount.max(0);
        let left = *hp;
        events.push(GameEvent::PropDamaged { prop: id, hp: left.max(0) });
        self.fire(Trigger { prop: Some(id), amount: amount as i64, ..Trigger::new(HookEvent::Damaged) }, events);
        if left <= 0 {
            self.destroy_prop(id, events);
        }
    }

    pub(super) fn destroy_prop(&mut self, id: PropId, events: &mut Vec<GameEvent>) {
        let Some(p) = self.props.get_mut(id.0 as usize).filter(|p| p.alive) else { return };
        p.alive = false;
        let pos = p.pos;
        if p.cover != CoverLevel::None {
            self.map.set_terrain(pos, Terrain::Floor);
            self.map_version += 1;
        }
        events.push(GameEvent::PropDestroyed { prop: id });
        events.push(GameEvent::TerrainChanged { pos });
        self.fire(Trigger { prop: Some(id), pos: Some(pos), ..Trigger::new(HookEvent::Destroyed) }, events);
    }

    /// Damages every unit and destructible prop within `radius` of `center`.
    pub(super) fn explode(&mut self, center: GridPos, radius: f32, damage: i32, events: &mut Vec<GameEvent>) {
        events.push(GameEvent::Explosion { center, radius });
        let units: Vec<UnitId> = self.units.iter().filter(|u| u.is_alive() && u.pos.distance(center) <= radius).map(|u| u.id).collect();
        for u in units {
            self.apply_damage(u, damage, true, events);
        }
        self.damage_props_in_area(center, radius, damage, events);
        self.damage_walls_in_area(center, radius, damage, events);
    }

    /// Hit points of the terrain cover on `pos` (walls, trees, rocks...);
    /// `None` = not cover, or indestructible.
    pub fn wall_hp(&self, pos: GridPos) -> Option<i32> {
        if self.props.iter().any(|p| p.alive && p.pos == pos) {
            return None; // props have their own hit points
        }
        let max = match (self.map.terrain(pos), self.map.surface(pos)) {
            (Terrain::HighCover, Surface::Rock) => return None,
            (Terrain::HighCover, Surface::Container | Surface::Metal) => 10,
            (Terrain::HighCover, _) => 6,
            (Terrain::LowCover, Surface::Rock) => 8,
            (Terrain::LowCover, _) => 3,
            _ => return None,
        };
        Some(max - self.wall_damage.get(&pos).copied().unwrap_or(0))
    }

    /// Blast damage to walls and other terrain cover within `radius`. A
    /// broken wall collapses into rubble (half cover); broken low cover
    /// becomes open ground.
    pub(super) fn damage_walls_in_area(&mut self, center: GridPos, radius: f32, damage: i32, events: &mut Vec<GameEvent>) {
        let r = radius.ceil() as i32;
        for y in center.y - r..=center.y + r {
            for x in center.x - r..=center.x + r {
                let pos = GridPos::new(x, y);
                if !self.map.in_bounds(pos) || pos.distance(center) > radius || damage <= 0 {
                    continue;
                }
                let Some(hp) = self.wall_hp(pos) else { continue };
                let left = hp - damage;
                if left > 0 {
                    *self.wall_damage.entry(pos).or_insert(0) += damage;
                    events.push(GameEvent::WallDamaged { pos, hp: left });
                    continue;
                }
                self.wall_damage.remove(&pos);
                if self.map.terrain(pos) == Terrain::HighCover {
                    self.map.set_terrain(pos, Terrain::LowCover);
                    self.map.set_surface(pos, Surface::Rubble);
                } else {
                    self.map.set_terrain(pos, Terrain::Floor);
                    self.map.set_surface(pos, Surface::Dirt);
                }
                self.map_version += 1;
                events.push(GameEvent::WallDestroyed { pos });
                events.push(GameEvent::TerrainChanged { pos });
            }
        }
    }

    pub(super) fn damage_props_in_area(&mut self, center: GridPos, radius: f32, damage: i32, events: &mut Vec<GameEvent>) {
        let props: Vec<PropId> = self.props.iter().filter(|p| p.alive && p.hp.is_some() && p.pos.distance(center) <= radius).map(|p| p.id).collect();
        for p in props {
            self.damage_prop(p, damage, events);
        }
    }

    // ------------------------------------------------------------------
    // Movement / turn triggers
    // ------------------------------------------------------------------

    /// A unit arrived on `pos` (each step of a move).
    pub(super) fn on_enter(&mut self, unit: UnitId, pos: GridPos, events: &mut Vec<GameEvent>) {
        self.pick_up_supplies(unit, pos, events);
        self.fire(Trigger { unit: Some(unit), pos: Some(pos), ..Trigger::new(HookEvent::Enter) }, events);
        self.open_doors_near(pos, events);
        self.check_spotted(events);
    }

    /// A squad member stepping on a supply cache (`supply_ammo`,
    /// `supply_medkit`, `supply_grenade` props) takes it.
    fn pick_up_supplies(&mut self, unit: UnitId, pos: GridPos, events: &mut Vec<GameEvent>) {
        if self.units[unit.0 as usize].team != Team::Player {
            return;
        }
        let Some(i) = self.props.iter().position(|p| p.alive && p.pos == pos && p.kind.starts_with("supply")) else { return };
        let kind = self.props[i].kind.clone();
        let rules = &self.rules;
        let u = &mut self.units[unit.0 as usize];
        let text = match kind.as_str() {
            "supply_medkit" => {
                let healed = 4.min(u.stats.max_hp - u.hp).max(0);
                u.hp += healed;
                if healed > 0 { format!("+{healed} HP") } else { "medkit (full health)".into() }
            }
            "supply_grenade" => {
                // A charge for the first limited-use ability (grenades), else ammo.
                let limited = u.abilities.iter().find(|a| rules.ability(a).is_some_and(|d| d.charges.is_some())).cloned();
                match limited {
                    Some(a) => {
                        *u.charges.entry(a.clone()).or_insert(0) += 1;
                        let name = rules.ability(&a).map_or("grenade".to_string(), |d| d.name.to_lowercase());
                        format!("+1 {name}")
                    }
                    None => {
                        u.weapon.spare += 1;
                        "+1 magazine".into()
                    }
                }
            }
            _ => {
                u.weapon.spare += 1;
                u.weapon.ammo = u.weapon.clip_size.max(u.weapon.ammo);
                "+1 magazine, reloaded".into()
            }
        };
        self.props[i].alive = false;
        let prop = self.props[i].id;
        events.push(GameEvent::SupplyFound { unit, prop, text });
    }

    /// Turn-start hooks (mission + units of the new active team).
    pub(super) fn on_turn_start(&mut self, events: &mut Vec<GameEvent>) {
        self.fire(Trigger::new(HookEvent::TurnStart), events);
        self.check_spotted(events);
    }

    /// Detection and pod activation, then `spotted` hooks for enemies the
    /// player's squad sees for the first time.
    pub(super) fn check_spotted(&mut self, events: &mut Vec<GameEvent>) {
        self.check_awareness(events);
        let newly: Vec<UnitId> = self
            .living(Team::Enemy)
            .filter(|e| !self.spotted.contains(&e.id) && self.team_can_see(Team::Player, e.pos))
            .map(|e| e.id)
            .collect();
        for id in newly {
            self.spotted.insert(id);
            let pos = self.units[id.0 as usize].pos;
            self.fire(Trigger { unit: Some(id), pos: Some(pos), ..Trigger::new(HookEvent::Spotted) }, events);
        }
    }
}

#[cfg(all(test, feature = "missions", feature = "scripting"))]
mod tests {
    use super::*;
    use crate::action::{Action, Target};
    use crate::content::Ruleset;
    use crate::mission::parse_mission;
    use crate::modules::GrenadierModule;

    fn battle(xml: &str) -> (Battle, Vec<GameEvent>) {
        let mut rules = Ruleset::standard();
        rules.add_module(&GrenadierModule);
        let mission = parse_mission(xml).unwrap_or_else(|e| panic!("{e}"));
        let (mut b, warnings) = Battle::from_mission(rules, &mission, 1);
        assert!(warnings.is_empty(), "{warnings:?}");
        let events = b.begin();
        (b, events)
    }

    fn messages(events: &[GameEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                GameEvent::Message { text } => Some(text.clone()),
                GameEvent::ScriptError { message } => Some(format!("ERROR {message}")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn start_script_zone_enter_and_scripted_win() {
        let (mut b, events) = battle(
            r#"<mission id="m" win="script">
                 <layout>
                   ......
                   ......
                 </layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="1"/>
                 <unit team="enemy" class="sectoid" name="E" x="5" y="0"/>
                 <zone tag="exit" x="3" y="1" width="1" height="1">
                   <script on="enter" team="player">message("extracted " + unit_name(unit)); win();</script>
                 </zone>
                 <script on="start">message("go");</script>
               </mission>"#,
        );
        assert_eq!(messages(&events), ["go"]);
        let a = b.living(Team::Player).next().unwrap().id;
        // y=1 in XML is grid row 0 (2-row map).
        let events = b.apply(Action::Move { unit: a, to: GridPos::new(3, 0) }).unwrap();
        assert_eq!(messages(&events), ["extracted A"]);
        assert_eq!(b.winner(), Some(Team::Player));
        // The walk is reported before the script's effects.
        let moved = events.iter().position(|e| matches!(e, GameEvent::UnitMoved { .. })).unwrap();
        let msg = events.iter().position(|e| matches!(e, GameEvent::Message { .. })).unwrap();
        assert!(moved < msg);
    }

    #[test]
    fn grenade_blows_up_barrel_which_explodes_and_kills_via_scripts() {
        let (mut b, _) = battle(
            r#"<mission id="m">
                 <layout>
                   ........
                   ........
                   ........
                 </layout>
                 <unit team="player" class="grenadier" name="G" x="0" y="2"/>
                 <unit team="enemy" class="sectoid" name="Boss" x="6" y="0" tag="boss">
                   <script on="death">message("boss down");</script>
                 </unit>
                 <unit team="enemy" class="sectoid" name="Other" x="7" y="2"/>
                 <prop tag="barrel" kind="barrel" x="5" y="1" cover="half" hp="1">
                   <script on="destroyed">message("boom"); explode(x, y, 1.5, 99);</script>
                 </prop>
               </mission>"#,
        );
        let g = b.living(Team::Player).next().unwrap().id;
        let barrel_tile = b.props()[0].pos;
        assert_eq!(b.map().terrain(barrel_tile), Terrain::LowCover, "props occupy their tile");
        let events = b.apply(Action::ability(g, "frag_grenade", Target::Tile(barrel_tile))).unwrap();
        assert_eq!(messages(&events), ["boom", "boss down"]);
        assert!(!b.props()[0].alive);
        assert_eq!(b.map().terrain(barrel_tile), Terrain::Floor, "destroyed props free their tile");
        assert!(!b.unit(UnitId(1)).unwrap().is_alive());
        assert!(events.iter().any(|e| matches!(e, GameEvent::PropDestroyed { .. })));
        assert!(!b.is_over(), "one enemy left");
    }

    #[test]
    fn spotted_turn_start_flags_and_reinforcements() {
        // A wall in the top two rows hides the enemy; the bottom row is open.
        let (mut b, events) = battle(
            r#"<mission id="m">
                 <layout>
                   ..........H....
                   ..........H....
                   ...............
                 </layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="Hidden" x="14" y="0">
                   <script on="spotted">message("contact!");</script>
                 </unit>
                 <script on="turn_start" team="enemy">
                   set_flag("enemy_turns", flag("enemy_turns") + 1);
                   if flag("enemy_turns") == 1 { spawn_unit("enemy", "sectoid", "Reinforcement", 14, 1); }
                 </script>
               </mission>"#,
        );
        assert!(messages(&events).is_empty(), "behind the wall: not spotted yet");
        let events = b.apply(Action::EndTurn).unwrap();
        assert!(events.iter().any(|e| matches!(e, GameEvent::UnitSpawned { .. })));
        assert_eq!(b.flags().get("enemy_turns"), Some(&1));
        assert_eq!(b.living(Team::Enemy).count(), 2);
        b.apply(Action::EndTurn).unwrap();
        // Dash along the open bottom row (grid y = 0) past the wall: contact.
        let a = b.living(Team::Player).next().unwrap().id;
        let events = b.apply(Action::Move { unit: a, to: GridPos::new(11, 0) }).unwrap();
        assert!(messages(&events).contains(&"contact!".to_string()), "{:?}", messages(&events));
    }

    #[test]
    fn heights_limit_climbing_unless_ladder() {
        let (b, _) = battle(
            r#"<mission id="m">
                 <layout>
                   .....
                   .....
                 </layout>
                 <heights>
                   00222
                   00222
                 </heights>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="E" x="4" y="1"/>
               </mission>"#,
        );
        let a = b.living(Team::Player).next().unwrap().id;
        let range = b.movement_range(a);
        assert!(!range.contains(GridPos::new(2, 1)), "a 2-level cliff can't be climbed");
        let (b, _) = battle(
            r#"<mission id="m">
                 <layout>
                   .....
                   .....
                 </layout>
                 <heights>
                   00222
                   00222
                 </heights>
                 <ladder x="2" y="0"/>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="E" x="4" y="1"/>
               </mission>"#,
        );
        let a = b.living(Team::Player).next().unwrap().id;
        assert!(b.movement_range(a).contains(GridPos::new(3, 1)), "the ladder leads up");
    }

    #[test]
    fn broken_scripts_report_errors_and_the_battle_goes_on() {
        let (b, events) = battle(
            r#"<mission id="m">
                 <layout>....</layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="E" x="3" y="0"/>
                 <script on="start">this is not valid</script>
                 <script on="start">loop { }</script>
               </mission>"#,
        );
        let m = messages(&events);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|x| x.starts_with("ERROR")), "{m:?}");
        assert!(!b.is_over());
    }

    #[test]
    fn scripts_that_trigger_each_other_forever_are_cut_off() {
        // Taking damage deals damage again: an infinite chain.
        let (mut b, _) = battle(
            r#"<mission id="m">
                 <layout>......</layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="E" x="5" y="0">
                   <script on="damaged">damage(unit, 0);</script>
                 </unit>
               </mission>"#,
        );
        let mut events = Vec::new();
        b.apply_script_command(ScriptCommand::Damage { unit: 1, amount: 0 }, &mut events);
        assert!(events.iter().any(|e| matches!(e, GameEvent::ScriptError { message } if message.contains("deeply"))));
        assert!(b.unit(UnitId(1)).unwrap().is_alive());
    }
}

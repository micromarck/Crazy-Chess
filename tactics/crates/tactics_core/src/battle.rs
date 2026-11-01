//! The battle: authoritative game state and the rules that change it.
//!
//! [`Battle`] owns the map, every unit, the turn order and the RNG. Its public
//! surface is intentionally small:
//!
//! * **Queries** (`unit`, `movement_range`, `shot_preview`, `visible_enemies`,
//!   ...) are read-only and cheap enough to call every frame from a UI.
//! * [`Battle::validate`] says whether an [`Action`] is legal without changing
//!   anything.
//! * [`Battle::apply`] performs an action and returns the resulting
//!   [`GameEvent`]s. It is the **only** way to mutate the battle.
//!
//! Turn structure (XCOM-like):
//! 1. All units of the active team get [`Unit::MAX_ACTION_POINTS`].
//!    Their overwatch / hunker states from the previous turn expire.
//! 2. Units act in any order. Moving costs 1 AP per `mobility` worth of
//!    movement points; shooting, overwatch and hunkering consume all remaining AP.
//! 3. The turn ends when the team issues [`Action::EndTurn`] or automatically
//!    when no unit of the active team can act any more.

use crate::action::{Action, ActionError};
use crate::combat::{preview_shot, ShotPreview};
use crate::event::{GameEvent, ShotOutcome};
use crate::grid::{GridPos, Map};
use crate::pathfinding::{self, ReachableSet};
use crate::rng::GameRng;
use crate::unit::{Team, Unit, UnitId, UnitTemplate};
use crate::visibility;

/// Authoritative state of one tactical engagement.
#[derive(Debug, Clone)]
pub struct Battle {
    map: Map,
    /// Indexed by `UnitId.0`. Dead units stay in the list (hp <= 0) so ids remain stable.
    units: Vec<Unit>,
    active_team: Team,
    round: u32,
    rng: GameRng,
    winner: Option<Team>,
}

impl Battle {
    /// Creates an empty battle on `map`. The player team moves first.
    pub fn new(map: Map, seed: u64) -> Self {
        Self {
            map,
            units: Vec::new(),
            active_team: Team::Player,
            round: 1,
            rng: GameRng::new(seed),
            winner: None,
        }
    }

    /// Places a new unit on the map and returns its id.
    ///
    /// # Panics
    /// Panics if `pos` is not walkable or already occupied; spawning is a setup
    /// step and a bad spawn is a programming error, not a gameplay situation.
    pub fn spawn(&mut self, template: UnitTemplate, team: Team, pos: GridPos) -> UnitId {
        assert!(self.map.is_walkable(pos), "cannot spawn on non-walkable tile {pos}");
        assert!(self.unit_at(pos).is_none(), "tile {pos} already occupied");
        let id = UnitId(self.units.len() as u32);
        self.units.push(Unit {
            id,
            name: template.name,
            team,
            pos,
            hp: template.stats.max_hp,
            action_points: if team == self.active_team { Unit::MAX_ACTION_POINTS } else { 0 },
            stats: template.stats,
            weapon: template.weapon,
            status: Default::default(),
        });
        id
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn map(&self) -> &Map {
        &self.map
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

    /// Every unit ever spawned, dead ones included.
    pub fn units(&self) -> &[Unit] {
        &self.units
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
        pathfinding::reachable(&self.map, unit.pos, budget, |p| self.unit_at(p).is_none())
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

    /// Living enemies that `id` could legally shoot right now (ignoring AP/ammo).
    pub fn visible_enemies(&self, id: UnitId) -> Vec<UnitId> {
        let Some(shooter) = self.unit(id) else {
            return Vec::new();
        };
        self.living(shooter.team.opponent())
            .filter(|t| self.check_shot_geometry(shooter, t).is_ok())
            .map(|t| t.id)
            .collect()
    }

    /// Hit/crit preview for a shot. Fails if the shot would be illegal geometrically.
    pub fn shot_preview(&self, shooter: UnitId, target: UnitId) -> Result<ShotPreview, ActionError> {
        let s = self.living_unit(shooter)?;
        let t = self.living_unit(target)?;
        if s.team == t.team {
            return Err(ActionError::InvalidTarget(target));
        }
        self.check_shot_geometry(s, t)?;
        Ok(preview_shot(&self.map, s, t, false))
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

    /// Range and line-of-sight checks shared by normal and reaction fire.
    fn check_shot_geometry(&self, shooter: &Unit, target: &Unit) -> Result<(), ActionError> {
        let dist = shooter.pos.distance(target.pos);
        if dist > shooter.weapon.max_range || dist > shooter.stats.sight {
            return Err(ActionError::OutOfRange);
        }
        if !visibility::has_line_of_sight(&self.map, shooter.pos, target.pos) {
            return Err(ActionError::NoLineOfSight);
        }
        Ok(())
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

    /// Returns `Ok(())` if `action` could be applied right now.
    pub fn validate(&self, action: &Action) -> Result<(), ActionError> {
        if self.is_over() {
            return Err(ActionError::BattleOver);
        }
        match *action {
            Action::Move { unit, to } => {
                let u = self.ready_actor(unit)?;
                if to == u.pos || !self.movement_range(unit).contains(to) {
                    return Err(ActionError::Unreachable(to));
                }
                Ok(())
            }
            Action::Shoot { unit, target } => {
                let s = self.ready_actor(unit)?;
                if s.weapon.ammo <= 0 {
                    return Err(ActionError::OutOfAmmo);
                }
                let t = self.living_unit(target)?;
                if t.team == s.team {
                    return Err(ActionError::InvalidTarget(target));
                }
                self.check_shot_geometry(s, t)
            }
            Action::Overwatch { unit } => {
                let u = self.ready_actor(unit)?;
                if u.weapon.ammo <= 0 {
                    return Err(ActionError::OutOfAmmo);
                }
                Ok(())
            }
            Action::Reload { unit } => {
                let u = self.ready_actor(unit)?;
                if u.weapon.ammo >= u.weapon.clip_size {
                    return Err(ActionError::AlreadyFullAmmo);
                }
                Ok(())
            }
            Action::Hunker { unit } | Action::Wait { unit } => self.ready_actor(unit).map(|_| ()),
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
            Action::Shoot { unit, target } => {
                self.resolve_shot(unit, target, false, &mut events);
                self.spend_all_ap(unit, &mut events);
            }
            Action::Overwatch { unit } => {
                self.unit_mut(unit).status.overwatch = true;
                events.push(GameEvent::OverwatchSet { unit });
                self.spend_all_ap(unit, &mut events);
            }
            Action::Hunker { unit } => {
                self.unit_mut(unit).status.hunkered = true;
                events.push(GameEvent::Hunkered { unit });
                self.spend_all_ap(unit, &mut events);
            }
            Action::Reload { unit } => {
                let u = self.unit_mut(unit);
                u.weapon.ammo = u.weapon.clip_size;
                events.push(GameEvent::Reloaded { unit });
                self.spend_ap(unit, 1, &mut events);
            }
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

    /// Moves step by step so overwatchers can interrupt mid-path.
    fn do_move(&mut self, id: UnitId, to: GridPos, events: &mut Vec<GameEvent>) {
        let range = self.movement_range(id);
        // Validated above, so the path exists.
        let path = range.path_to(to).unwrap_or_default();
        let cost = range.cost(to).unwrap_or(0);
        let ap_cost = self.move_cost_ap(id, cost);
        // AP is paid up front: being interrupted does not refund it (XCOM rule).
        self.spend_ap(id, ap_cost, events);

        let mut segment_start = self.units[id.0 as usize].pos;
        let mut segment: Vec<GridPos> = Vec::new();
        for step in path {
            self.unit_mut(id).pos = step;
            segment.push(step);

            if let Some(watcher) = self.find_overwatcher(id) {
                // Flush the walked part first so the frontend animates in order.
                events.push(GameEvent::UnitMoved {
                    unit: id,
                    from: segment_start,
                    path: std::mem::take(&mut segment),
                });
                segment_start = step;
                self.unit_mut(watcher).status.overwatch = false;
                self.resolve_shot(watcher, id, true, events);
                if !self.units[id.0 as usize].is_alive() {
                    return;
                }
            }
        }
        if !segment.is_empty() {
            events.push(GameEvent::UnitMoved {
                unit: id,
                from: segment_start,
                path: segment,
            });
        }
    }

    /// First enemy on overwatch that can see and reach `mover` at its current tile.
    fn find_overwatcher(&self, mover: UnitId) -> Option<UnitId> {
        let m = &self.units[mover.0 as usize];
        self.living(m.team.opponent())
            .filter(|w| w.status.overwatch && w.weapon.ammo > 0)
            .find(|w| self.check_shot_geometry(w, m).is_ok())
            .map(|w| w.id)
    }

    /// Rolls a shot and applies damage. Does not touch action points.
    fn resolve_shot(&mut self, shooter: UnitId, target: UnitId, reaction: bool, events: &mut Vec<GameEvent>) {
        let preview = preview_shot(&self.map, &self.units[shooter.0 as usize], &self.units[target.0 as usize], reaction);
        self.unit_mut(shooter).weapon.ammo -= 1;

        let outcome = if self.rng.roll_percent(preview.hit_chance) {
            let critical = self.rng.roll_percent(preview.crit_chance);
            let weapon = &self.units[shooter.0 as usize].weapon;
            let (lo, hi, bonus) = (weapon.damage_min, weapon.damage_max, weapon.crit_bonus);
            let damage = self.rng.range_inclusive(lo, hi) + if critical { bonus } else { 0 };
            ShotOutcome::Hit { damage, critical }
        } else {
            ShotOutcome::Miss
        };

        events.push(GameEvent::ShotFired {
            shooter,
            target,
            hit_chance: preview.hit_chance,
            outcome,
            reaction,
        });

        if let ShotOutcome::Hit { damage, .. } = outcome {
            let t = self.unit_mut(target);
            t.hp -= damage;
            if t.hp <= 0 {
                t.hp = 0;
                t.action_points = 0;
                t.status = Default::default();
                events.push(GameEvent::UnitKilled { unit: target });
            }
        }
    }

    fn check_victory(&mut self, events: &mut Vec<GameEvent>) {
        if self.winner.is_some() {
            return;
        }
        for team in [Team::Player, Team::Enemy] {
            if self.living(team).next().is_none() {
                self.winner = Some(team.opponent());
                events.push(GameEvent::BattleEnded { winner: team.opponent() });
                return;
            }
        }
    }

    fn end_turn(&mut self, events: &mut Vec<GameEvent>) {
        // Units of the ending team lose any unused AP.
        for u in self.units.iter_mut().filter(|u| u.team == self.active_team) {
            u.action_points = 0;
        }
        self.active_team = self.active_team.opponent();
        if self.active_team == Team::Player {
            self.round += 1;
        }
        // Refresh the incoming team. Overwatch/hunker expire now (they protected
        // the unit during the opponent's turn and have done their job).
        let team = self.active_team;
        for u in self.units.iter_mut().filter(|u| u.team == team && u.is_alive()) {
            u.action_points = Unit::MAX_ACTION_POINTS;
            u.status.overwatch = false;
            u.status.hunkered = false;
        }
        events.push(GameEvent::TurnStarted { team, round: self.round });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::UnitTemplate;

    fn duel(art: &str) -> (Battle, UnitId, UnitId) {
        let map = Map::from_ascii(art);
        let mut b = Battle::new(map, 1234);
        let a = b.spawn(UnitTemplate::rifleman("A"), Team::Player, Map::find_markers(art, 'A')[0]);
        let e = b.spawn(UnitTemplate::alien_grunt("E"), Team::Enemy, Map::find_markers(art, 'E')[0]);
        (b, a, e)
    }

    #[test]
    fn move_costs_ap_and_dash_costs_two() {
        let (mut b, a, _) = duel(
            "
            E.........
            ..........
            A.........
            ",
        );
        // 3 orthogonal steps = 6 MP <= 12 mobility -> 1 AP.
        b.apply(Action::Move { unit: a, to: GridPos::new(3, 0) }).unwrap();
        assert_eq!(b.unit(a).unwrap().action_points, 1);

        let (mut b, a, _) = duel(
            "
            E.........
            ..........
            A.........
            ",
        );
        // 8 orthogonal steps = 16 MP > 12 -> 2 AP (dash), turn passes automatically.
        b.apply(Action::Move { unit: a, to: GridPos::new(8, 0) }).unwrap();
        assert_eq!(b.active_team(), Team::Enemy);
    }

    #[test]
    fn cannot_shoot_through_walls() {
        let (b, a, e) = duel(
            "
            ..E..
            HHHHH
            ..A..
            ",
        );
        assert_eq!(
            b.validate(&Action::Shoot { unit: a, target: e }),
            Err(ActionError::NoLineOfSight)
        );
    }

    #[test]
    fn shooting_uses_ammo_and_ends_unit_turn() {
        let (mut b, a, e) = duel(
            "
            ..E..
            .....
            ..A..
            ",
        );
        let events = b.apply(Action::Shoot { unit: a, target: e }).unwrap();
        assert!(events.iter().any(|ev| matches!(ev, GameEvent::ShotFired { .. })));
        assert_eq!(b.unit(a).unwrap().weapon.ammo, b.unit(a).unwrap().weapon.clip_size - 1);
        // Only unit on the team is out of AP, so the turn passed (unless the shot ended the battle).
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
        let (mut b, a, e) = duel(art);
        b.apply(Action::Overwatch { unit: a }).unwrap(); // also ends player turn
        assert_eq!(b.active_team(), Team::Enemy);
        let events = b.apply(Action::Move { unit: e, to: GridPos::new(4, 2) }).unwrap();
        let reaction = events
            .iter()
            .any(|ev| matches!(ev, GameEvent::ShotFired { reaction: true, .. }));
        assert!(reaction, "overwatch should fire: {events:?}");
        assert!(!b.unit(a).unwrap().status.overwatch, "overwatch is consumed");
    }

    #[test]
    fn same_seed_same_battle() {
        let run = || {
            let (mut b, a, e) = duel(
                "
                ..E..
                .....
                ..A..
                ",
            );
            let mut log = Vec::new();
            for _ in 0..6 {
                if b.is_over() {
                    break;
                }
                let action = match b.active_team() {
                    Team::Player => Action::Shoot { unit: a, target: e },
                    Team::Enemy => Action::Shoot { unit: e, target: a },
                };
                let action = if b.validate(&action) == Err(ActionError::OutOfAmmo) {
                    Action::Reload { unit: action.actor().unwrap() }
                } else {
                    action
                };
                log.extend(b.apply(action).unwrap());
            }
            log
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn killing_last_enemy_ends_battle() {
        let (mut b, a, e) = duel(
            "
            .E.
            ...
            .A.
            ",
        );
        b.units[e.0 as usize].hp = 1;
        // Close range, open ground: keep shooting (reloading as needed) until it lands.
        for _ in 0..20 {
            if b.is_over() {
                break;
            }
            if b.active_team() == Team::Enemy {
                b.apply(Action::EndTurn).unwrap();
                continue;
            }
            let shoot = Action::Shoot { unit: a, target: e };
            if b.validate(&shoot) == Err(ActionError::OutOfAmmo) {
                b.apply(Action::Reload { unit: a }).unwrap();
            } else {
                b.apply(shoot).unwrap();
            }
        }
        assert_eq!(b.winner(), Some(Team::Player));
    }
}

//! Symbiote mechanics: **mind control**, **rage**, **insanity** and
//! spreading **vines** (all data-driven; the Symbiotes module uses them).
//!
//! * **Mind control** (`AbilityEffect::MindControl`, script
//!   `mind_control`): the victim fights for the other side for a few of the
//!   controller's turns, then comes back to its senses. Killing the
//!   controller frees everyone it holds. A side only loses when none of its
//!   own are left: controlled soldiers still count as the player's.
//! * **Rage** (`UnitClass::rage`): hits and kindred deaths nearby build it,
//!   it cools each turn, and enraged units hit harder and move further.
//! * **Insanity** (`Unit::insanity`, 0-100): horrors (scripts'
//!   `add_insanity`, being mind-controlled, friends dying, vines) wear
//!   soldiers' minds down. Disturbed (30+) and Unhinged (60+) cost aim;
//!   an Unhinged soldier may **snap** at the start of a turn (the enemy has
//!   them until the end of its next turn); at 100 they're **lost** for good.
//!   Hunkering down steadies the mind a little.
//! * **Vines** (`Battle::vines`): symbiote growth on the floor. It spreads a
//!   tile at a time (script `set_vine_growth`), lashes and unnerves anyone
//!   not vine-kin who starts a turn on it, heals symbiotes, and **burns**:
//!   fire on a vine burns it away and runs along the growth.

use std::collections::BTreeSet;

use super::Battle;
use crate::event::GameEvent;
use crate::grid::{GridPos, Terrain};
use crate::unit::{Sanity, Team, UnitId};

/// Most vine tiles a map can hold (growth stops there).
const MAX_VINES: usize = 1500;
/// Insanity for allies when a soldier dies within this many tiles / further away.
const DEATH_NEAR: f32 = 6.0;
const DEATH_INSANITY_NEAR: i32 = 8;
const DEATH_INSANITY_FAR: i32 = 3;
/// Starting a turn on vines (not vine-kin).
const VINE_INSANITY: i32 = 4;
const VINE_DAMAGE: i32 = 1;
/// Hunkering down steadies the mind.
pub(super) const HUNKER_CALM: i32 = 6;

impl Battle {
    // ------------------------------------------------------------------
    // Mind control
    // ------------------------------------------------------------------

    /// Takes `target`'s mind for `turns` of `by_team`'s turns.
    pub(super) fn mind_control(&mut self, target: UnitId, by: Option<UnitId>, by_team: Team, turns: u32, events: &mut Vec<GameEvent>) {
        let active = self.active_team;
        let ap = self.rules.combat.action_points;
        let Some(u) = self.units.get_mut(target.0 as usize).filter(|u| u.is_alive()) else { return };
        if u.team == by_team || turns == 0 {
            return;
        }
        let home = u.home_team();
        u.status.home_team = Some(home);
        u.team = by_team;
        u.status.controlled = turns;
        u.status.controlled_by = by;
        u.status.overwatch = None;
        u.status.suppressed_by = None;
        u.status.defense_bonus = 0;
        // Taken on its new side's turn: it acts for them at once.
        u.action_points = if by_team == active && u.status.stunned == 0 { ap } else { 0 };
        let remaining = u.action_points;
        events.push(GameEvent::UnitControlled { unit: target, team: by_team, turns });
        events.push(GameEvent::Callout { unit: target, text: "MIND CONTROLLED".into() });
        events.push(GameEvent::ActionPointsChanged { unit: target, remaining });
    }

    /// Gives a controlled unit its mind back (it sits out the rest of this turn).
    pub(super) fn release_mind(&mut self, target: UnitId, events: &mut Vec<GameEvent>) {
        let active = self.active_team;
        let Some(u) = self.units.get_mut(target.0 as usize) else { return };
        let Some(home) = u.status.home_team.take() else { return };
        u.team = home;
        u.status.controlled = 0;
        u.status.controlled_by = None;
        u.status.overwatch = None;
        if u.team != active {
            u.action_points = 0;
        }
        if u.is_alive() {
            events.push(GameEvent::UnitControlled { unit: target, team: home, turns: 0 });
            events.push(GameEvent::Callout { unit: target, text: "FREED".into() });
        }
    }

    /// Start of `team`'s turn: the minds it holds tick down; spent ones come back.
    pub(super) fn control_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let ids: Vec<UnitId> =
            self.units.iter().filter(|u| u.is_alive() && u.team == team && u.status.controlled > 0).map(|u| u.id).collect();
        for id in ids {
            let u = &mut self.units[id.0 as usize];
            if u.status.controlled != u32::MAX {
                u.status.controlled -= 1;
            }
            if u.status.controlled == 0 {
                self.release_mind(id, events);
            }
        }
    }

    /// `dead` died: everyone it held is freed, and a dead puppet is its own again.
    pub(super) fn control_on_death(&mut self, dead: UnitId, events: &mut Vec<GameEvent>) {
        let held: Vec<UnitId> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.status.controlled_by == Some(dead) && u.status.controlled != u32::MAX)
            .map(|u| u.id)
            .collect();
        for id in held {
            self.release_mind(id, events);
        }
    }

    // ------------------------------------------------------------------
    // Insanity
    // ------------------------------------------------------------------

    /// Changes a unit's insanity (only those whose mind can give way). At
    /// 100 the unit is lost to the other side for good.
    pub(super) fn add_insanity(&mut self, id: UnitId, amount: i32, events: &mut Vec<GameEvent>) {
        let Some(u) = self.units.get_mut(id.0 as usize).filter(|u| u.is_alive() && u.can_go_insane()) else { return };
        if amount == 0 || u.sanity() == Sanity::Lost {
            return;
        }
        let before = u.sanity();
        u.insanity = (u.insanity + amount).clamp(0, 100);
        let after = u.sanity();
        let insanity = u.insanity;
        if before != after {
            events.push(GameEvent::InsanityChanged { unit: id, insanity, level: after.label().into() });
            if after > before {
                events.push(GameEvent::Callout { unit: id, text: after.label().to_uppercase() });
            }
        }
        if after == Sanity::Lost {
            let enemy = self.units[id.0 as usize].home_team().opponent();
            events.push(GameEvent::Callout { unit: id, text: "LOST".into() });
            if self.units[id.0 as usize].team == enemy {
                // Already held: now for good.
                self.units[id.0 as usize].status.controlled = u32::MAX;
            } else {
                self.mind_control(id, None, enemy, u32::MAX, events);
            }
        }
    }

    /// A soldier died: friends nearby (and, less, everyone else) take it hard.
    pub(super) fn insanity_on_death(&mut self, dead: UnitId, events: &mut Vec<GameEvent>) {
        let d = &self.units[dead.0 as usize];
        if !d.can_go_insane() {
            return;
        }
        let (home, pos) = (d.home_team(), d.pos);
        let friends: Vec<(UnitId, bool)> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.id != dead && u.home_team() == home && u.can_go_insane())
            .map(|u| (u.id, u.pos.distance(pos) <= DEATH_NEAR))
            .collect();
        for (f, near) in friends {
            self.add_insanity(f, if near { DEATH_INSANITY_NEAR } else { DEATH_INSANITY_FAR }, events);
        }
    }

    /// Start of `team`'s turn: its unhinged members may snap (the other
    /// side has them until the end of its next turn).
    pub(super) fn insanity_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let shaky: Vec<(UnitId, i32)> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.team == team && u.status.controlled == 0 && u.sanity() == Sanity::Unhinged)
            .map(|u| (u.id, u.insanity))
            .collect();
        for (id, insanity) in shaky {
            // 60 -> 10%, 99 -> 49%.
            if self.rng.roll_percent(insanity - 50) {
                events.push(GameEvent::Callout { unit: id, text: "SNAPS!".into() });
                // 2: released at the start of the enemy's turn after next.
                self.mind_control(id, None, team.opponent(), 2, events);
            }
        }
    }

    // ------------------------------------------------------------------
    // Rage
    // ------------------------------------------------------------------

    /// Adds rage to a unit that has rage rules (announces becoming enraged).
    pub(super) fn add_rage(&mut self, id: UnitId, amount: i32, events: &mut Vec<GameEvent>) {
        let Some(u) = self.units.get_mut(id.0 as usize).filter(|u| u.is_alive() && u.rage_rules.is_some()) else { return };
        let before = u.enraged();
        u.rage = (u.rage + amount).clamp(0, 100);
        let after = u.enraged();
        if before != after {
            events.push(GameEvent::Callout { unit: id, text: if after { "ENRAGED".into() } else { "calms".into() } });
        }
    }

    /// Rage for a hit taken.
    pub(super) fn rage_on_hit(&mut self, id: UnitId, events: &mut Vec<GameEvent>) {
        if let Some(r) = self.units[id.0 as usize].rage_rules {
            self.add_rage(id, r.per_hit, events);
        }
    }

    /// One of their kind died: raging kin nearby.
    pub(super) fn rage_on_death(&mut self, dead: UnitId, events: &mut Vec<GameEvent>) {
        let d = &self.units[dead.0 as usize];
        let (home, pos) = (d.home_team(), d.pos);
        let kin: Vec<(UnitId, i32)> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.id != dead && u.home_team() == home && u.pos.distance(pos) <= 8.0)
            .filter_map(|u| u.rage_rules.map(|r| (u.id, r.per_ally_death)))
            .collect();
        for (k, amount) in kin {
            self.add_rage(k, amount, events);
        }
    }

    /// Start of `team`'s turn: rage cools.
    pub(super) fn rage_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let ids: Vec<(UnitId, i32)> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.team == team && u.rage > 0)
            .filter_map(|u| u.rage_rules.map(|r| (u.id, r.decay)))
            .collect();
        for (id, decay) in ids {
            self.add_rage(id, -decay, events);
        }
    }

    // ------------------------------------------------------------------
    // Vines
    // ------------------------------------------------------------------

    /// Tiles covered in symbiote vines.
    pub fn vines(&self) -> &BTreeSet<GridPos> {
        &self.vines
    }

    /// Are there vines on `pos`?
    pub fn has_vines(&self, pos: GridPos) -> bool {
        self.vines.contains(&pos)
    }

    fn vine_ground(&self, p: GridPos) -> bool {
        self.map.in_bounds(p) && self.map.terrain(p) == Terrain::Floor && !self.fires.contains_key(&p)
    }

    /// Grows vines on the floor within `radius` of `center`.
    pub(super) fn plant_vines(&mut self, center: GridPos, radius: f32, events: &mut Vec<GameEvent>) {
        let r = radius.max(0.0).ceil() as i32;
        let mut grown = Vec::new();
        for y in center.y - r..=center.y + r {
            for x in center.x - r..=center.x + r {
                let p = GridPos::new(x, y);
                if center.distance(p) <= radius + 0.01 && self.vine_ground(p) && self.vines.len() < MAX_VINES && self.vines.insert(p) {
                    grown.push(p);
                }
            }
        }
        if !grown.is_empty() {
            events.push(GameEvent::VinesGrew { tiles: grown });
        }
    }

    /// Clears vines within `radius` of `center` (scripts).
    pub(super) fn clear_vines(&mut self, center: GridPos, radius: f32, events: &mut Vec<GameEvent>) {
        let gone: Vec<GridPos> = self.vines.iter().copied().filter(|p| center.distance(*p) <= radius + 0.01).collect();
        for p in &gone {
            self.vines.remove(p);
        }
        if !gone.is_empty() {
            events.push(GameEvent::VinesBurned { tiles: gone });
        }
    }

    /// Start of each round (the enemy's turn): vines creep onto neighbouring
    /// floor, `vine_growth` percent per free side.
    pub(super) fn vines_grow(&mut self, events: &mut Vec<GameEvent>) {
        if self.vines.is_empty() || self.vine_growth == 0 || self.vines.len() >= MAX_VINES {
            return;
        }
        let mut grown = Vec::new();
        let edge: Vec<GridPos> = self.vines.iter().copied().collect();
        for p in edge {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let q = GridPos::new(p.x + dx, p.y + dy);
                if self.vines.contains(&q) || !self.vine_ground(q) || !self.map.is_walkable(q) {
                    continue;
                }
                if self.rng.roll_percent(self.vine_growth as i32) && self.vines.len() + grown.len() < MAX_VINES {
                    grown.push(q);
                }
            }
        }
        grown.sort_by_key(|p| (p.y, p.x));
        grown.dedup();
        for p in &grown {
            self.vines.insert(*p);
        }
        if !grown.is_empty() {
            events.push(GameEvent::VinesGrew { tiles: grown });
        }
    }

    /// Start of `team`'s turn on vines: symbiotes heal, everyone else is
    /// lashed and unnerved.
    pub(super) fn vines_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        if self.vines.is_empty() {
            return;
        }
        let on: Vec<(UnitId, bool)> = self
            .living(team)
            .filter(|u| u.machine == crate::unit::Machine::None && u.tiles().any(|p| self.vines.contains(&p)))
            .map(|u| (u.id, u.vine_kin))
            .collect();
        for (id, kin) in on {
            if self.is_over() {
                return;
            }
            if kin {
                let u = &mut self.units[id.0 as usize];
                if u.hp < u.stats.max_hp {
                    u.hp += 1;
                    events.push(GameEvent::UnitHealed { unit: id, amount: 1 });
                }
            } else {
                events.push(GameEvent::Callout { unit: id, text: "TENDRILS".into() });
                self.add_insanity(id, VINE_INSANITY, events);
                self.apply_damage(id, VINE_DAMAGE, true, events);
            }
        }
    }

    /// Round start, with the fire: burning vines burn away, and the fire
    /// runs along the growth to the vines next to them.
    pub(super) fn vines_burn(&mut self, events: &mut Vec<GameEvent>) {
        let burning: Vec<GridPos> = self.vines.iter().copied().filter(|p| self.fires.contains_key(p)).collect();
        if burning.is_empty() {
            return;
        }
        let mut spread = Vec::new();
        for p in &burning {
            self.vines.remove(p);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
                let q = GridPos::new(p.x + dx, p.y + dy);
                if self.vines.contains(&q) && !self.fires.contains_key(&q) {
                    spread.push(q);
                }
            }
        }
        events.push(GameEvent::VinesBurned { tiles: burning });
        spread.sort_by_key(|p| (p.y, p.x));
        spread.dedup();
        if !spread.is_empty() {
            self.ignite(&spread, 2, events);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::battle::Battle;
    use crate::content::{AbilityDef, AbilityEffect, AbilityId, AbilityRange, ApCost, Ruleset, Targeting};
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::unit::{RageRules, Sanity, Team};

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_ability(
            AbilityDef::new("dominate", "Dominate")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::Enemy, AbilityRange::Fixed(10.0))
                .with_effect(AbilityEffect::MindControl { turns: 2, insanity: 10 }),
        );
        let mut p = r.class("rifleman").unwrap().clone();
        p.id = "puppeteer".into();
        p.fearless = true;
        p.abilities = vec![AbilityId::new("dominate")];
        r.add_class(p);
        let mut s = r.class("rifleman").unwrap().clone();
        s.id = "symbiote".into();
        s.fearless = true;
        s.flammable = true;
        s.vine_kin = true;
        s.rage = Some(RageRules::default());
        r.add_class(s);
        r
    }

    fn open(w: usize, h: usize) -> Map {
        Map::from_ascii(&format!("{}\n", ".".repeat(w)).repeat(h))
    }

    #[test]
    fn mind_control_turns_a_soldier_for_two_turns_and_killing_the_puppeteer_frees_them() {
        let mut b = Battle::new(open(12, 5), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 2)).unwrap();
        let c = b.spawn("rifleman", "C", Team::Player, GridPos::new(1, 0)).unwrap();
        let p = b.spawn("puppeteer", "P", Team::Enemy, GridPos::new(8, 2)).unwrap();
        b.apply(Action::EndTurn).unwrap(); // the enemy's turn
        b.apply(Action::ability(p, "dominate", Target::Unit(a))).unwrap();
        let u = b.unit(a).unwrap();
        assert_eq!((u.team, u.home_team()), (Team::Enemy, Team::Player));
        assert!(u.action_points > 0, "it acts for them at once");
        assert_eq!(u.insanity, 10);
        // Through the next enemy turn, then home.
        b.apply(Action::EndTurn).unwrap(); // player
        b.apply(Action::EndTurn).unwrap(); // enemy (2 -> 1)
        assert_eq!(b.unit(a).unwrap().team, Team::Enemy);
        b.apply(Action::EndTurn).unwrap(); // player
        b.apply(Action::EndTurn).unwrap(); // enemy (1 -> 0: freed)
        assert_eq!(b.unit(a).unwrap().team, Team::Player);
        // Again, then kill the puppeteer: freed at once.
        b.apply(Action::ability(p, "dominate", Target::Unit(a))).unwrap();
        assert_eq!(b.unit(a).unwrap().team, Team::Enemy);
        b.apply(Action::EndTurn).unwrap();
        let mut events = Vec::new();
        b.apply_damage(p, 99, true, &mut events);
        assert_eq!(b.unit(a).unwrap().team, Team::Player);
        assert!(events.iter().any(|e| matches!(e, GameEvent::UnitControlled { turns: 0, .. })));
        let _ = c;
    }

    #[test]
    fn a_squad_isnt_beaten_while_its_soldiers_are_only_controlled() {
        let mut b = Battle::new(open(12, 3), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
        let p = b.spawn("puppeteer", "P", Team::Enemy, GridPos::new(8, 1)).unwrap();
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::ability(p, "dominate", Target::Unit(a))).unwrap();
        assert!(!b.is_over(), "the only soldier is controlled, not gone");
    }

    #[test]
    fn insanity_costs_aim_and_at_100_the_soldier_is_lost() {
        let mut b = Battle::new(open(10, 3), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
        b.spawn("rifleman", "B", Team::Player, GridPos::new(1, 0)).unwrap();
        let e = b.spawn("rifleman", "E", Team::Enemy, GridPos::new(6, 1)).unwrap();
        let aim = |b: &Battle| b.preview(a, &AbilityId::new("shoot"), Target::Unit(e)).unwrap()[0].1.aim_chance;
        let sane = aim(&b);
        let mut ev = Vec::new();
        b.add_insanity(a, 40, &mut ev);
        assert_eq!(b.unit(a).unwrap().sanity(), Sanity::Disturbed);
        assert!(aim(&b) < sane);
        // Hunkering steadies the mind.
        b.apply(Action::ability(a, "hunker", Target::None)).unwrap();
        assert!(b.unit(a).unwrap().insanity < 40);
        b.add_insanity(a, 100, &mut ev);
        let u = b.unit(a).unwrap();
        assert_eq!(u.sanity(), Sanity::Lost);
        assert_eq!(u.team, Team::Enemy, "lost to the other side");
        assert_eq!(u.status.controlled, u32::MAX);
        // Monsters and machines don't go mad.
        let s = b.spawn("symbiote", "S", Team::Enemy, GridPos::new(8, 2)).unwrap();
        b.add_insanity(s, 50, &mut ev);
        assert_eq!(b.unit(s).unwrap().insanity, 0);
    }

    #[test]
    fn unhinged_soldiers_sometimes_snap() {
        let mut snapped = 0;
        for seed in 0..30 {
            let mut b = Battle::new(open(10, 3), rules(), seed);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
            b.spawn("rifleman", "B", Team::Player, GridPos::new(1, 0)).unwrap();
            b.spawn("rifleman", "E", Team::Enemy, GridPos::new(9, 2)).unwrap();
            let mut ev = Vec::new();
            b.add_insanity(a, 95, &mut ev);
            b.apply(Action::EndTurn).unwrap();
            b.apply(Action::EndTurn).unwrap(); // player turn start: a 45% snap
            if b.unit(a).unwrap().team == Team::Enemy {
                snapped += 1;
            }
        }
        assert!((5..=25).contains(&snapped), "{snapped}/30");
    }

    #[test]
    fn rage_builds_with_hits_and_hits_harder() {
        let mut b = Battle::new(open(10, 3), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
        let s = b.spawn("symbiote", "S", Team::Enemy, GridPos::new(5, 1)).unwrap();
        let dmg = |b: &Battle| b.preview(s, &AbilityId::new("shoot"), Target::Unit(a)).unwrap()[0].1.damage;
        let calm = dmg(&b);
        let calm_move = b.unit(s).unwrap().mobility();
        let mut ev = Vec::new();
        b.apply_damage(s, 1, true, &mut ev);
        assert!(!b.unit(s).unwrap().enraged());
        b.apply_damage(s, 1, true, &mut ev);
        assert!(b.unit(s).unwrap().enraged(), "two hits: 60 rage");
        assert!(ev.iter().any(|e| matches!(e, GameEvent::Callout { text, .. } if text == "ENRAGED")));
        assert_eq!(dmg(&b).0, calm.0 + 2);
        assert!(b.unit(s).unwrap().mobility() > calm_move);
    }

    #[test]
    fn vines_spread_lash_soldiers_heal_symbiotes_and_burn_along_the_growth() {
        let mut b = Battle::new(open(16, 5), rules(), 3);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(2, 2)).unwrap();
        let s = b.spawn("symbiote", "S", Team::Enemy, GridPos::new(12, 2)).unwrap();
        b.spawn("rifleman", "Spare", Team::Enemy, GridPos::new(15, 0)).unwrap();
        let mut ev = Vec::new();
        b.plant_vines(GridPos::new(2, 2), 1.0, &mut ev);
        b.plant_vines(GridPos::new(12, 2), 1.0, &mut ev);
        assert!(b.has_vines(GridPos::new(2, 2)));
        b.vine_growth = 100;
        let before = b.vines().len();
        let hp = b.unit(a).unwrap().hp;
        b.debug_set_hp(s, 3);
        b.apply(Action::EndTurn).unwrap(); // enemy turn: growth, the symbiote heals
        assert!(b.vines().len() > before, "they spread");
        assert_eq!(b.unit(s).unwrap().hp, 4);
        b.apply(Action::EndTurn).unwrap(); // player's turn: tendrils
        let u = b.unit(a).unwrap();
        assert_eq!(u.hp, hp - 1);
        assert!(u.insanity > 0);
        // Fire on one vine runs along the growth.
        b.vine_growth = 0;
        let tile = GridPos::new(12, 2);
        b.ignite(&[tile], 3, &mut ev);
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap(); // round start: burns and spreads
        assert!(!b.has_vines(tile), "burned away");
        assert!(b.on_fire(GridPos::new(13, 2)) || b.on_fire(GridPos::new(11, 2)), "the fire runs along the vines");
    }

    #[test]
    fn flammable_units_burn_twice_as_badly() {
        let mut b = Battle::new(open(10, 3), rules(), 1);
        b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
        let s = b.spawn("symbiote", "S", Team::Enemy, GridPos::new(5, 1)).unwrap();
        let r = b.spawn("rifleman", "R", Team::Enemy, GridPos::new(7, 1)).unwrap();
        let mut ev = Vec::new();
        b.ignite(&[GridPos::new(5, 1), GridPos::new(7, 1)], 3, &mut ev);
        let (hs, hr) = (b.unit(s).unwrap().hp, b.unit(r).unwrap().hp);
        b.apply(Action::EndTurn).unwrap(); // enemy turn starts in the fire
        let fire = b.rules().combat.fire_damage;
        assert_eq!(b.unit(r).unwrap().hp, hr - fire);
        assert_eq!(b.unit(s).unwrap().hp, hs - 2 * fire);
    }
}

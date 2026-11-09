//! Fear: when it rises and falls (numbers in
//! [`MoraleRules`](crate::content::MoraleRules)).
//!
//! | trigger | who | fear |
//! |---|---|---|
//! | took damage | the victim | `+per_damage` per HP |
//! | an attack missed them | the target | `+shot_at` |
//! | an ally died | allies within `near_radius` / everyone else | `+ally_killed_near` / `+ally_killed_far` |
//! | landed a kill | the killer / its allies nearby | `-kill_relief` / `-ally_kill_relief` |
//! | hunkered down | the unit | `-hunker_relief` |
//! | own turn starts | each unit | `-recovery` (`-hidden_recovery` more out of sight), `+flanked` if flanked, `+outnumbered` per extra enemy in sight |
//!
//! Gains are scaled by bravery; fearless units (robots, drones) ignore all of
//! it. Crossing a threshold emits [`GameEvent::MoraleChanged`].

use super::Battle;
use crate::cover::cover_against;
use crate::event::GameEvent;
use crate::unit::{Team, UnitId};
use crate::visibility;

impl Battle {
    /// Sets a unit's fear directly (tests, tools, debug consoles). Prefer
    /// scripts' `add_fear` in content.
    pub fn debug_set_fear(&mut self, id: UnitId, fear: i32) {
        if let Some(u) = self.units.get_mut(id.0 as usize) {
            u.fear = fear.clamp(0, 100);
        }
    }

    /// Veterancy and other between-mission bonuses: adds `m`'s aim, defence,
    /// mobility, sight and max HP to a unit (healing it by the extra HP).
    pub fn apply_bonus(&mut self, id: UnitId, m: crate::content::StatModifiers) {
        if let Some(u) = self.units.get_mut(id.0 as usize).filter(|u| u.is_alive()) {
            u.stats.aim += m.aim;
            u.stats.defense += m.defense;
            u.stats.mobility = (u.stats.mobility as i32 + m.mobility).max(2) as u32;
            u.stats.sight = (u.stats.sight + m.sight).max(1.0);
            u.stats.max_hp = (u.stats.max_hp + m.max_hp).max(1);
            u.hp = (u.hp + m.max_hp).clamp(1, u.stats.max_hp);
        }
    }

    /// Gives a unit an ability it may not have (skills learned between
    /// missions). With charges, adds `charges` uses (on top of any it has).
    pub fn grant_ability(&mut self, id: UnitId, ability: &crate::content::AbilityId, charges: u32) {
        let Some(def) = self.rules.ability(ability).cloned() else { return };
        let Some(u) = self.units.get_mut(id.0 as usize).filter(|u| u.is_alive()) else { return };
        let had = u.abilities.contains(ability);
        if !had {
            u.abilities.push(ability.clone());
        }
        if let Some(max) = def.charges {
            let left = if had { u.charges.get(ability).copied().unwrap_or(max) } else { 0 };
            u.charges.insert(ability.clone(), left + charges.max(1));
        }
    }

    /// Sets a unit's HP directly (tests, tools).
    pub fn debug_set_hp(&mut self, id: UnitId, hp: i32) {
        if let Some(u) = self.units.get_mut(id.0 as usize) {
            u.hp = hp.clamp(1, u.stats.max_hp);
        }
    }

    /// Changes a unit's fear (gains scaled by bravery). Ignored for dead or
    /// fearless units and when morale is disabled.
    pub(super) fn add_fear(&mut self, id: UnitId, amount: i32, events: &mut Vec<GameEvent>) {
        let rules = &self.rules.combat.morale;
        if !rules.enabled || amount == 0 {
            return;
        }
        let Some(u) = self.units.get_mut(id.0 as usize).filter(|u| u.is_alive() && !u.fearless) else { return };
        let before = rules.level(u.fear);
        u.fear = (u.fear + rules.scaled(amount, u.stats.bravery)).clamp(0, 100);
        let after = rules.level(u.fear);
        if before != after {
            events.push(GameEvent::MoraleChanged { unit: id, level: after, fear: u.fear });
        }
    }

    /// A unit died: its allies are frightened (more if close), the killer
    /// and its friends nearby take heart.
    pub(super) fn fear_on_death(&mut self, dead: UnitId, killer: Option<UnitId>, events: &mut Vec<GameEvent>) {
        let (near, near_gain, far_gain, relief, ally_relief) = {
            let m = &self.rules.combat.morale;
            (m.near_radius, m.ally_killed_near, m.ally_killed_far, m.kill_relief, m.ally_kill_relief)
        };
        let d = &self.units[dead.0 as usize];
        let (team, pos) = (d.team, d.pos);
        let allies: Vec<(UnitId, bool)> =
            self.living(team).map(|u| (u.id, u.pos.distance(pos) <= near)).collect();
        for (a, close) in allies {
            self.add_fear(a, if close { near_gain } else { far_gain }, events);
        }
        if let Some(k) = killer.filter(|k| self.units[k.0 as usize].team != team) {
            self.add_fear(k, -relief, events);
            let kpos = self.units[k.0 as usize].pos;
            let friends: Vec<UnitId> =
                self.living(team.opponent()).filter(|u| u.id != k && u.pos.distance(kpos) <= near).map(|u| u.id).collect();
            for f in friends {
                self.add_fear(f, -ally_relief, events);
            }
        }
    }

    /// Turn-start morale for `team`: recovery, flanking and odds; then
    /// panicked units lose action points.
    pub(super) fn morale_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let m = self.rules.combat.morale.clone();
        if !m.enabled {
            return;
        }
        let ids: Vec<UnitId> = self.living(team).filter(|u| !u.fearless).map(|u| u.id).collect();
        for id in ids {
            let u = &self.units[id.0 as usize];
            let watchers: Vec<_> = self
                .living(team.opponent())
                .filter(|e| !e.ai.dormant && self.can_unit_see(e, u.pos))
                .map(|e| e.pos)
                .collect();
            let flanked = watchers.iter().any(|&e| cover_against(&self.map, u.pos, e).flanked);
            let friends = self
                .living(team)
                .filter(|a| a.pos.distance(u.pos) <= m.near_radius && visibility::has_sight(&self.map, a.pos, u.pos))
                .count() as i32; // includes itself
            let extra = (watchers.len() as i32 - friends).max(0);
            let mut delta = -m.recovery + extra * m.outnumbered;
            if watchers.is_empty() {
                delta -= m.hidden_recovery;
            }
            if flanked {
                delta += m.flanked;
            }
            // Apply relief and gains separately so only gains are scaled.
            let (gain, relief) = if delta > 0 { (delta, 0) } else { (0, delta) };
            self.add_fear(id, relief, events);
            self.add_fear(id, gain, events);

            let u = &mut self.units[id.0 as usize];
            if m.level(u.fear) == crate::content::FearLevel::Panicked && m.panicked_ap_loss > 0 && u.action_points > 1 {
                u.action_points = u.action_points.saturating_sub(m.panicked_ap_loss).max(1);
                events.push(GameEvent::ActionPointsChanged { unit: id, remaining: u.action_points });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::battle::Battle;
    use crate::content::{FearLevel, Ruleset};
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::modules::GrenadierModule;
    use crate::unit::{Team, UnitId};

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    #[test]
    fn deaths_frighten_allies_but_not_robots() {
        let mut r = rules();
        let mut robot = r.class("rifleman").unwrap().clone();
        robot.id = "robot".into();
        robot.fearless = true;
        r.add_class(robot);
        let mut b = Battle::new(Map::from_ascii(".........."), r, 3);
        let shooter = b.spawn("sniper", "S", Team::Enemy, GridPos::new(9, 0)).unwrap();
        let victim = b.spawn("rifleman", "V", Team::Player, GridPos::new(2, 0)).unwrap();
        let buddy = b.spawn("rifleman", "B", Team::Player, GridPos::new(1, 0)).unwrap();
        let bot = b.spawn("robot", "R", Team::Player, GridPos::new(0, 0)).unwrap();
        let mut events = Vec::new();
        b.apply_damage(victim, 99, true, &mut events);
        b.fear_on_death(victim, Some(shooter), &mut events);
        assert!(b.unit(buddy).unwrap().fear >= 25, "a nearby ally is frightened");
        assert_eq!(b.unit(bot).unwrap().fear, 0, "robots feel nothing");
        let _ = events;
    }

    #[test]
    fn fear_lowers_aim_costs_ap_and_hunkering_helps() {
        let mut b = Battle::new(Map::from_ascii("........"), rules(), 5);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(6, 0)).unwrap();
        let calm = b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.aim_chance;
        let mut events = Vec::new();
        b.add_fear(a, 40, &mut events); // bravery 50: x1
        assert!(events.iter().any(|ev| matches!(ev, GameEvent::MoraleChanged { level: FearLevel::Shaken, .. })));
        let shaken = b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.aim_chance;
        // Shaken: -10 aim, scaled by the global accuracy multiplier.
        let expected = 10.0 * b.rules().combat.accuracy_multiplier;
        assert!(((calm - shaken) as f32 - expected).abs() <= 1.0, "{calm} - {shaken}");
        b.add_fear(a, 60, &mut events);
        assert_eq!(b.rules().combat.morale.level(b.unit(a).unwrap().fear), FearLevel::Panicked);
        // Panic costs an action point next turn.
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(b.unit(a).unwrap().action_points, 1);
        let before = b.unit(a).unwrap().fear;
        b.apply(Action::ability(a, "hunker", Target::None)).unwrap();
        assert!(b.unit(UnitId(0)).unwrap().fear < before);
    }

    #[test]
    fn loadouts_change_gear_stats_and_charges() {
        let mut b = Battle::new(Map::from_ascii("........"), rules(), 5);
        let over = crate::content::Loadout { items: vec!["frag".into(), "frag".into()], ..Default::default() };
        let id = b.spawn_equipped("assault", "A", Team::Player, GridPos::new(0, 0), Some("breacher"), &over).unwrap();
        let u = b.unit(id).unwrap();
        assert_eq!(u.weapon.id, "shotgun");
        assert_eq!(u.stats.max_hp, 14, "plated armor +4");
        assert_eq!(u.stats.mobility, 12, "plated armor -2");
        assert!(u.has_ability(&"frag_grenade".into()));
        let def = b.ability(&"frag_grenade".into()).unwrap();
        assert_eq!(b.charges_left(u, def), Some(2), "two grenades carried");
        // Illegal gear is rejected.
        let bad = crate::content::Loadout { weapon: Some("sniper_rifle".into()), ..Default::default() };
        assert!(b.spawn_equipped("assault", "B", Team::Player, GridPos::new(1, 0), None, &bad).is_err());
        let bad = crate::content::Loadout { items: vec!["kevlar".into()], ..Default::default() };
        assert!(b.spawn_equipped("assault", "B", Team::Player, GridPos::new(1, 0), None, &bad).is_err());
    }
}

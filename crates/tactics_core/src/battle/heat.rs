//! Heat: machines (robot frames) that run on heat instead of health. See
//! [`HeatRules`](crate::unit::HeatRules): `max_hp` is the heat limit and HP
//! the headroom left, so damage simply adds heat. This file adds the rest:
//! heat from the unit's own shots and moves, cooling each turn, overheating,
//! and rebooting after a Shutdown.

use super::Battle;
use crate::action::Action;
use crate::content::{AbilityDef, AbilityEffect, DamageSource};
use crate::event::GameEvent;
use crate::unit::{Team, Unit, UnitId};

impl Battle {
    /// Heat one attack with the unit's weapon adds (0 for units that don't
    /// run on heat).
    pub fn weapon_heat(&self, unit: &Unit) -> i32 {
        let Some(h) = unit.heat else { return 0 };
        if unit.weapon.heat > 0 { unit.weapon.heat } else { h.per_shot }
    }

    /// Heat using `def` adds to `unit`: the ability's own heat, plus the
    /// weapon's for attacks that fire it.
    pub fn ability_heat(&self, unit: &Unit, def: &AbilityDef) -> i32 {
        if unit.heat.is_none() {
            return 0;
        }
        let fires_weapon = match &def.effect {
            AbilityEffect::Attack(p) | AbilityEffect::Suppress(p) => p.damage == DamageSource::Weapon && !p.melee,
            _ => false,
        };
        let heat = def.heat.max(0) + if fires_weapon { self.weapon_heat(unit) } else { 0 };
        if unit.status.overclocked { heat * 2 } else { heat }
    }

    /// Heat moving with `ap` action points adds to `unit`.
    pub fn move_heat(&self, unit: &Unit, ap: u8) -> i32 {
        let heat = unit.heat.map_or(0, |h| h.per_move.max(0) * ap as i32);
        if unit.status.overclocked { heat * 2 } else { heat }
    }

    /// Heat `action` would add to its unit (0 when it doesn't run on heat).
    /// Overwatch costs heat only when it fires.
    pub fn heat_cost(&self, action: &Action) -> i32 {
        match action {
            Action::Move { unit, to } => {
                let Some(u) = self.unit(*unit) else { return 0 };
                let cost = self.movement_range(*unit).cost(*to).unwrap_or(0);
                self.move_heat(u, self.move_cost_ap(*unit, cost))
            }
            Action::UseAbility { unit, ability, .. } => match (self.unit(*unit), self.ability(ability)) {
                (Some(u), Some(def)) => self.ability_heat(u, def),
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Would `action` overheat (destroy) its own unit?
    pub fn overheats(&self, action: &Action) -> bool {
        let Some(u) = action.actor().and_then(|id| self.unit(id)) else { return false };
        u.heat.is_some() && self.heat_cost(action) >= u.hp
    }

    /// Adds `amount` heat (negative: sheds it, never below 0). Reaching the
    /// limit destroys the unit.
    pub(super) fn add_heat(&mut self, id: UnitId, amount: i32, events: &mut Vec<GameEvent>) {
        let Some(u) = self.units.get(id.0 as usize).filter(|u| u.is_alive() && u.heat.is_some()) else { return };
        let max = u.stats.max_hp;
        if amount >= u.hp {
            // Overheat: everything inside it cooks off.
            events.push(GameEvent::HeatChanged { unit: id, delta: u.hp, heat: max, max });
            events.push(GameEvent::Callout { unit: id, text: "OVERHEATED".into() });
            let hp = u.hp;
            self.apply_damage(id, hp, false, events);
            return;
        }
        let u = &mut self.units[id.0 as usize];
        let before = u.hp;
        u.hp = (u.hp - amount).min(max);
        let delta = before - u.hp;
        if delta != 0 {
            events.push(GameEvent::HeatChanged { unit: id, delta, heat: max - u.hp, max });
        }
    }

    /// Start of `team`'s turn: its machines cool down; shut-down ones tick
    /// their countdown (offline, no AP) and, when it ends, reboot with all
    /// heat vented.
    pub(super) fn heat_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let machines: Vec<(UnitId, i32)> = self
            .units
            .iter()
            .filter(|u| u.team == team && u.is_alive())
            .filter_map(|u| u.heat.map(|h| (u.id, h.cooling)))
            .collect();
        for (id, cooling) in machines {
            let u = &mut self.units[id.0 as usize];
            if u.status.shutdown > 0 {
                u.status.shutdown -= 1;
                if u.status.shutdown > 0 {
                    u.action_points = 0;
                    events.push(GameEvent::ActionPointsChanged { unit: id, remaining: 0 });
                    continue;
                }
                let vent = u.heat();
                if vent > 0 {
                    self.add_heat(id, -vent, events);
                }
                events.push(GameEvent::Shutdown { unit: id, shut_down: false });
            } else if cooling > 0 && u.heat() > 0 {
                self.add_heat(id, -cooling, events);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::battle::Battle;
    use crate::content::{AbilityDef, AbilityEffect, AbilityRange, ApCost, Ruleset, Targeting};
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::unit::{HeatRules, Team};

    /// The standard rules plus a heat-running "frame" class (limit 10,
    /// cooling 3, 1 per move AP, 4 per shot) that knows Shutdown and Cool.
    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        let mut frame = r.class("rifleman").unwrap().clone();
        frame.id = "frame".into();
        frame.stats.max_hp = 10;
        frame.fearless = true;
        frame.heat = Some(HeatRules { cooling: 3, per_move: 1, per_shot: 4 });
        frame.abilities.push("shutdown".into());
        frame.abilities.push("coolant".into());
        r.add_class(frame);
        r.add_ability(
            AbilityDef::new("shutdown", "Shutdown")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
                .with_effect(AbilityEffect::Shutdown { turns: 2 }),
        );
        r.add_ability(
            AbilityDef::new("coolant", "Coolant")
                .with_cost(ApCost::Points(1))
                .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
                .with_effect(AbilityEffect::Cool { amount: 5 }),
        );
        r
    }

    fn battle() -> (Battle, crate::unit::UnitId, crate::unit::UnitId) {
        let mut b = Battle::new(Map::from_ascii("..........\n..........\n.........."), rules(), 7);
        let f = b.spawn("frame", "F", Team::Player, GridPos::new(0, 0)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(6, 2)).unwrap();
        // A second enemy far away keeps the battle going if E falls.
        b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(9, 2)).unwrap();
        (b, f, e)
    }

    fn heat(b: &Battle, id: crate::unit::UnitId) -> i32 {
        b.unit(id).unwrap().heat()
    }

    #[test]
    fn shooting_moving_and_hits_add_heat_and_it_cools_off() {
        let (mut b, f, e) = battle();
        assert_eq!(heat(&b, f), 0);
        assert_eq!(b.heat_cost(&Action::ability(f, "shoot", Target::Unit(e))), 4);
        b.apply(Action::Move { unit: f, to: GridPos::new(2, 0) }).unwrap();
        assert_eq!(heat(&b, f), 1, "1 AP of moving");
        let ev = b.apply(Action::ability(f, "shoot", Target::Unit(e))).unwrap();
        assert_eq!(heat(&b, f), 5, "+4 for the shot");
        assert!(ev.iter().any(|x| matches!(x, GameEvent::HeatChanged { delta: 4, heat: 5, max: 10, .. })));
        // Shooting ended the squad's turn; no cooling on the enemy's turn,
        // then it sheds 3 at the start of its next one.
        assert_eq!(b.active_team(), Team::Enemy);
        assert_eq!(heat(&b, f), 5);
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(heat(&b, f), 2);
        // Hits add heat (damage = heat).
        let mut ev = Vec::new();
        b.apply_damage(f, 3, true, &mut ev);
        assert_eq!(heat(&b, f), 5);
        // Coolant flush sheds 5.
        b.apply(Action::ability(f, "coolant", Target::None)).unwrap();
        assert_eq!(heat(&b, f), 0);
        assert!(b.apply(Action::ability(f, "coolant", Target::None)).is_err(), "nothing to shed");
    }

    #[test]
    fn overheating_destroys_the_machine() {
        let (mut b, f, e) = battle();
        let mut ev = Vec::new();
        b.apply_damage(f, 7, true, &mut ev); // heat 7 of 10
        let shot = Action::ability(f, "shoot", Target::Unit(e));
        assert!(b.overheats(&shot), "a 4-heat shot at 7/10 cooks it");
        let ev = b.apply(shot).unwrap();
        assert!(!b.unit(f).unwrap().is_alive());
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Callout { text, .. } if text == "OVERHEATED")));
    }

    #[test]
    fn shutdown_vents_everything_but_any_hit_kills() {
        let (mut b, f, _) = battle();
        let mut ev = Vec::new();
        b.apply_damage(f, 8, true, &mut ev); // heat 8 of 10
        b.apply(Action::ability(f, "shutdown", Target::None)).unwrap();
        assert!(b.unit(f).unwrap().is_shut_down());
        assert_eq!(b.active_team(), Team::Enemy, "shutting down ends the turn");
        // Two of its own turns offline...
        for _ in 0..2 {
            b.apply(Action::EndTurn).unwrap(); // the squad's turn: still offline
            assert_eq!(b.active_team(), Team::Player);
            assert_eq!(b.unit(f).unwrap().action_points, 0, "offline");
            assert!(b.unit(f).unwrap().is_shut_down());
            b.apply(Action::EndTurn).unwrap(); // enemy
        }
        // ...then back online with no heat.
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(b.active_team(), Team::Player);
        let u = b.unit(f).unwrap();
        assert!(!u.is_shut_down() && u.action_points > 0);
        assert_eq!(heat(&b, f), 0, "vented");

        // Shut down again, and get hit for 1: destroyed.
        let (mut b, f, _) = battle();
        b.apply(Action::ability(f, "shutdown", Target::None)).unwrap();
        let mut ev = Vec::new();
        b.apply_damage(f, 1, true, &mut ev);
        assert!(!b.unit(f).unwrap().is_alive(), "any hit while shut down is fatal");
    }


    /// The AI never fires a shot that would overheat its machine: a hot
    /// frame vents (coolant) instead; with no coolant and nobody watching,
    /// it shuts down.
    #[test]
    fn ai_vents_instead_of_overheating() {
        use crate::ai::{self, AiConfig};
        let (mut b, f, _) = battle();
        let mut ev = Vec::new();
        b.apply_damage(f, 8, true, &mut ev); // 8/10: a 4-heat shot would kill it
        let a = ai::choose_action(&b, f, &AiConfig::default());
        assert!(!b.overheats(&a), "{a:?}");
        assert_eq!(a, Action::ability(f, "coolant", Target::None));

        // No coolant, out of sight behind a wall: shut down.
        let mut r = rules();
        let mut frame = r.class("frame").unwrap().clone();
        frame.abilities.retain(|a| a.as_str() != "coolant");
        r.add_class(frame);
        let mut b = Battle::new(Map::from_ascii("....H.....\n....H.....\n....H....."), r, 7);
        let f = b.spawn("frame", "F", Team::Player, GridPos::new(0, 1)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 1)).unwrap();
        let mut ev = Vec::new();
        b.apply_damage(f, 8, true, &mut ev);
        let a = ai::choose_action(&b, f, &AiConfig::default());
        assert_eq!(a, Action::ability(f, "shutdown", Target::None));
    }

    fn movement_rules() -> Ruleset {
        let mut r = rules();
        let mut frame = r.class("frame").unwrap().clone();
        frame.stats.max_hp = 30;
        frame.abilities.extend(["overclock".into(), "boost".into()]);
        r.add_class(frame);
        let mut soldier = r.class("rifleman").unwrap().clone();
        soldier.abilities.push("jump".into());
        r.add_class(soldier);
        r.add_ability(
            AbilityDef::new("overclock", "Overclock")
                .with_cost(ApCost::Points(0))
                .with_charges(3)
                .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
                .with_effect(AbilityEffect::Overclock),
        );
        let leap = |id: &str, reach: f32, over_walls: bool| {
            let mut d = AbilityDef::new(id, id)
                .with_cost(ApCost::Points(1))
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(0.0))
                .with_effect(AbilityEffect::Leap { reach, over_walls });
            d.requires_los = false;
            d
        };
        r.add_ability(leap("boost", 3.0, false));
        r.add_ability(leap("jump", 2.0, true));
        r
    }

    /// Overclock: one extra shot this turn, every shot at double heat.
    #[test]
    fn overclock_gives_an_extra_shot_at_double_heat() {
        let mut b = Battle::new(Map::from_ascii("..........\n..........\n.........."), movement_rules(), 7);
        let f = b.spawn("frame", "F", Team::Player, GridPos::new(0, 0)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(6, 2)).unwrap();
        b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(9, 2)).unwrap();
        b.unit_mut(e).hp = 99; // keep the target alive
        b.apply(Action::ability(f, "overclock", Target::None)).unwrap();
        assert_eq!(b.unit(f).unwrap().action_points, 2, "overclocking is free");
        assert_eq!(b.heat_cost(&Action::ability(f, "shoot", Target::Unit(e))), 8, "double heat");
        b.apply(Action::ability(f, "shoot", Target::Unit(e))).unwrap();
        assert_eq!(b.unit(f).unwrap().action_points, 1, "the first shot costs 1 AP");
        assert_eq!(heat(&b, f), 8);
        b.apply(Action::ability(f, "shoot", Target::Unit(e))).unwrap();
        assert_eq!(heat(&b, f), 16);
        assert_eq!(b.active_team(), Team::Enemy, "the second shot ends the turn");
        // Over at the end of the turn; 2 uses left.
        assert!(!b.unit(f).unwrap().status.overclocked);
        assert_eq!(b.charges_left(b.unit(f).unwrap(), b.ability(&"overclock".into()).unwrap()), Some(2));
    }

    /// Boost thrusters stop at walls; the jump pack goes over them.
    #[test]
    fn boost_is_stopped_by_walls_jump_is_not() {
        let art = "
            ..........
            ...H......
            ..........
        ";
        let mut b = Battle::new(Map::from_ascii(art), movement_rules(), 7);
        let f = b.spawn("frame", "F", Team::Player, GridPos::new(1, 0)).unwrap();
        let s = b.spawn("rifleman", "S", Team::Player, GridPos::new(1, 1)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 2)).unwrap();
        // Through the wall at (3,1): blocked for the boost.
        assert!(b.validate(&Action::ability(f, "boost", Target::Tile(GridPos::new(6, 2)))).is_err());
        // Open ground: fine, and far (3 x 6 tiles per AP).
        assert!(b.validate(&Action::ability(f, "boost", Target::Tile(GridPos::new(9, 0)))).is_ok());
        // The jump pack goes over the wall (2 x 6 tiles).
        let ev = b.apply(Action::ability(s, "jump", Target::Tile(GridPos::new(6, 1)))).unwrap();
        assert_eq!(b.unit(s).unwrap().pos, GridPos::new(6, 1));
        assert!(ev.iter().any(|x| matches!(x, GameEvent::UnitLeaped { over_walls: true, .. })));
        // Never onto a wall or past its reach.
        assert!(b.validate(&Action::ability(f, "boost", Target::Tile(GridPos::new(3, 1)))).is_err());
    }
}

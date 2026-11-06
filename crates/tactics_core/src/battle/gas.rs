//! Gas on the battlefield (gas grenades): tiles full of choking gas for a
//! few rounds. Living units in it can't aim (see `combat::preview_attack`)
//! and stagger to a random tile when they try to move. Machines are immune.

use std::collections::BTreeMap;

use super::Battle;
use crate::event::GameEvent;
use crate::grid::{GridPos, Terrain};
use crate::pathfinding::ReachableSet;
use crate::unit::{Machine, Team, UnitId};

impl Battle {
    /// Gassed tiles and the rounds of gas left on each.
    pub fn gas(&self) -> &BTreeMap<GridPos, u32> {
        &self.gas
    }

    /// Is there gas on `pos`?
    pub fn gassed(&self, pos: GridPos) -> bool {
        self.gas.contains_key(&pos)
    }

    /// Fills `tiles` with gas for `turns` rounds (not walls or holes).
    pub(super) fn release_gas(&mut self, tiles: &[GridPos], turns: u32, events: &mut Vec<GameEvent>) {
        let filled: Vec<GridPos> = tiles
            .iter()
            .copied()
            .filter(|p| self.map.in_bounds(*p) && !matches!(self.map.terrain(*p), Terrain::HighCover | Terrain::Void))
            .collect();
        for p in &filled {
            let t = self.gas.entry(*p).or_insert(0);
            *t = (*t).max(turns);
        }
        if !filled.is_empty() {
            events.push(GameEvent::GasReleased { tiles: filled, turns });
        }
        self.refresh_gassed();
    }

    /// At the start of each round (the player's turn) the gas thins by one.
    pub(super) fn gas_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        if self.gas.is_empty() || team != Team::Player {
            return;
        }
        let mut out = Vec::new();
        self.gas.retain(|p, t| {
            *t = t.saturating_sub(1);
            if *t == 0 {
                out.push(*p);
            }
            *t > 0
        });
        if !out.is_empty() {
            events.push(GameEvent::GasCleared { tiles: out });
        }
    }

    /// Marks who's choking: living (non-machine) units with any tile in gas.
    pub(super) fn refresh_gassed(&mut self) {
        let gas = &self.gas;
        for u in self.units.iter_mut() {
            u.status.gassed = !gas.is_empty() && u.is_alive() && u.machine == Machine::None && u.tiles().any(|p| gas.contains_key(&p));
        }
    }

    /// Where a move to `to` really goes: a gassed unit staggers to a random
    /// tile it could reach for no more than the planned move's cost.
    pub(super) fn stagger(&mut self, id: UnitId, to: GridPos, range: &ReachableSet, events: &mut Vec<GameEvent>) -> GridPos {
        if !self.units[id.0 as usize].status.gassed {
            return to;
        }
        let Some(budget) = range.cost(to) else { return to };
        let from = self.units[id.0 as usize].pos;
        let mut options: Vec<GridPos> = range.iter().filter(|&(p, c)| p != from && c <= budget).map(|(p, _)| p).collect();
        options.sort_by_key(|p| (p.y, p.x)); // deterministic, whatever the set's order
        if options.is_empty() {
            return to;
        }
        let pick = options[self.rng.range_inclusive(0, options.len() as i32 - 1) as usize];
        events.push(GameEvent::Callout { unit: id, text: "STAGGERING".into() });
        pick
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::battle::Battle;
    use crate::content::{
        AbilityDef, AbilityEffect, AbilityId, AbilityRange, ApCost, AttackProfile, DamageSource, ItemDef, ItemSlot, Loadout,
        Ruleset, StatModifiers, Targeting,
    };
    use crate::grid::{GridPos, Map};
    use crate::unit::{Machine, Team};

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_ability(
            AbilityDef::new("gas_grenade", "Gas Grenade")
                .with_cost(ApCost::Points(1))
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(10.0))
                .with_effect(AbilityEffect::Attack(AttackProfile {
                    can_miss: false,
                    range_falloff: false,
                    damage: DamageSource::Fixed { min: 0, max: 0 },
                    area_radius: 2.0,
                    gas: 3,
                    ..Default::default()
                })),
        );
        let mut c = r.class("rifleman").unwrap().clone();
        c.abilities.push(AbilityId::new("gas_grenade"));
        r.add_class(c.clone());
        c.id = "bot".into();
        c.machine = Machine::Light;
        r.add_class(c);
        r.add_item(ItemDef {
            id: "scanner".into(),
            slot: ItemSlot::Armor,
            stats: StatModifiers { sight_percent: 50, ..Default::default() },
            ..Default::default()
        });
        r
    }

    #[test]
    fn gas_stops_aim_and_spares_robots_then_clears() {
        let mut b = Battle::new(Map::from_ascii(&"............\n".repeat(7)), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 3)).unwrap();
        let e = b.spawn("rifleman", "E", Team::Enemy, GridPos::new(8, 3)).unwrap();
        let bot = b.spawn("bot", "Bot", Team::Enemy, GridPos::new(9, 3)).unwrap();
        let hp = b.unit(e).unwrap().hp;
        b.apply(Action::ability(a, "gas_grenade", Target::Tile(GridPos::new(8, 3)))).unwrap();
        assert!(b.gassed(GridPos::new(8, 3)) && b.gassed(GridPos::new(10, 3)));
        assert_eq!(b.unit(e).unwrap().hp, hp, "gas does no damage");
        assert!(b.unit(e).unwrap().status.gassed);
        assert!(!b.unit(bot).unwrap().status.gassed, "robots don't breathe");
        // In the gas the enemy can't aim at all.
        b.apply(Action::EndTurn).unwrap();
        let shot = b.preview(e, &AbilityId::new("shoot"), Target::Unit(a)).unwrap();
        assert_eq!(shot[0].1.hit_chance, 0);
        let bot_shot = b.preview(bot, &AbilityId::new("shoot"), Target::Unit(a)).unwrap();
        assert!(bot_shot[0].1.hit_chance > 0);
        // It thins out after 3 rounds.
        for _ in 0..6 {
            b.apply(Action::EndTurn).unwrap();
        }
        assert!(b.gas().is_empty(), "{:?}", b.gas());
        assert!(!b.unit(e).unwrap().status.gassed);
    }

    #[test]
    fn moving_in_gas_staggers_somewhere_random_within_reach() {
        let mut moved_astray = 0;
        for seed in 0..12 {
            let mut b = Battle::new(Map::from_ascii(&"............\n".repeat(7)), rules(), seed);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(5, 3)).unwrap();
            b.spawn("rifleman", "E", Team::Enemy, GridPos::new(11, 6)).unwrap();
            b.apply(Action::ability(a, "gas_grenade", Target::Tile(GridPos::new(5, 3)))).unwrap();
            assert!(b.unit(a).unwrap().status.gassed);
            let goal = GridPos::new(5, 5);
            let budget = b.movement_range(a).cost(goal).unwrap();
            let range = b.movement_range(a);
            b.apply(Action::Move { unit: a, to: goal }).unwrap();
            let at = b.unit(a).unwrap().pos;
            assert!(range.cost(at).is_some_and(|c| c <= budget), "no further than planned: {at:?}");
            if at != goal {
                moved_astray += 1;
            }
        }
        assert!(moved_astray >= 6, "mostly not where it meant to go: {moved_astray}/12");
    }

    #[test]
    fn scanner_armor_sees_half_as_far_again() {
        let mut b = Battle::new(Map::from_ascii(&"....\n".repeat(3)), rules(), 1);
        let plain = b.spawn("rifleman", "Plain", Team::Player, GridPos::new(0, 0)).unwrap();
        let gear = Loadout { armor: Some("scanner".into()), ..Default::default() };
        let scan = b.spawn_equipped("rifleman", "Scanner", Team::Player, GridPos::new(1, 0), None, &gear).unwrap();
        let (s0, s1) = (b.unit(plain).unwrap().stats.sight, b.unit(scan).unwrap().stats.sight);
        assert!((s1 - s0 * 1.5).abs() < 0.01, "{s0} -> {s1}");
    }
}

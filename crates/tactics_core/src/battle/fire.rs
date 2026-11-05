//! Fire, cones and rockets.
//!
//! * **Fire**: attacks with `ignite` (flamethrowers, incendiaries) and the
//!   script function `ignite` set tiles burning for a number of rounds. A
//!   unit that starts its turn in fire takes `CombatRules::fire_damage`; each
//!   burning tile walked into costs `fire_step_damage` (machines that run on
//!   heat just get hotter). Props on burning tiles take 1 damage a round
//!   (fuel barrels cook off). Walls and holes don't burn.
//! * **Cones** (`AttackProfile::cone`): everyone the user can see within the
//!   cone's angle, out to the target tile.
//! * **Rockets** (`AttackProfile::direct`): the blast happens at the first
//!   wall or unit on the straight line to the target.

use std::collections::BTreeMap;

use super::Battle;
use crate::action::Target;
use crate::content::{AbilityEffect, AbilityId, AttackProfile};
use crate::event::GameEvent;
use crate::grid::{GridPos, Terrain};
use crate::unit::{Team, UnitId};
use crate::visibility;

/// A delayed charge waiting to go off (see `AttackProfile::fuse`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmedCharge {
    /// Who threw it (it still goes off if they fall).
    pub unit: UnitId,
    pub team: Team,
    pub ability: AbilityId,
    pub pos: GridPos,
    /// The thrower's turns left before it goes off.
    pub turns: u32,
}

impl Battle {
    /// Charges waiting to go off.
    pub fn armed_charges(&self) -> &[ArmedCharge] {
        &self.charges_armed
    }

    /// Tiles an armed charge will hit when it goes off.
    pub fn charge_tiles(&self, charge: &ArmedCharge) -> Vec<GridPos> {
        match self.rules.ability(&charge.ability).and_then(|d| d.effect.attack_profile()) {
            Some(p) => self.area_tiles(charge.pos, p, charge.pos),
            None => Vec::new(),
        }
    }

    /// Start of `team`'s turn: its armed charges tick down; those at zero
    /// go off (the same blast as the ability, now).
    pub(super) fn charges_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        let mut due = Vec::new();
        self.charges_armed.retain_mut(|c| {
            if c.team != team {
                return true;
            }
            c.turns = c.turns.saturating_sub(1);
            if c.turns == 0 {
                due.push(c.clone());
                false
            } else {
                true
            }
        });
        for c in due {
            if self.is_over() {
                return;
            }
            let Some(mut def) = self.rules.ability(&c.ability).cloned() else { continue };
            if let AbilityEffect::Attack(p) = &mut def.effect {
                p.fuse = 0;
                // It's already where it landed.
                p.direct = false;
            }
            self.run_effect(c.unit, &def, c.pos, Target::Tile(c.pos), events);
        }
    }

    /// Burning tiles and the rounds of fire they have left.
    pub fn fires(&self) -> &BTreeMap<GridPos, u32> {
        &self.fires
    }

    pub fn on_fire(&self, pos: GridPos) -> bool {
        self.fires.contains_key(&pos)
    }

    /// Where an area attack from `from` aimed at `point` goes off: `point`,
    /// or for a rocket (`direct`) the first wall or unit on the way.
    pub fn blast_point(&self, from: GridPos, profile: &AttackProfile, point: GridPos) -> GridPos {
        if !profile.direct || from == point {
            return point;
        }
        let (dx, dy) = ((point.x - from.x) as f32, (point.y - from.y) as f32);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = (len * 5.0).ceil() as i32;
        let mut last = from;
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            let p = GridPos::new((from.x as f32 + dx * k).round() as i32, (from.y as f32 + dy * k).round() as i32);
            if p == last || p == from {
                continue;
            }
            last = p;
            if p == point {
                return point;
            }
            if !self.map.in_bounds(p) || self.map.terrain(p) == Terrain::HighCover || self.unit_at(p).is_some() {
                return p;
            }
        }
        point
    }

    /// Tiles an area attack covers: a blast around `point`, or a cone from
    /// `from` towards `point` (only tiles `from` can see; not its own).
    pub fn area_tiles(&self, from: GridPos, profile: &AttackProfile, point: GridPos) -> Vec<GridPos> {
        let (reach, cone) = if profile.cone > 0.0 { (from.distance(point).max(1.0), true) } else { (profile.area_radius, false) };
        // A square blast: everything up to `area_radius` tiles out on each side.
        if !cone && profile.square {
            let r = profile.area_radius.floor() as i32;
            return (point.y - r..=point.y + r)
                .flat_map(|y| (point.x - r..=point.x + r).map(move |x| GridPos::new(x, y)))
                .filter(|p| self.map.in_bounds(*p))
                .collect();
        }
        let center = if cone { from } else { point };
        let r = reach.ceil() as i32;
        let dir = ((point.x - from.x) as f32, (point.y - from.y) as f32);
        let dir_len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(1e-4);
        let half = (profile.cone / 2.0).to_radians().cos();
        let mut out = Vec::new();
        for y in center.y - r..=center.y + r {
            for x in center.x - r..=center.x + r {
                let p = GridPos::new(x, y);
                if !self.map.in_bounds(p) || center.distance(p) > reach + 0.01 {
                    continue;
                }
                if cone {
                    if p == from {
                        continue;
                    }
                    let v = ((p.x - from.x) as f32, (p.y - from.y) as f32);
                    let cos = (v.0 * dir.0 + v.1 * dir.1) / ((v.0 * v.0 + v.1 * v.1).sqrt() * dir_len);
                    if cos < half - 1e-4 || !visibility::has_line_of_sight(&self.map, from, p) {
                        continue;
                    }
                }
                out.push(p);
            }
        }
        out
    }

    /// Sets `tiles` burning for `turns` rounds (walls and holes don't burn;
    /// a tile already burning keeps the longer fire).
    pub(super) fn ignite(&mut self, tiles: &[GridPos], turns: u32, events: &mut Vec<GameEvent>) {
        if turns == 0 {
            return;
        }
        let lit: Vec<GridPos> = tiles
            .iter()
            .copied()
            .filter(|p| self.map.in_bounds(*p) && !matches!(self.map.terrain(*p), Terrain::HighCover | Terrain::Void))
            .collect();
        for p in &lit {
            let t = self.fires.entry(*p).or_insert(0);
            *t = (*t).max(turns);
        }
        if !lit.is_empty() {
            events.push(GameEvent::FireStarted { tiles: lit, turns });
        }
    }

    /// Fire in a circle (scripts, barrels).
    pub(super) fn ignite_area(&mut self, center: GridPos, radius: f32, turns: u32, events: &mut Vec<GameEvent>) {
        let profile = AttackProfile { area_radius: radius.max(0.5), ..Default::default() };
        let tiles = self.area_tiles(center, &profile, center);
        self.ignite(&tiles, turns, events);
    }

    /// Start of `team`'s turn: its units standing in fire burn. At the
    /// start of a round (the player's turn) every fire burns down by one
    /// and scorches the props on it.
    pub(super) fn fire_turn_start(&mut self, team: Team, events: &mut Vec<GameEvent>) {
        if self.fires.is_empty() {
            return;
        }
        if team == Team::Player {
            let mut out = Vec::new();
            self.fires.retain(|p, t| {
                *t = t.saturating_sub(1);
                if *t == 0 {
                    out.push(*p);
                }
                *t > 0
            });
            if !out.is_empty() {
                events.push(GameEvent::FireOut { tiles: out });
            }
            let burning: Vec<_> = self
                .props
                .iter()
                .filter(|p| p.alive && p.hp.is_some() && self.fires.contains_key(&p.pos))
                .map(|p| p.id)
                .collect();
            for id in burning {
                self.damage_prop(id, 1, events);
            }
        }
        let damage = self.rules.combat.fire_damage;
        let burning: Vec<UnitId> = self.living(team).filter(|u| self.fires.contains_key(&u.pos)).map(|u| u.id).collect();
        for id in burning {
            if self.is_over() {
                return;
            }
            events.push(GameEvent::Callout { unit: id, text: "BURNING".into() });
            self.apply_damage(id, damage, true, events);
        }
    }

    /// A unit walked into a burning tile. Returns whether it's still alive.
    pub(super) fn burn_step(&mut self, id: UnitId, events: &mut Vec<GameEvent>) -> bool {
        let damage = self.rules.combat.fire_step_damage;
        if damage > 0 {
            events.push(GameEvent::Callout { unit: id, text: "BURNING".into() });
            self.apply_damage(id, damage, true, events);
        }
        self.units[id.0 as usize].is_alive()
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::battle::Battle;
    use crate::content::{AbilityDef, AbilityEffect, AbilityRange, ApCost, AttackProfile, DamageSource, Ruleset, Targeting};
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::unit::Team;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        let area = |cone: f32, radius: f32, ignite: u32, direct: bool| AttackProfile {
            can_miss: false,
            half_cover_penalty: 0,
            full_cover_penalty: 0,
            range_falloff: false,
            flank_crit_bonus: 0,
            damage: DamageSource::Fixed { min: 1, max: 1 },
            area_radius: radius,
            cone,
            ignite,
            direct,
            ..Default::default()
        };
        r.add_ability(
            AbilityDef::new("flamer", "Flamer")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(5.0))
                .with_effect(AbilityEffect::Attack(area(60.0, 0.0, 2, false))),
        );
        r.add_ability(
            AbilityDef::new("rocket", "Rocket")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(12.0))
                .with_effect(AbilityEffect::Attack(area(0.0, 1.0, 0, true))),
        );
        let mut c = r.class("rifleman").unwrap().clone();
        c.abilities.extend(["flamer".into(), "rocket".into()]);
        r.add_class(c);
        r
    }

    #[test]
    fn flamethrower_burns_a_cone_and_fire_hurts_then_dies_out() {
        let art = "
            ..........
            ..........
            A....E...E
            ..........
            ..........
        ";
        let mut b = Battle::new(Map::from_ascii(art), rules(), 3);
        let a = b.spawn("rifleman", "A", Team::Player, Map::find_markers(art, 'A')[0]).unwrap();
        let e: Vec<_> = Map::find_markers(art, 'E').into_iter().map(|p| b.spawn("sectoid", "E", Team::Enemy, p).unwrap()).collect();
        let near = b.unit(e[0]).unwrap().pos;
        let hp = b.unit(e[0]).unwrap().hp;
        let ev = b.apply(Action::ability(a, "flamer", Target::Tile(near))).unwrap();
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Flames { .. })));
        // 1 from the flames, then (the flamer ends the turn) 2 more for
        // starting the enemy's turn in the fire.
        assert_eq!(b.active_team(), Team::Enemy);
        assert_eq!(b.unit(e[0]).unwrap().hp, hp - 3, "caught in the cone, then burning");
        assert_eq!(b.unit(e[1]).unwrap().hp, hp, "beyond its reach");
        assert!(b.on_fire(near) && b.on_fire(GridPos::new(near.x - 2, near.y)));
        assert!(!b.on_fire(b.unit(a).unwrap().pos), "not under the user");
        // The fire burns out after its 2 rounds.
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
        assert!(b.fires().is_empty(), "{:?}", b.fires());
    }

    #[test]
    fn starting_a_turn_in_fire_or_walking_through_it_burns() {
        let mut b = Battle::new(Map::from_ascii("..........\n.........."), rules(), 3);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 1)).unwrap();
        let mut ev = Vec::new();
        b.ignite(&[GridPos::new(1, 0), GridPos::new(2, 0)], 3, &mut ev);
        let hp = b.unit(a).unwrap().hp;
        b.apply(Action::Move { unit: a, to: GridPos::new(3, 0) }).unwrap();
        assert_eq!(b.unit(a).unwrap().hp, hp - 2, "two burning tiles walked through");
        // Standing in fire at the start of its turn.
        let mut ev = Vec::new();
        b.ignite(&[GridPos::new(3, 0)], 3, &mut ev);
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(b.unit(a).unwrap().hp, hp - 4);
    }

    #[test]
    fn rockets_blow_up_on_the_first_thing_in_the_way() {
        let art = "
            A..B....E.
            ..........
        ";
        let mut b = Battle::new(Map::from_ascii(art), rules(), 3);
        let a = b.spawn("rifleman", "A", Team::Player, Map::find_markers(art, 'A')[0]).unwrap();
        let blocker = b.spawn("sectoid", "B", Team::Enemy, Map::find_markers(art, 'B')[0]).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, Map::find_markers(art, 'E')[0]).unwrap();
        let target = b.unit(e).unwrap().pos;
        let profile = b.ability(&"rocket".into()).unwrap().effect.attack_profile().unwrap().clone();
        assert_eq!(b.blast_point(b.unit(a).unwrap().pos, &profile, target), b.unit(blocker).unwrap().pos);
        let hp = b.unit(e).unwrap().hp;
        let ev = b.apply(Action::ability(a, "rocket", Target::Tile(target))).unwrap();
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Explosion { center, .. } if *center == Map::find_markers(art, 'B')[0])));
        assert_eq!(b.unit(e).unwrap().hp, hp, "the far target is untouched");
    }

    /// A particle grenade: lands now, goes off at the start of the
    /// thrower's second turn after, hitting a 5x5 square (not a circle).
    #[test]
    fn delayed_square_charges_go_off_later() {
        let mut r = rules();
        r.add_ability(
            AbilityDef::new("particle", "Particle")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(10.0))
                .with_effect(AbilityEffect::Attack(AttackProfile {
                    can_miss: false,
                    half_cover_penalty: 0,
                    full_cover_penalty: 0,
                    range_falloff: false,
                    damage: DamageSource::Fixed { min: 5, max: 5 },
                    area_radius: 2.0,
                    square: true,
                    fuse: 2,
                    ..Default::default()
                })),
        );
        let mut c = r.class("rifleman").unwrap().clone();
        c.abilities.push("particle".into());
        r.add_class(c);
        let mut b = Battle::new(Map::from_ascii(&"............\n".repeat(9)), r, 3);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        let corner = b.spawn("sectoid", "Corner", Team::Enemy, GridPos::new(8, 6)).unwrap(); // (6,4) + (2,2): in the square
        let outside = b.spawn("sectoid", "Out", Team::Enemy, GridPos::new(9, 4)).unwrap(); // 3 out: safe
        let hp = b.unit(corner).unwrap().hp;
        let ev = b.apply(Action::ability(a, "particle", Target::Tile(GridPos::new(6, 4)))).unwrap();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::ChargeArmed { turns: 2, .. })));
        assert_eq!(b.unit(corner).unwrap().hp, hp, "nothing yet");
        assert_eq!(b.armed_charges().len(), 1);
        assert_eq!(b.charge_tiles(&b.armed_charges()[0]).len(), 25, "5x5");
        // Throwing ended the squad's turn. Its next turn (1 left), the
        // enemy turn...
        assert_eq!(b.active_team(), Team::Enemy);
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(b.unit(corner).unwrap().hp, hp, "still ticking");
        b.apply(Action::EndTurn).unwrap();
        assert_eq!(b.unit(corner).unwrap().hp, hp, "still ticking");
        // ...and it goes off at the start of the thrower's second turn.
        let ev = b.apply(Action::EndTurn).unwrap();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::Explosion { .. })));
        assert_eq!(b.unit(corner).unwrap().hp, hp - 5, "5 damage in the corner of the square");
        assert_eq!(b.unit(outside).unwrap().hp, hp, "3 tiles out: untouched");
        assert!(b.armed_charges().is_empty());
    }

    /// A flamethrower as the main weapon: Shoot sweeps a cone out to the
    /// target, burning everyone in it (friend or foe), never missing, and
    /// leaves fire; it can't suppress.
    #[test]
    fn flamethrower_main_weapon_shoots_cones() {
        use crate::unit::Weapon;
        let mut r = rules();
        r.add_weapon(Weapon {
            id: "flamer".into(),
            name: "Flamer".into(),
            damage_min: 2,
            damage_max: 2,
            optimal_range: 3.0,
            max_range: 5.0,
            clip_size: 4,
            cone: 45.0,
            ignite: 2,
            ..Default::default()
        });
        let mut c = r.class("rifleman").unwrap().clone();
        c.id = "pyro".into();
        c.weapon = "flamer".into();
        c.abilities.push("suppress".into());
        r.add_class(c);
        r.add_module(&crate::modules::GrenadierModule);
        let mut b = Battle::new(Map::from_ascii(&"..........\n".repeat(5)), r, 3);
        let a = b.spawn("pyro", "A", Team::Player, GridPos::new(0, 2)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(4, 2)).unwrap();
        let near = b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(2, 2)).unwrap();
        b.spawn("sectoid", "Far", Team::Enemy, GridPos::new(9, 4)).unwrap();
        let hp = b.unit(e).unwrap().hp;
        assert!(b.validate(&Action::ability(a, "suppress", Target::Unit(e))).is_err(), "no suppressing with a flamer");
        let previews = b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap();
        assert!(previews.iter().all(|(_, p)| p.hit_chance == 100) && previews.len() == 2, "{previews:?}");
        let ev = b.apply(Action::ability(a, "shoot", Target::Unit(e))).unwrap();
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Flames { .. })));
        assert!(!ev.iter().any(|x| matches!(x, GameEvent::Volley { .. })), "no bullets");
        assert_eq!(b.unit(e).unwrap().hp, hp - 2 - 2, "burned, then burning at its turn start");
        assert!(b.unit(near).unwrap().hp < hp);
        assert!(b.on_fire(GridPos::new(3, 2)));
        assert_eq!(b.unit(a).unwrap().weapon.ammo, 3, "one burst of fuel");
    }
}

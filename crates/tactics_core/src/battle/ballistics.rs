//! Ballistics: shots are bullets, not dice.
//!
//! A ballistic attack (single target, can miss: rifles, pistols, shotguns,
//! lasers, psi bolts...) fires `rounds` projectiles (shotguns: `pellets` per
//! round). Each one:
//!
//! 1. leaves the shooter's muzzle aimed at the target's centre of mass,
//! 2. is deflected by a random angular error: the shooter's **aim** sets it
//!    so that, at this range, a projectile hits with the aim chance (see
//!    [`crate::combat::sigma_for`]); the weapon's **spread** widens it
//!    (shotguns spray, snipers barely deviate),
//! 3. flies in a straight line and stops at the first thing it meets:
//!    * **any unit**: friend or foe (friendly fire is real);
//!    * the intended target's **cover**: a projectile that would hit the
//!      target is stopped by its cover with the `cover_block` chance (cover
//!      penalties become block chances, so odds stay comparable);
//!    * **walls** (full cover) and **low cover** it passes too low over,
//!      raised ground, destructible **props** (barrels explode);
//!    * or it flies off past its maximum range.
//!
//! Cover right next to the shooter never blocks (soldiers lean out), and
//! cover right next to the target is handled by the block roll above, not by
//! geometry, so it isn't counted twice.
//!
//! Projectiles that miss still frighten: the target and anyone they whizz
//! past gain fear. Everything is seeded: same battle, same bullets.

use super::Battle;
use crate::combat::{TARGET_RADIUS, preview_attack};
use crate::content::{AbilityId, AttackProfile};
use crate::event::{Bullet, GameEvent};
use crate::grid::{GridPos, Terrain};
use crate::unit::{Team, UnitId};

/// Metres per height level.
pub const LEVEL_M: f32 = 0.5;
/// Muzzle height above the shooter's ground, metres.
pub const MUZZLE_M: f32 = 0.9;
/// Aim point (centre of mass) above the target's ground, metres.
pub const CHEST_M: f32 = 0.7;
/// A unit's body height, metres.
pub const BODY_M: f32 = 1.25;
/// Heights of cover, metres.
pub const LOW_COVER_M: f32 = 0.6;
pub const HIGH_COVER_M: f32 = 1.8;
/// Ray-march step, tiles.
const STEP: f32 = 0.05;

/// A gaussian sample (Box-Muller) from the battle's RNG.
fn gaussian(rng: &mut crate::rng::GameRng) -> f32 {
    let u1 = ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64).max(1e-12);
    let u2 = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
    ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
}

/// Where a traced projectile ended.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stop {
    Unit(UnitId),
    Cover,
    Wall(GridPos),
    Ground,
    Away,
}

impl Battle {
    /// Fires a ballistic volley from `attacker` at `target` and applies the
    /// results. `rounds_cap` limits rounds (ammo left); returns rounds fired.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fire_volley(
        &mut self,
        attacker: UnitId,
        target: UnitId,
        ability: &AbilityId,
        profile: &AttackProfile,
        rounds_cap: Option<u32>,
        reaction: bool,
        suppress: bool,
        events: &mut Vec<GameEvent>,
    ) -> u32 {
        let shooter = self.units[attacker.0 as usize].clone();
        let aimed = self.units[target.0 as usize].clone();
        let preview = preview_attack(&self.rules.combat, &self.map, shooter.pos, &shooter, &aimed, profile);
        let per_round = match profile.damage {
            crate::content::DamageSource::Weapon => shooter.weapon.pellets.max(1),
            crate::content::DamageSource::Fixed { .. } => 1,
        };
        let rounds = (preview.rounds / per_round).max(1);
        let rounds = rounds_cap.map_or(rounds, |cap| rounds.min(cap)).max(1);

        let origin = [shooter.pos.x as f32, shooter.pos.y as f32, self.map.elevation(shooter.pos) as f32 * LEVEL_M + MUZZLE_M];
        let aim = [aimed.pos.x as f32, aimed.pos.y as f32, self.map.elevation(aimed.pos) as f32 * LEVEL_M + CHEST_M];
        let max_range = shooter.weapon.max_range.max(preview.distance) * 1.4 + 2.0;
        let block = preview.cover_block as f32 / 100.0;

        // Units that are still standing as the volley goes on (no damage is
        // applied until the whole volley is traced, so dead units don't
        // absorb later bullets).
        let mut hp: Vec<i32> = self.units.iter().map(|u| u.hp).collect();
        let mut bullets = Vec::new();
        let mut whizzed: Vec<UnitId> = Vec::new();
        for _ in 0..rounds * per_round {
            let (dx, dy) = (gaussian(&mut self.rng) * preview.sigma, gaussian(&mut self.rng) * preview.sigma);
            let (stop, end, passed) = self.trace(attacker, target, origin, aim, dx, dy, max_range, block, &hp);
            for u in passed {
                if !whizzed.contains(&u) {
                    whizzed.push(u);
                }
            }
            let mut bullet = Bullet { end, hit: None, prop: None, damage: 0, critical: false, blocked: stop == Stop::Cover };
            match stop {
                Stop::Unit(u) => {
                    let crit = self.rng.roll_percent(preview.crit_chance);
                    let (lo, hi) = preview.damage;
                    let dmg = (self.rng.range_inclusive(lo, hi) + if crit { preview.crit_damage } else { 0 }).max(0);
                    hp[u.0 as usize] -= dmg;
                    bullet.hit = Some(u);
                    bullet.damage = dmg;
                    bullet.critical = crit;
                }
                Stop::Wall(p) => {
                    bullet.prop = self.props.iter().find(|q| q.alive && q.pos == p && q.hp.is_some()).map(|q| q.id);
                }
                _ => {}
            }
            bullets.push(bullet);
        }

        events.push(GameEvent::Volley {
            attacker,
            ability: ability.clone(),
            target,
            origin,
            hit_chance: preview.hit_chance,
            reaction,
            suppress,
            bullets: bullets.clone(),
        });

        // Apply: damage per hit in order (deaths, scripts, fear), props.
        for b in &bullets {
            if self.is_over() {
                break;
            }
            if let Some(u) = b.hit {
                self.apply_damage_from(u, b.damage, false, Some(attacker), events);
            }
            if let Some(p) = b.prop {
                self.damage_prop(p, b.damage.max(1), events);
            }
        }
        // Being shot at is frightening: the target if nothing hit it, and
        // the shooter's enemies the bullets whizzed past.
        let shot_at = self.rules.combat.morale.shot_at;
        if !bullets.iter().any(|b| b.hit == Some(target)) {
            self.add_fear(target, shot_at, events);
        }
        for u in whizzed {
            if u != target && self.units[u.0 as usize].team != shooter.team && !bullets.iter().any(|b| b.hit == Some(u)) {
                self.add_fear(u, shot_at / 2, events);
            }
        }
        rounds
    }

    /// Marches one projectile. Returns where it stopped, its end point and
    /// the units it passed within a metre of.
    #[allow(clippy::too_many_arguments)]
    fn trace(
        &mut self,
        shooter: UnitId,
        target: UnitId,
        origin: [f32; 3],
        aim: [f32; 3],
        yaw_err: f32,
        pitch_err: f32,
        max_range: f32,
        block: f32,
        hp: &[i32],
    ) -> (Stop, [f32; 3], Vec<UnitId>) {
        // Direction: towards the aim point, rotated by the errors.
        let (vx, vy, vz) = (aim[0] - origin[0], aim[1] - origin[1], aim[2] - origin[2]);
        let horiz = (vx * vx + vy * vy).sqrt().max(1e-4);
        let yaw = vy.atan2(vx) + yaw_err;
        let pitch = vz.atan2(horiz) + pitch_err;
        let dir = [yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()];

        let shooter_pos = self.units[shooter.0 as usize].pos;
        let target_pos = self.units[target.0 as usize].pos;
        // Candidate units near the line (cheap pre-filter).
        let candidates: Vec<(UnitId, f32, f32, f32)> = self
            .units
            .iter()
            .filter(|u| u.id != shooter && hp[u.id.0 as usize] > 0)
            .map(|u| (u.id, u.pos.x as f32, u.pos.y as f32, self.map.elevation(u.pos) as f32 * LEVEL_M))
            .filter(|(_, x, y, _)| {
                // Distance from the infinite line in the horizontal plane.
                let (rx, ry) = (x - origin[0], y - origin[1]);
                let along = rx * dir[0] + ry * dir[1];
                let hlen = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-4);
                along > 0.0 && ((rx * dir[1] - ry * dir[0]) / hlen).abs() < 1.2
            })
            .collect();
        let mut passed = Vec::new();
        let mut t = 0.3; // start clear of the shooter's own body
        while t < max_range {
            let p = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
            let tile = GridPos::new(p[0].round() as i32, p[1].round() as i32);
            if !self.map.in_bounds(tile) {
                return (Stop::Away, p, passed);
            }
            // Units.
            for &(id, ux, uy, ground) in &candidates {
                let d = ((p[0] - ux).powi(2) + (p[1] - uy).powi(2)).sqrt();
                if d < 1.0 && !passed.contains(&id) && id != target {
                    passed.push(id);
                }
                if d <= TARGET_RADIUS && p[2] >= ground && p[2] <= ground + BODY_M {
                    if id == target && block > 0.0 && self.rng.roll_percent((block * 100.0).round() as i32) {
                        // Stopped by the target's cover, just in front of it.
                        let back = [p[0] - dir[0] * 0.35, p[1] - dir[1] * 0.35, p[2] - dir[2] * 0.35];
                        return (Stop::Cover, back, passed);
                    }
                    return (Stop::Unit(id), p, passed);
                }
            }
            // Terrain.
            let ground = self.map.elevation(tile) as f32 * LEVEL_M;
            let near_shooter = tile.chebyshev(shooter_pos) <= 1;
            let near_target = tile.chebyshev(target_pos) <= 1 && tile != target_pos;
            match self.map.terrain(tile) {
                Terrain::Void => {}
                _ if p[2] < ground && tile != shooter_pos => return (Stop::Ground, p, passed),
                Terrain::HighCover if !near_shooter && !near_target && p[2] < ground + HIGH_COVER_M => return (Stop::Wall(tile), p, passed),
                Terrain::LowCover if !near_shooter && !near_target && p[2] < ground + LOW_COVER_M => return (Stop::Wall(tile), p, passed),
                _ => {}
            }
            if p[2] < -1.0 {
                return (Stop::Ground, p, passed);
            }
            t += STEP;
        }
        let p = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
        (Stop::Away, p, passed)
    }

    /// Unit of `team` that would be hit standing between `from` and `to`
    /// (for the AI's friendly-fire check): any within ~0.6 tiles of the line.
    pub fn units_in_line_of_fire(&self, from: GridPos, to: GridPos, team: Team) -> Vec<UnitId> {
        let (ax, ay, bx, by) = (from.x as f32, from.y as f32, to.x as f32, to.y as f32);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-4);
        self.living(team)
            .filter(|u| u.pos != from && u.pos != to)
            .filter(|u| {
                let (px, py) = (u.pos.x as f32 - ax, u.pos.y as f32 - ay);
                let t = (px * dx + py * dy) / len2;
                t > 0.0 && t < 1.0 && ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt() < 0.6
            })
            .map(|u| u.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, ActionError, Target};
    use crate::battle::Battle;
    use crate::content::{Loadout, Ruleset};
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::modules::GrenadierModule;
    use crate::unit::{Team, UnitId};

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    fn bullets(events: &[GameEvent]) -> Vec<crate::event::Bullet> {
        events
            .iter()
            .filter_map(|e| match e {
                GameEvent::Volley { bullets, .. } => Some(bullets.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn shoot(b: &mut Battle, a: UnitId, t: UnitId) -> Vec<GameEvent> {
        b.apply(Action::ability(a, "shoot", Target::Unit(t))).unwrap()
    }

    #[test]
    fn friendly_fire_hits_soldiers_in_the_line_of_fire() {
        let mut ally_hits = 0;
        for seed in 0..40 {
            let mut b = Battle::new(Map::from_ascii("............"), rules(), seed);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
            let ally = b.spawn("rifleman", "Ally", Team::Player, GridPos::new(4, 0)).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 0)).unwrap();
            assert_eq!(b.units_in_line_of_fire(GridPos::new(0, 0), GridPos::new(9, 0), Team::Player), vec![ally]);
            let ev = shoot(&mut b, a, e);
            ally_hits += bullets(&ev).iter().filter(|x| x.hit == Some(ally)).count();
        }
        assert!(ally_hits > 20, "standing in the line of fire is dangerous ({ally_hits} hits)");
    }

    #[test]
    fn misses_fly_on_and_walls_stop_them() {
        // A wall row just above the line of fire: high strays stop in it.
        let mut wall_stops = 0;
        for seed in 0..30 {
            let mut b = Battle::new(
                Map::from_ascii(
                    "..HHHHHH..\n\
                     ..........",
                ),
                rules(),
                seed,
            );
            let a = b.spawn("sniper", "A", Team::Player, GridPos::new(0, 0)).unwrap();
            b.units[a.0 as usize].stats.aim = 5; // terrible shot: lots of strays
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 0)).unwrap();
            for x in bullets(&shoot(&mut b, a, e)) {
                if x.hit.is_none() && !x.blocked {
                    let in_wall_row = x.end[1] > 0.5 && x.end[0] > 1.5 && x.end[0] < 7.5;
                    let flew_on = x.end[0] > 9.0 || x.end[2] < 0.05 || x.end[1] < -0.3 || x.end[1] > 1.4;
                    assert!(in_wall_row || flew_on, "a miss stops in the wall or flies on: {:?}", x.end);
                    wall_stops += in_wall_row as i32;
                }
            }
        }
        assert!(wall_stops > 0, "some strays hit the wall");
    }

    #[test]
    fn shotgun_pellets_spread_with_range() {
        let r = rules();
        let at = |d: i32| {
            let mut b = Battle::new(Map::from_ascii("................"), r.clone(), 1);
            let a = b.spawn_equipped("assault", "A", Team::Player, GridPos::new(0, 0), None, &Loadout { weapon: Some("shotgun".into()), ..Default::default() }).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(d, 0)).unwrap();
            b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.clone()
        };
        let (close, far) = (at(2), at(9));
        assert_eq!(close.rounds, 8, "eight pellets");
        assert!(close.hit_chance > far.hit_chance + 25, "close {} vs far {}", close.hit_chance, far.hit_chance);
        assert!(close.expected_damage() > far.expected_damage() * 2.0);
    }

    #[test]
    fn cover_blocks_rounds() {
        let mut blocked = 0;
        let mut hits = 0;
        for seed in 0..20 {
            // Low cover right in front of the target.
            let mut b = Battle::new(Map::from_ascii("........h."), rules(), seed);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(9, 0)).unwrap();
            b.units[e.0 as usize].hp = 99;
            let ev = shoot(&mut b, a, e);
            let bs = bullets(&ev);
            blocked += bs.iter().filter(|x| x.blocked).count();
            hits += bs.iter().filter(|x| x.hit == Some(e)).count();
            assert!(bs.iter().filter(|x| x.blocked).all(|x| x.end[0] < 9.0), "stopped in front of the target");
        }
        assert!(blocked > 5 && hits > 5, "cover stops some rounds ({blocked}) but not all ({hits})");
    }

    #[test]
    fn magazines_run_out_and_pouches_add_more() {
        let mut b = Battle::new(Map::from_ascii("........"), rules(), 3);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        let p = b.spawn_equipped("rifleman", "P", Team::Player, GridPos::new(1, 0), None, &Loadout { items: vec!["ammo_pouch".into()], ..Default::default() }).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(7, 0)).unwrap();
        assert_eq!(b.unit(a).unwrap().weapon.spare, 3);
        assert_eq!(b.unit(p).unwrap().weapon.spare, 5, "ammo pouch: +2");
        // Empty the gun and reload until the spares are gone.
        for round in 0..4 {
            let u = b.units[a.0 as usize].clone();
            b.units[a.0 as usize].weapon.ammo = 0;
            let r = b.validate(&Action::ability(a, "reload", Target::None));
            if round < 3 {
                assert!(r.is_ok(), "{r:?}");
                b.apply(Action::ability(a, "reload", Target::None)).unwrap();
                assert_eq!(b.unit(a).unwrap().weapon.ammo, u.weapon.clip_size);
                b.apply(Action::EndTurn).unwrap();
                b.apply(Action::EndTurn).unwrap();
            } else {
                assert_eq!(r, Err(ActionError::OutOfMagazines));
            }
        }
    }

    #[test]
    fn suppression_pins_and_punishes_movement() {
        let mut b = Battle::new(
            Map::from_ascii(
                "..........\n\
                 ..........\n\
                 ..........",
            ),
            rules(),
            9,
        );
        let g = b.spawn("grenadier", "G", Team::Player, GridPos::new(0, 1)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(8, 1)).unwrap();
        b.units[e.0 as usize].hp = 99;
        b.units[e.0 as usize].stats.max_hp = 99;
        let ev = b.apply(Action::ability(g, "suppress", Target::Unit(e))).unwrap();
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Volley { suppress: true, bullets, .. } if bullets.len() == 12)));
        assert!(b.unit(e).unwrap().status.suppressed_by.is_some());
        assert_eq!(b.unit(g).unwrap().weapon.ammo, 50 - 12);
        // The pinned unit aims worse...
        let me = b.unit(e).unwrap().clone();
        let p = b.preview(e, &"shoot".into(), Target::Unit(g)).unwrap()[0].1.clone();
        assert!(p.breakdown.iter().any(|(l, v)| l == "Suppressed" && *v == -25), "{:?}", p.breakdown);
        let _ = me;
        // ...and moving draws fire.
        assert_eq!(b.active_team(), Team::Enemy);
        let ev = b.apply(Action::Move { unit: e, to: GridPos::new(7, 2) }).unwrap();
        assert!(ev.iter().any(|x| matches!(x, GameEvent::Volley { reaction: true, attacker, .. } if *attacker == g)), "{ev:?}");
        assert!(b.unit(e).unwrap().status.suppressed_by.is_none(), "one reaction only");
    }

    #[test]
    fn same_seed_same_bullets() {
        let run = || {
            let mut b = Battle::new(Map::from_ascii("..........\n.........."), rules(), 77);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(8, 1)).unwrap();
            bullets(&shoot(&mut b, a, e))
        };
        assert_eq!(run(), run());
    }
}

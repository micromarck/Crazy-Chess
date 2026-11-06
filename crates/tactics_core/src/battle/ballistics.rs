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
/// Heights of cover, metres. Unit heights (muzzle, aim point, hitbox) come
/// from each unit's [`Body`](crate::unit::Body).
pub use crate::combat::{HIGH_COVER_M, LOW_COVER_M};
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
        // Corner peeking: the shooter may lean out (and the target may be
        // leaning out) past a wall corner.
        // Big targets are shot at their nearest tile, aimed at the middle.
        let (from, at) = self.shot_line(shooter.nearest_tile(aimed.pos), aimed.nearest_tile(shooter.pos));
        let preview = preview_attack(&self.rules.combat, &self.map, from, &shooter, &aimed, profile);
        let per_round = match profile.damage {
            crate::content::DamageSource::Weapon => shooter.weapon.pellets.max(1),
            crate::content::DamageSource::Fixed { .. } => 1,
        };
        let rounds = (preview.rounds / per_round).max(1);
        let rounds = rounds_cap.map_or(rounds, |cap| rounds.min(cap)).max(1);

        let origin = [from.x as f32, from.y as f32, self.map.elevation(from) as f32 * LEVEL_M + shooter.body.muzzle()];
        let (ax, ay) = if aimed.side() > 1 { aimed.center() } else { (at.x as f32, at.y as f32) };
        let aim = [ax, ay, self.map.elevation(at) as f32 * LEVEL_M + aimed.body.aim_point()];
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
            let (stop, end, passed) = self.trace(attacker, target, (from, at), origin, aim, dx, dy, max_range, block, &hp);
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
        // Gunfire is loud: dormant units that can hear it come running.
        self.hear_gunfire(shooter.pos, events);

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
        // Where the shooter and the target are for this shot (lean tiles
        // when peeking around a corner).
        (shooter_pos, target_pos): (GridPos, GridPos),
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

        // Candidate units near the line (cheap pre-filter).
        // (id, x, y, hitbox bottom, hitbox top, hitbox radius)
        let candidates: Vec<(UnitId, f32, f32, f32, f32, f32)> = self
            .units
            .iter()
            .filter(|u| u.id != shooter && hp[u.id.0 as usize] > 0)
            .map(|u| {
                // A target leaning out of cover is hit where it leans; big
                // units are hit anywhere on their footprint.
                let pos = if u.id == target && u.side() == 1 { target_pos } else { u.pos };
                let (x, y) = if u.side() > 1 { u.center() } else { (pos.x as f32, pos.y as f32) };
                let ground = self.map.elevation(pos) as f32 * LEVEL_M;
                let (lo, hi) = u.body.span();
                (u.id, x, y, ground + lo, ground + hi, TARGET_RADIUS + (u.side() - 1) as f32 * 0.5)
            })
            .filter(|(_, x, y, _, _, r)| {
                // Distance from the infinite line in the horizontal plane.
                let (rx, ry) = (x - origin[0], y - origin[1]);
                let along = rx * dir[0] + ry * dir[1];
                let hlen = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-4);
                along > -r && ((rx * dir[1] - ry * dir[0]) / hlen).abs() < 1.2 + r
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
            for &(id, ux, uy, bottom, top, radius) in &candidates {
                let d = ((p[0] - ux).powi(2) + (p[1] - uy).powi(2)).sqrt();
                if d < 1.0 && !passed.contains(&id) && id != target {
                    passed.push(id);
                }
                if d <= radius && p[2] >= bottom && p[2] <= top {
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

    /// Grenade fragments: `count` rays thrown out of a blast at `center` in
    /// every direction (evenly spread, with jitter, mostly flat), reaching
    /// `radius + 5` tiles. Each hits the first unit in its path for 1 damage
    /// (friend or foe; units inside the blast already took the blast) and
    /// stops on walls, cover it flies into and the ground.
    pub(super) fn shrapnel(&mut self, attacker: Option<UnitId>, center: GridPos, radius: f32, count: u32, events: &mut Vec<GameEvent>) {
        let ground = self.map.elevation(center) as f32 * LEVEL_M;
        let origin = [center.x as f32, center.y as f32, ground + 0.4];
        let range = radius + 5.0;
        let candidates: Vec<(UnitId, f32, f32, f32, f32, f32)> = self
            .units
            .iter()
            .filter(|u| u.is_alive() && u.pos.distance(center) > radius && u.pos.distance(center) <= range + 1.0)
            .map(|u| {
                let g = self.map.elevation(u.pos) as f32 * LEVEL_M;
                let (lo, hi) = u.body.span();
                let (x, y) = u.center();
                (u.id, x, y, g + lo, g + hi, TARGET_RADIUS + (u.side() - 1) as f32 * 0.5)
            })
            .collect();
        let mut hp: Vec<i32> = self.units.iter().map(|u| u.hp).collect();
        let mut fragments = Vec::new();
        for i in 0..count {
            let jitter = (self.rng.next_u64() >> 11) as f32 / (1u64 << 53) as f32;
            let yaw = (i as f32 + jitter) / count as f32 * std::f32::consts::TAU;
            let pitch = 0.05 + gaussian(&mut self.rng) * 0.12;
            let dir = [yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()];
            let mut t = 0.3;
            let mut end = origin;
            let mut hit = None;
            'fly: while t < range {
                let p = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
                end = p;
                let tile = GridPos::new(p[0].round() as i32, p[1].round() as i32);
                if !self.map.in_bounds(tile) {
                    break;
                }
                for &(id, ux, uy, bottom, top, r) in &candidates {
                    if hp[id.0 as usize] > 0
                        && ((p[0] - ux).powi(2) + (p[1] - uy).powi(2)).sqrt() <= r
                        && p[2] >= bottom
                        && p[2] <= top
                    {
                        hit = Some(id);
                        hp[id.0 as usize] -= 1;
                        break 'fly;
                    }
                }
                let g = self.map.elevation(tile) as f32 * LEVEL_M;
                let blocked = match self.map.terrain(tile) {
                    Terrain::Void => false,
                    _ if p[2] < g => true,
                    Terrain::HighCover => tile != center && p[2] < g + HIGH_COVER_M,
                    Terrain::LowCover => tile != center && p[2] < g + LOW_COVER_M,
                    _ => false,
                };
                if blocked || p[2] < -1.0 {
                    break;
                }
                t += 0.1;
            }
            fragments.push(Bullet { end, hit, prop: None, damage: hit.map_or(0, |_| 1), critical: false, blocked: false });
        }
        events.push(GameEvent::Shrapnel { attacker, center: origin, fragments: fragments.clone() });
        for f in &fragments {
            if self.is_over() {
                break;
            }
            if let Some(u) = f.hit {
                self.apply_damage_from(u, f.damage, false, attacker, events);
            }
        }
    }

    /// Units of `team` that would be hit standing between `from` and `to`
    /// (for the AI's friendly-fire check): any within ~0.6 tiles of the line
    /// whose hitbox the shot passes through, so a drone firing over an
    /// ally's head (or at a drone above it) does not count it.
    pub fn units_in_line_of_fire(&self, from: GridPos, to: GridPos, team: Team) -> Vec<UnitId> {
        let (ax, ay, bx, by) = (from.x as f32, from.y as f32, to.x as f32, to.y as f32);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-4);
        let ground = |p: GridPos| self.map.elevation(p) as f32 * LEVEL_M;
        let body_at = |p: GridPos| self.unit_at(p).map(|u| u.body).unwrap_or_default();
        let h0 = ground(from) + body_at(from).muzzle();
        let h1 = ground(to) + body_at(to).aim_point();
        self.living(team)
            .filter(|u| u.pos != from && u.pos != to)
            .filter(|u| {
                let (px, py) = (u.pos.x as f32 - ax, u.pos.y as f32 - ay);
                let t = (px * dx + py * dy) / len2;
                if !(t > 0.0 && t < 1.0 && ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt() < 0.6) {
                    return false;
                }
                let h = h0 + (h1 - h0) * t;
                let (lo, hi) = u.body.span();
                let g = ground(u.pos);
                h >= g + lo - 0.15 && h <= g + hi + 0.15
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

    /// Corner peeking: at the end of a wall a soldier leans out and shoots
    /// past the corner (the rounds leave from the lean tile); in the middle
    /// of the wall there's no shot.
    #[test]
    fn soldiers_peek_around_corners() {
        let art = "
            ...E..
            ......
            HHHH..
            ...A..
        ";
        let mut hits = 0;
        for seed in 0..20 {
            let mut b = Battle::new(Map::from_ascii(art), rules(), seed);
            let a = b.spawn("rifleman", "A", Team::Player, Map::find_markers(art, 'A')[0]).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, Map::find_markers(art, 'E')[0]).unwrap();
            assert!(!crate::visibility::has_line_of_sight(b.map(), GridPos::new(3, 0), GridPos::new(3, 3)));
            assert_eq!(b.visible_enemies(a), vec![e], "sees it by leaning out");
            assert_eq!(b.visible_enemies(e), vec![a], "and is seen");
            let ev = shoot(&mut b, a, e);
            let origin = ev.iter().find_map(|x| if let GameEvent::Volley { origin, .. } = x { Some(*origin) } else { None }).unwrap();
            assert_eq!((origin[0], origin[1]), (4.0, 0.0), "fired from the lean tile");
            hits += bullets(&ev).iter().filter(|x| x.hit == Some(e)).count();
        }
        assert!(hits > 5, "peeking shots land ({hits})");

        let art = "
            .E....
            ......
            HHHH..
            .A....
        ";
        let mut b = Battle::new(Map::from_ascii(art), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, Map::find_markers(art, 'A')[0]).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, Map::find_markers(art, 'E')[0]).unwrap();
        assert!(b.visible_enemies(a).is_empty(), "nothing to lean around mid-wall");
        assert!(b.apply(Action::ability(a, "shoot", Target::Unit(e))).is_err());
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

    /// The standard rules plus a hovering "flyer" class (like the drone).
    fn rules_with_flyer() -> Ruleset {
        let mut r = rules();
        let mut flyer = r.class("rifleman").unwrap().clone();
        flyer.id = "flyer".into();
        flyer.body = crate::unit::Body { hover: 1.6, height: 0.55, muzzle: Some(1.65) };
        r.add_class(flyer);
        r
    }

    #[test]
    fn flyers_shoot_over_heads_and_are_aimed_at_high() {
        let (mut over, mut low) = (0, 0);
        for seed in 0..40 {
            // A flyer and a soldier both fire past an ally at a target.
            let mut b = Battle::new(Map::from_ascii("..........\n..........\n.........."), rules_with_flyer(), seed);
            let flyer = b.spawn("flyer", "F", Team::Player, GridPos::new(0, 0)).unwrap();
            let soldier = b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 2)).unwrap();
            let ally_a = b.spawn("rifleman", "Ally A", Team::Player, GridPos::new(2, 0)).unwrap();
            let ally_b = b.spawn("rifleman", "Ally B", Team::Player, GridPos::new(2, 2)).unwrap();
            let e1 = b.spawn("sectoid", "E1", Team::Enemy, GridPos::new(8, 0)).unwrap();
            let e2 = b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(8, 2)).unwrap();
            over += bullets(&shoot(&mut b, flyer, e1)).iter().filter(|x| x.hit == Some(ally_a)).count();
            low += bullets(&shoot(&mut b, soldier, e2)).iter().filter(|x| x.hit == Some(ally_b)).count();
        }
        assert!(low > 20, "a standing shooter hits the ally in the way ({low})");
        assert!(over * 4 < low, "a flyer fires over its ally's head ({over} vs {low})");

        // The AI's friendly-fire check knows it too.
        let mut b = Battle::new(Map::from_ascii("........."), rules_with_flyer(), 1);
        b.spawn("flyer", "F", Team::Player, GridPos::new(0, 0)).unwrap();
        b.spawn("rifleman", "Ally", Team::Player, GridPos::new(2, 0)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(8, 0)).unwrap();
        assert!(b.units_in_line_of_fire(GridPos::new(0, 0), GridPos::new(8, 0), Team::Player).is_empty());
    }

    #[test]
    fn flyers_float_above_half_cover_but_not_full() {
        let rules = rules_with_flyer();
        let preview = |map: &str, class: &str| {
            let mut b = Battle::new(Map::from_ascii(map), rules.clone(), 1);
            let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
            let t = b.spawn(class, "T", Team::Enemy, GridPos::new(6, 0)).unwrap();
            let (a, t) = (b.unit(a).unwrap(), b.unit(t).unwrap());
            let profile = b.rules().ability(&"shoot".into()).unwrap().effect.attack_profile().unwrap().clone();
            crate::combat::preview_attack(&b.rules().combat, b.map(), a.pos, a, t, &profile).cover_block
        };
        assert!(preview(".....h.", "rifleman") > 0, "half cover protects a soldier");
        assert_eq!(preview(".....h.", "flyer"), 0, "but not a flyer above it");
        assert!(preview(".....H.", "flyer") > 0, "full cover still hides a flyer");
    }

    #[test]
    fn grenade_shrapnel_flies_past_the_blast_and_walls_stop_it() {
        let (mut open, mut sheltered) = (0, 0);
        for seed in 0..30 {
            // Two soldiers 3 tiles from the blast: one in the open, one
            // behind a high wall (on the far side of it).
            let mut b = Battle::new(Map::from_ascii("...........\n...........\n.....H.....\n..........."), rules(), seed);
            let g = b.spawn("grenadier", "G", Team::Player, GridPos::new(0, 3)).unwrap();
            let in_open = b.spawn("rifleman", "Open", Team::Enemy, GridPos::new(8, 3)).unwrap();
            let behind = b.spawn("rifleman", "Behind", Team::Enemy, GridPos::new(5, 0)).unwrap();
            let ev = b.apply(Action::ability(g, "frag_grenade", Target::Tile(GridPos::new(5, 3)))).unwrap();
            let frags: Vec<crate::event::Bullet> = ev
                .iter()
                .filter_map(|e| match e {
                    GameEvent::Shrapnel { fragments, .. } => Some(fragments.clone()),
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(frags.len(), 16);
            open += frags.iter().filter(|f| f.hit == Some(in_open)).count();
            sheltered += frags.iter().filter(|f| f.hit == Some(behind)).count();
        }
        assert!(open >= 10, "fragments reach units outside the blast ({open})");
        assert!(sheltered * 3 < open.max(1), "walls stop fragments ({sheltered} vs {open})");
    }

    #[test]
    fn blasts_break_walls_into_rubble_then_flatten_them() {
        let mut b = Battle::new(Map::from_ascii("....\n.Hh.\n...."), rules(), 1);
        let g = b.spawn("grenadier", "G", Team::Player, GridPos::new(0, 0)).unwrap();
        let high = GridPos::new(1, 1);
        let low = GridPos::new(2, 1);
        assert_eq!(b.wall_hp(high), Some(6));
        assert_eq!(b.wall_hp(low), Some(3));
        let version = b.map_version();
        let ev = b.apply(Action::ability(g, "frag_grenade", Target::Tile(GridPos::new(2, 0)))).unwrap();
        assert_eq!(b.map().terrain(low), crate::grid::Terrain::Floor, "3-4 damage flattens low cover");
        assert!(ev.iter().any(|e| matches!(e, GameEvent::WallDestroyed { pos } if *pos == low)));
        assert!(b.wall_hp(high).unwrap() <= 3, "the high wall is cracked");
        assert!(b.map_version() > version);
        // A second blast brings the wall down to rubble: still half cover.
        b.explode(GridPos::new(1, 0), 1.5, 4, &mut Vec::new());
        assert_eq!(b.map().terrain(high), crate::grid::Terrain::LowCover);
        assert_eq!(b.map().surface(high), crate::grid::Surface::Rubble);
        assert_eq!(b.wall_hp(high), Some(3), "rubble can be blasted away too");
    }

    #[test]
    fn shotgun_is_accurate_up_close_and_cover_cannot_block_it() {
        let preview = |map: &str, d: i32| {
            let mut b = Battle::new(Map::from_ascii(map), rules(), 1);
            let gear = Loadout { weapon: Some("shotgun".into()), ..Default::default() };
            let a = b.spawn_equipped("assault", "A", Team::Player, GridPos::new(0, 0), None, &gear).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(d, 0)).unwrap();
            let (a, e) = (b.unit(a).unwrap(), b.unit(e).unwrap());
            let profile = b.rules().ability(&"shoot".into()).unwrap().effect.attack_profile().unwrap().clone();
            crate::combat::preview_attack(&b.rules().combat, b.map(), a.pos, a, e, &profile)
        };
        let open = preview("..........", 4);
        assert!(open.hit_chance >= 60, "each pellet hits often at 4 tiles: {}%", open.hit_chance);
        assert!(open.expected_damage() >= 6.0, "{}", open.expected_damage());
        for map in ["...h......", "...H......"] {
            let covered = preview(map, 4);
            assert_eq!(covered.cover_block, 0, "{map}: cover can't block pellets");
            assert_eq!(covered.hit_chance, open.hit_chance, "{map}: same odds as in the open");
        }
    }

    #[test]
    fn snipers_are_bad_up_close() {
        let aim_at = |d: i32| {
            let mut b = Battle::new(Map::from_ascii("................"), rules(), 1);
            let a = b.spawn("sniper", "S", Team::Player, GridPos::new(0, 0)).unwrap();
            let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(d, 0)).unwrap();
            b.preview(a, &"shoot".into(), Target::Unit(e)).unwrap()[0].1.aim_chance
        };
        let (adjacent, near, ok) = (aim_at(1), aim_at(3), aim_at(6));
        assert!(adjacent < near && near < ok, "{adjacent} < {near} < {ok}");
        assert!(ok - adjacent >= 40, "point blank costs a lot: {adjacent} vs {ok}");
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

//! Hit-chance and damage math for attack abilities.
//!
//! Everything here is a pure function, so the UI can show an exact preview
//! ("72% to hit, 18% crit") and the AI can evaluate attacks without rolling
//! dice. The rolls themselves happen in [`crate::battle`].
//!
//! All numbers come from data: the ability's [`AttackProfile`] (base accuracy,
//! cover penalties, crit and damage modifiers), the attacker's weapon and stats,
//! and the global [`CombatRules`]. Each term is recorded in
//! [`ShotPreview::breakdown`] so players can see *why* a shot is 35%.
//!
//! ```text
//! hit  = (profile.base_accuracy or attacker.aim) + profile.accuracy_bonus
//!        - target.defense - target.status.defense_bonus
//!        - profile.{half,full}_cover_penalty        (if in cover vs. origin)
//!        ± range modifier                             (if profile.range_falloff)
//!        + rules.height_advantage_bonus               (if firing from higher ground)
//! crit = weapon.crit (weapon damage only) + profile.crit_bonus
//!        + profile.flank_crit_bonus                   (if flanked)
//! ```

use crate::content::{AttackProfile, CombatRules, DamageSource};
use crate::cover::{CoverResult, cover_against};
use crate::grid::{CoverLevel, GridPos, Map};
use crate::unit::Unit;

/// Effective radius of a unit as a target, in tiles (for bullet geometry).
pub const TARGET_RADIUS: f32 = 0.3;
/// Heights of low (half) and high (full) cover, metres.
pub const LOW_COVER_M: f32 = 0.6;
pub const HIGH_COVER_M: f32 = 1.8;

/// Probability that a projectile with angular error `sigma` (radians, per
/// axis) lands within a target of angular radius `theta` (Rayleigh).
pub fn geometric_hit(theta: f32, sigma: f32) -> f32 {
    if sigma <= 1e-6 {
        return 1.0;
    }
    1.0 - (-(theta * theta) / (2.0 * sigma * sigma)).exp()
}

/// Angular error that gives hit probability `p` against a target of
/// angular radius `theta` (inverse of [`geometric_hit`]).
pub fn sigma_for(theta: f32, p: f32) -> f32 {
    let p = p.clamp(0.02, 0.995);
    theta / (-2.0 * (1.0 - p).ln()).sqrt()
}

/// Full description of a potential attack, before any dice are rolled.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotPreview {
    /// Chance that **each projectile** hits the target, in percent (100 for
    /// attacks that cannot miss). Includes cover and spread.
    pub hit_chance: i32,
    /// Projectiles fired (rounds × pellets). 1 for non-ballistic attacks.
    pub rounds: u32,
    /// Aim before cover (what sets the aim error of a ballistic shot).
    pub aim_chance: i32,
    /// Chance cover stops a projectile that would hit, percent.
    pub cover_block: i32,
    /// Angular error per projectile, radians (aim + weapon spread).
    pub sigma: f32,
    /// Final critical chance in percent (applies only on a hit).
    pub crit_chance: i32,
    /// Cover of the target as seen from the attack origin.
    pub cover: CoverResult,
    /// Distance in tiles from the attack origin.
    pub distance: f32,
    /// Damage range on a normal hit.
    pub damage: (i32, i32),
    /// Extra damage on a crit.
    pub crit_damage: i32,
    /// Human-readable modifiers, e.g. `("Half cover", -20)`.
    pub breakdown: Vec<(String, i32)>,
}

impl ShotPreview {
    /// Expected damage of the whole attack (all projectiles), used by the AI.
    pub fn expected_damage(&self) -> f32 {
        let avg = (self.damage.0 + self.damage.1) as f32 / 2.0;
        let p_hit = self.hit_chance as f32 / 100.0;
        let p_crit = self.crit_chance as f32 / 100.0;
        self.rounds.max(1) as f32 * p_hit * (avg + p_crit * self.crit_damage as f32)
    }

    /// Most damage the attack could do (every projectile hits and crits).
    pub fn max_damage(&self) -> i32 {
        self.rounds.max(1) as i32 * (self.damage.1 + self.crit_damage)
    }
}

/// Computes the preview of `attacker` hitting `target` with `profile`.
///
/// `origin` is where the attack comes from for cover and range purposes: the
/// attacker's tile for shots, the blast centre for area attacks. Legality
/// (range, line of sight, ammo) is checked separately by the battle.
pub fn preview_attack(
    rules: &CombatRules,
    map: &Map,
    origin: GridPos,
    attacker: &Unit,
    target: &Unit,
    profile: &AttackProfile,
) -> ShotPreview {
    let mut cover = cover_against(map, target.pos, origin);
    // A hovering unit is above cover lower than its hitbox.
    let cover_height = match cover.level {
        CoverLevel::Half => LOW_COVER_M,
        CoverLevel::Full => HIGH_COVER_M,
        CoverLevel::None => 0.0,
    };
    if cover.level != CoverLevel::None && target.body.above(cover_height) {
        cover.level = CoverLevel::None;
    }
    let distance = origin.distance(target.pos);
    let weapon = &attacker.weapon;
    let mut breakdown: Vec<(String, i32)> = Vec::new();
    let mut push = |label: &str, value: i32| {
        if value != 0 {
            breakdown.push((label.to_string(), value));
        }
    };

    match profile.base_accuracy {
        Some(base) => push("Base accuracy", base),
        None => push("Aim", attacker.stats.aim),
    }
    push("Ability", profile.accuracy_bonus);
    let fear = rules.morale.aim_modifier(attacker.fear);
    match rules.morale.level(attacker.fear) {
        crate::content::FearLevel::Panicked => push("Panicked", fear),
        _ => push("Shaken", fear),
    }
    if map.elevation(origin) > map.elevation(target.pos) {
        push("Height advantage", rules.height_advantage_bonus);
    }
    push("Defense", -target.stats.defense);
    push("Defending", -target.status.defense_bonus);
    if attacker.status.suppressed_by.is_some() {
        push("Suppressed", -rules.suppression_aim);
    }
    if profile.range_falloff {
        let range_mod = if distance > weapon.optimal_range {
            -((distance - weapon.optimal_range) * rules.falloff_per_tile).round() as i32
        } else {
            (((weapon.optimal_range - distance) * rules.close_bonus_per_tile).round() as i32)
                .min(rules.max_close_bonus)
        };
        push("Range", range_mod);
    }

    // Aim before cover; cover is applied below (as a penalty for rolled
    // attacks, as a per-projectile block chance for ballistic ones).
    // Global accuracy scaling (combat rules), shown as its own line.
    let raw: i32 = breakdown.iter().map(|(_, v)| *v).sum();
    let scaled = (raw.max(0) as f32 * rules.accuracy_multiplier).round() as i32;
    if scaled != raw.max(0) && raw > 0 {
        breakdown.push((format!("Accuracy x{}", rules.accuracy_multiplier), scaled - raw));
    }
    let aim_chance = if raw > 0 { scaled } else { raw }.clamp(rules.min_hit, rules.max_hit);
    let cover_penalty = match cover.level {
        CoverLevel::Half => profile.half_cover_penalty,
        CoverLevel::Full => profile.full_cover_penalty,
        CoverLevel::None => 0,
    };
    match cover.level {
        CoverLevel::Half => breakdown.push(("Half cover".into(), -cover_penalty)),
        CoverLevel::Full => breakdown.push(("Full cover".into(), -cover_penalty)),
        CoverLevel::None => {}
    }
    let weapon_shot = matches!(profile.damage, DamageSource::Weapon);
    let (hit_chance, rounds, cover_block, sigma) = if !profile.can_miss {
        (100, 1, 0, 0.0)
    } else if !profile.is_ballistic() {
        ((aim_chance - cover_penalty).clamp(rules.min_hit, rules.max_hit), 1, 0, 0.0)
    } else {
        // Ballistic: aim sets the angular error so that, ignoring spread and
        // cover, a projectile hits with `aim_chance`; spread widens it; cover
        // stops a share of the ones that would hit.
        let theta = (TARGET_RADIUS / distance.max(0.5)).atan();
        let aim_sigma = sigma_for(theta, aim_chance as f32 / 100.0);
        let spread = if weapon_shot { weapon.spread } else { 0.3 }.max(0.0).to_radians();
        let sigma = (aim_sigma * aim_sigma + spread * spread).sqrt();
        let block = if cover_penalty > 0 { (cover_penalty as f32 / aim_chance.max(1) as f32).clamp(0.0, 0.95) } else { 0.0 };
        let p = geometric_hit(theta, sigma) * (1.0 - block);
        let per_round = if weapon_shot { weapon.pellets.max(1) } else { 1 };
        let rounds = profile.rounds.unwrap_or(if weapon_shot { weapon.burst.max(1) } else { 1 }) * per_round;
        ((p * 100.0).round() as i32, rounds, (block * 100.0).round() as i32, sigma)
    };

    let (damage, crit_damage, weapon_crit) = match profile.damage {
        DamageSource::Weapon => ((weapon.damage_min, weapon.damage_max), weapon.crit_bonus, weapon.crit_chance),
        DamageSource::Fixed { min, max } => ((min, max), 0, 0),
    };
    let mut crit_chance = weapon_crit + profile.crit_bonus;
    if cover.flanked && profile.flank_crit_bonus != 0 {
        crit_chance += profile.flank_crit_bonus;
        breakdown.push(("Flanked (crit)".into(), profile.flank_crit_bonus));
    }

    ShotPreview {
        hit_chance,
        rounds,
        aim_chance,
        cover_block,
        sigma,
        crit_chance: crit_chance.clamp(0, 100),
        cover,
        distance,
        damage: (damage.0 + profile.damage_bonus, damage.1 + profile.damage_bonus),
        crit_damage,
        breakdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Ruleset;
    use crate::grid::Terrain;
    use crate::unit::{Team, UnitId, UnitStatus};

    fn make(team: Team, pos: GridPos) -> Unit {
        let rules = Ruleset::standard();
        let class = rules.class("rifleman").unwrap();
        Unit {
            id: UnitId(0),
            name: "t".into(),
            class: class.id.clone(),
            team,
            pos,
            hp: class.stats.max_hp,
            action_points: 2,
            stats: class.stats,
            weapon: rules.weapon(&class.weapon).unwrap().clone(),
            abilities: class.abilities.clone(),
            cooldowns: Default::default(),
            charges: Default::default(),
            status: UnitStatus::default(),
            ai: Default::default(),
            fear: 0,
            fearless: false,
            body: Default::default(),
            gear: Default::default(),
            appearance: None,
            model: String::new(),
        }
    }

    #[test]
    fn cover_penalties_come_from_the_profile() {
        let rules = CombatRules::default();
        let mut map = Map::new(10, 10);
        // Shooter south of target; half cover south of target.
        map.set_terrain(GridPos::new(5, 4), Terrain::LowCover);
        let shooter = make(Team::Player, GridPos::new(5, 0));
        let target = make(Team::Enemy, GridPos::new(5, 5));
        let open_map = Map::new(10, 10);

        let standard = AttackProfile::default();
        let precise = AttackProfile { half_cover_penalty: 5, ..Default::default() };

        let open = preview_attack(&rules, &open_map, shooter.pos, &shooter, &target, &standard);
        let covered = preview_attack(&rules, &map, shooter.pos, &shooter, &target, &standard);
        let covered_precise = preview_attack(&rules, &map, shooter.pos, &shooter, &target, &precise);

        assert!(open.cover.flanked);
        // Ballistic: cover doesn't change the aim, it blocks a share of the
        // projectiles that would hit (penalty / aim).
        assert_eq!(covered.aim_chance, open.aim_chance);
        assert_eq!(covered.cover_block, (20.0 / open.aim_chance as f32 * 100.0).round() as i32);
        assert_eq!(covered_precise.cover_block, (5.0 / open.aim_chance as f32 * 100.0).round() as i32);
        assert!(covered.hit_chance < covered_precise.hit_chance && covered_precise.hit_chance < open.hit_chance);
    }

    #[test]
    fn accuracy_multiplier_scales_aim_before_cover() {
        let map = Map::new(10, 10);
        let a = make(Team::Player, GridPos::new(0, 0));
        let b = make(Team::Enemy, GridPos::new(0, 6));
        let fixed = AttackProfile { base_accuracy: Some(50), range_falloff: false, ..Default::default() };
        let plain = preview_attack(&CombatRules { accuracy_multiplier: 1.0, ..Default::default() }, &map, a.pos, &a, &b, &fixed);
        let boosted = preview_attack(&CombatRules::default(), &map, a.pos, &a, &b, &fixed);
        assert_eq!(plain.aim_chance, 50);
        assert_eq!(boosted.aim_chance, 65, "1.3x by default");
        assert!(boosted.hit_chance > plain.hit_chance);
        assert!(boosted.breakdown.iter().any(|(l, v)| l.starts_with("Accuracy x") && *v == 15));
    }

    #[test]
    fn high_ground_bonus() {
        let rules = CombatRules { accuracy_multiplier: 1.0, ..Default::default() };
        let mut map = Map::new(10, 10);
        let a = make(Team::Player, GridPos::new(0, 0));
        let b = make(Team::Enemy, GridPos::new(0, 5));
        let flat = preview_attack(&rules, &map, a.pos, &a, &b, &AttackProfile::default()).aim_chance;
        map.set_elevation(a.pos, 2);
        let high = preview_attack(&rules, &map, a.pos, &a, &b, &AttackProfile::default()).aim_chance;
        assert_eq!(high, flat + rules.height_advantage_bonus);
    }

    #[test]
    fn base_accuracy_overrides_aim_and_can_miss_false_always_hits() {
        let rules = CombatRules { accuracy_multiplier: 1.0, ..Default::default() };
        let map = Map::new(10, 10);
        let a = make(Team::Player, GridPos::new(0, 0));
        let b = make(Team::Enemy, GridPos::new(0, 6));
        let fixed = AttackProfile { base_accuracy: Some(50), range_falloff: false, ..Default::default() };
        let p = preview_attack(&rules, &map, a.pos, &a, &b, &fixed);
        assert_eq!(p.aim_chance, 50);
        // Without spread the per-projectile chance is the aim; the weapon's
        // spread lowers it a little.
        assert!(p.hit_chance <= 50 && p.hit_chance >= 40, "{}", p.hit_chance);
        let sure = AttackProfile { can_miss: false, ..Default::default() };
        assert_eq!(preview_attack(&rules, &map, a.pos, &a, &b, &sure).hit_chance, 100);
    }
}

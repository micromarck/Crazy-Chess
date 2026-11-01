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
//! crit = weapon.crit (weapon damage only) + profile.crit_bonus
//!        + profile.flank_crit_bonus                   (if flanked)
//! ```

use crate::content::{AttackProfile, CombatRules, DamageSource};
use crate::cover::{CoverResult, cover_against};
use crate::grid::{CoverLevel, GridPos, Map};
use crate::unit::Unit;

/// Full description of a potential attack, before any dice are rolled.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotPreview {
    /// Final hit chance in percent (100 for attacks that cannot miss).
    pub hit_chance: i32,
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
    /// Expected damage, used by the AI to rank options.
    pub fn expected_damage(&self) -> f32 {
        let avg = (self.damage.0 + self.damage.1) as f32 / 2.0;
        let p_hit = self.hit_chance as f32 / 100.0;
        let p_crit = self.crit_chance as f32 / 100.0;
        p_hit * (avg + p_crit * self.crit_damage as f32)
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
    let cover = cover_against(map, target.pos, origin);
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
    push("Defense", -target.stats.defense);
    push("Defending", -target.status.defense_bonus);
    match cover.level {
        CoverLevel::Half => push("Half cover", -profile.half_cover_penalty),
        CoverLevel::Full => push("Full cover", -profile.full_cover_penalty),
        CoverLevel::None => {}
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

    let hit_chance = if profile.can_miss {
        breakdown.iter().map(|(_, v)| *v).sum::<i32>().clamp(rules.min_hit, rules.max_hit)
    } else {
        100
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
        assert_eq!(covered.hit_chance, open.hit_chance - 20);
        assert_eq!(covered_precise.hit_chance, open.hit_chance - 5);
    }

    #[test]
    fn base_accuracy_overrides_aim_and_can_miss_false_always_hits() {
        let rules = CombatRules::default();
        let map = Map::new(10, 10);
        let a = make(Team::Player, GridPos::new(0, 0));
        let b = make(Team::Enemy, GridPos::new(0, 6));
        let fixed = AttackProfile { base_accuracy: Some(50), range_falloff: false, ..Default::default() };
        assert_eq!(preview_attack(&rules, &map, a.pos, &a, &b, &fixed).hit_chance, 50);
        let sure = AttackProfile { can_miss: false, ..Default::default() };
        assert_eq!(preview_attack(&rules, &map, a.pos, &a, &b, &sure).hit_chance, 100);
    }
}

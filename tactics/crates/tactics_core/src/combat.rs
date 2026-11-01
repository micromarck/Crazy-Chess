//! Hit-chance and damage math.
//!
//! Everything here is a pure function of the battle state, so the UI can show
//! an exact preview ("72% to hit, 18% crit") and the AI can evaluate shots
//! without rolling dice. The actual rolls happen in [`crate::battle`].
//!
//! The formula, XCOM-style:
//!
//! ```text
//! hit  = aim - target.defense - cover_bonus + range_modifier [- reaction penalty]
//! crit = weapon.crit + (flanked ? FLANK_CRIT_BONUS : 0)
//! ```
//! Each term is recorded in a [`ShotPreview::breakdown`] so players can see *why*
//! a shot is 35%.

use crate::cover::{cover_against, CoverResult};
use crate::grid::{CoverLevel, Map};
use crate::unit::Unit;

/// Defense bonus granted by half cover.
pub const HALF_COVER_BONUS: i32 = 20;
/// Defense bonus granted by full cover.
pub const FULL_COVER_BONUS: i32 = 40;
/// Extra defense while hunkered (stacks with cover).
pub const HUNKER_BONUS: i32 = 30;
/// Extra crit chance against a flanked target.
pub const FLANK_CRIT_BONUS: i32 = 40;
/// Accuracy penalty for reaction (overwatch) shots.
pub const REACTION_PENALTY: i32 = 15;
/// Accuracy lost per tile beyond the weapon's optimal range.
pub const FALLOFF_PER_TILE: f32 = 3.0;
/// Accuracy gained per tile closer than optimal range (capped).
pub const CLOSE_BONUS_PER_TILE: f32 = 2.0;
/// Cap on the close-range bonus.
pub const MAX_CLOSE_BONUS: i32 = 15;

/// Hit-chance bounds. XCOM famously allows 100% shots; we keep a sliver of
/// uncertainty at the top and a "hail mary" at the bottom.
pub const MIN_HIT: i32 = 1;
pub const MAX_HIT: i32 = 99;

/// Full description of a potential shot, before any dice are rolled.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotPreview {
    /// Final, clamped hit chance in percent.
    pub hit_chance: i32,
    /// Final, clamped critical chance in percent (applies only on a hit).
    pub crit_chance: i32,
    /// Cover evaluation from the shooter's point of view.
    pub cover: CoverResult,
    /// Distance in tiles.
    pub distance: f32,
    /// Damage range on a normal hit.
    pub damage: (i32, i32),
    /// Human-readable modifiers, e.g. `("Half cover", -20)`.
    pub breakdown: Vec<(&'static str, i32)>,
}

impl ShotPreview {
    /// Expected damage of the shot, used by the AI to rank targets.
    pub fn expected_damage(&self, crit_bonus: i32) -> f32 {
        let avg = (self.damage.0 + self.damage.1) as f32 / 2.0;
        let p_hit = self.hit_chance as f32 / 100.0;
        let p_crit = self.crit_chance as f32 / 100.0;
        p_hit * (avg + p_crit * crit_bonus as f32)
    }
}

/// Computes the shot preview for `shooter` firing at `target`.
///
/// This does **not** check line of sight, range or ammo; that is the job of
/// [`crate::battle::Battle::validate`]. Keeping the math separate means it can be
/// evaluated for hypothetical positions (AI) as well.
pub fn preview_shot(map: &Map, shooter: &Unit, target: &Unit, reaction: bool) -> ShotPreview {
    let cover = cover_against(map, target.pos, shooter.pos);
    let distance = shooter.pos.distance(target.pos);
    let weapon = &shooter.weapon;
    let mut breakdown: Vec<(&'static str, i32)> = vec![("Aim", shooter.stats.aim)];

    if target.stats.defense != 0 {
        breakdown.push(("Defense", -target.stats.defense));
    }

    match cover.level {
        CoverLevel::Half => breakdown.push(("Half cover", -HALF_COVER_BONUS)),
        CoverLevel::Full => breakdown.push(("Full cover", -FULL_COVER_BONUS)),
        CoverLevel::None => {}
    }

    if target.status.hunkered {
        breakdown.push(("Hunkered", -HUNKER_BONUS));
    }

    // Range: lose accuracy beyond optimal, gain a little when closing in.
    let range_mod = if distance > weapon.optimal_range {
        -((distance - weapon.optimal_range) * FALLOFF_PER_TILE).round() as i32
    } else {
        (((weapon.optimal_range - distance) * CLOSE_BONUS_PER_TILE).round() as i32)
            .min(MAX_CLOSE_BONUS)
    };
    if range_mod != 0 {
        breakdown.push(("Range", range_mod));
    }

    if reaction {
        breakdown.push(("Reaction shot", -REACTION_PENALTY));
    }

    let hit_chance = breakdown
        .iter()
        .map(|(_, v)| *v)
        .sum::<i32>()
        .clamp(MIN_HIT, MAX_HIT);

    let mut crit_chance = weapon.crit_chance;
    if cover.flanked {
        crit_chance += FLANK_CRIT_BONUS;
        breakdown.push(("Flanked (crit)", FLANK_CRIT_BONUS));
    }

    ShotPreview {
        hit_chance,
        crit_chance: crit_chance.clamp(0, 100),
        cover,
        distance,
        damage: (weapon.damage_min, weapon.damage_max),
        breakdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::GridPos;
    use crate::unit::{Team, UnitId, UnitStatus, UnitTemplate};

    fn make(team: Team, pos: GridPos) -> Unit {
        let t = UnitTemplate::rifleman("t");
        Unit {
            id: UnitId(0),
            name: t.name,
            team,
            pos,
            hp: t.stats.max_hp,
            action_points: 2,
            stats: t.stats,
            weapon: t.weapon,
            status: UnitStatus::default(),
        }
    }

    #[test]
    fn cover_reduces_hit_chance() {
        let open = Map::new(10, 10);
        let mut covered = Map::new(10, 10);
        covered.set_terrain(GridPos::new(5, 6), crate::grid::Terrain::HighCover);

        let shooter = make(Team::Player, GridPos::new(5, 0));
        let target = make(Team::Enemy, GridPos::new(5, 5));

        // Shooter is south of target; place cover south of target instead.
        let mut south_cover = Map::new(10, 10);
        south_cover.set_terrain(GridPos::new(5, 4), crate::grid::Terrain::LowCover);

        let p_open = preview_shot(&open, &shooter, &target, false);
        let p_behind = preview_shot(&covered, &shooter, &target, false);
        let p_half = preview_shot(&south_cover, &shooter, &target, false);

        assert!(p_open.cover.flanked);
        assert_eq!(p_behind.hit_chance, p_open.hit_chance); // cover is on the wrong side
        assert_eq!(p_half.hit_chance, p_open.hit_chance - HALF_COVER_BONUS);
        assert!(p_open.crit_chance > p_half.crit_chance);
    }

    #[test]
    fn reaction_penalty_applies() {
        let map = Map::new(10, 10);
        let a = make(Team::Player, GridPos::new(0, 0));
        let b = make(Team::Enemy, GridPos::new(0, 6));
        let normal = preview_shot(&map, &a, &b, false);
        let reaction = preview_shot(&map, &a, &b, true);
        assert_eq!(normal.hit_chance - REACTION_PENALTY, reaction.hit_chance);
    }
}

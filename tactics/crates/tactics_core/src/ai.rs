//! A small utility-based AI.
//!
//! The AI answers one question: *"what should this unit do next?"* It returns
//! a single [`Action`]; the caller applies it, plays the resulting events, and
//! asks again. Deciding one action at a time keeps the AI reactive (a reaction
//! shot that kills an ally changes the next decision) and lets the frontend
//! animate between decisions.
//!
//! Decision procedure for a unit with action points:
//! 1. **Empty gun?** Reload.
//! 2. **Good shot available?** Shoot the target with the best score (expected
//!    damage, with a big bonus for likely kills) if the hit chance clears
//!    [`AiConfig::shoot_threshold`].
//! 3. **Reposition.** Score every tile reachable with *one* AP (so a shot is
//!    still possible afterwards) by cover against visible enemies, the best
//!    shot it would offer, and distance to the enemy. Move if it is clearly
//!    better than staying put. If no enemy is in sight from anywhere nearby,
//!    advance (dash allowed) towards the closest enemy.
//! 4. **Otherwise** take any shot, or overwatch, or hunker.
//!
//! The AI is currently omniscient (it knows where every enemy is). Plugging in
//! fog of war means filtering `battle.living(...)` through `team_can_see`.

use crate::action::Action;
use crate::combat::preview_shot;
use crate::cover::cover_against;
use crate::grid::{CoverLevel, GridPos};
use crate::battle::Battle;
use crate::unit::{Unit, UnitId};
use crate::visibility;

/// Tunable knobs, e.g. per difficulty level or per alien species.
#[derive(Debug, Clone, Copy)]
pub struct AiConfig {
    /// Minimum hit chance (percent) to take a shot instead of repositioning.
    pub shoot_threshold: i32,
    /// Score weight per point of cover bonus at a destination.
    pub cover_weight: f32,
    /// Score weight of the best hit chance a destination offers.
    pub offense_weight: f32,
    /// Penalty per tile of distance to the nearest enemy.
    pub distance_weight: f32,
    /// How much better a tile must score before the unit bothers to move.
    pub move_margin: f32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            shoot_threshold: 55,
            cover_weight: 1.0,
            offense_weight: 0.6,
            distance_weight: 1.5,
            move_margin: 8.0,
        }
    }
}

/// Chooses the next action for the active team. Returns [`Action::EndTurn`]
/// when no unit can act.
pub fn next_team_action(battle: &Battle, config: &AiConfig) -> Action {
    battle
        .living(battle.active_team())
        .find(|u| u.can_act())
        .map(|u| choose_action(battle, u.id, config))
        .unwrap_or(Action::EndTurn)
}

/// Chooses the next action for a single unit.
pub fn choose_action(battle: &Battle, id: UnitId, config: &AiConfig) -> Action {
    let Some(unit) = battle.unit(id) else {
        return Action::EndTurn;
    };
    if !unit.can_act() {
        return Action::EndTurn;
    }

    // 1. Reload when empty.
    if unit.weapon.ammo <= 0 {
        return Action::Reload { unit: id };
    }

    // 2. Take a good shot.
    let best_shot = best_target(battle, unit);
    if let Some((target, hit, _)) = best_shot
        && hit >= config.shoot_threshold {
            return Action::Shoot { unit: id, target };
        }

    // 3. Reposition.
    if let Some(to) = best_destination(battle, unit, config) {
        return Action::Move { unit: id, to };
    }

    // 4. Fallbacks.
    if let Some((target, _, _)) = best_shot {
        return Action::Shoot { unit: id, target };
    }
    if battle.validate(&Action::Overwatch { unit: id }).is_ok() {
        return Action::Overwatch { unit: id };
    }
    Action::Hunker { unit: id }
}

/// Best (target, hit chance, score) among enemies the unit can shoot now.
fn best_target(battle: &Battle, unit: &Unit) -> Option<(UnitId, i32, f32)> {
    battle
        .visible_enemies(unit.id)
        .into_iter()
        .filter_map(|t| {
            let target = battle.unit(t)?;
            let p = battle.shot_preview(unit.id, t).ok()?;
            let mut score = p.expected_damage(unit.weapon.crit_bonus);
            // Prefer shots likely to finish someone off.
            if p.damage.0 >= target.hp {
                score += 10.0 * p.hit_chance as f32 / 100.0;
            }
            Some((t, p.hit_chance, score))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
}

/// Scores standing on `pos` (cover vs. threats + offensive potential + distance).
fn score_tile(battle: &Battle, unit: &Unit, pos: GridPos, config: &AiConfig) -> f32 {
    let map = battle.map();
    // Evaluate as if the unit were standing on `pos`.
    let mut ghost = unit.clone();
    ghost.pos = pos;

    let mut score = 0.0;
    let mut nearest = f32::MAX;
    let mut best_hit = 0;
    for enemy in battle.living(unit.team.opponent()) {
        let d = pos.distance(enemy.pos);
        nearest = nearest.min(d);
        let sees = d <= enemy.stats.sight.max(enemy.weapon.max_range)
            && visibility::has_line_of_sight(map, enemy.pos, pos);
        if !sees {
            continue;
        }
        // Defensive value against this enemy.
        let cover = cover_against(map, pos, enemy.pos);
        score += config.cover_weight
            * match cover.level {
                CoverLevel::Full => 30.0,
                CoverLevel::Half => 15.0,
                CoverLevel::None => -25.0,
            };
        // Offensive value: could we shoot them from here?
        if d <= ghost.weapon.max_range && d <= ghost.stats.sight {
            best_hit = best_hit.max(preview_shot(map, &ghost, enemy, false).hit_chance);
        }
    }
    score += config.offense_weight * best_hit as f32;
    if nearest < f32::MAX {
        // Penalise straying far from the optimal range of our weapon.
        score -= config.distance_weight * (nearest - unit.weapon.optimal_range).abs();
    }
    score
}

/// Picks a destination worth moving to, or `None` to stay put.
fn best_destination(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<GridPos> {
    let range = battle.movement_range(unit.id);
    let current = score_tile(battle, unit, unit.pos, config);

    // Prefer tiles that leave one AP for shooting.
    let one_ap = unit.stats.mobility;
    let mut best: Option<(GridPos, f32)> = None;
    let mut any_engagement = false;
    for (pos, cost) in range.iter() {
        if cost > one_ap && unit.action_points > 1 {
            continue;
        }
        let s = score_tile(battle, unit, pos, config);
        any_engagement |= s != 0.0;
        // Tie-break on position for determinism.
        if best.is_none_or(|(bp, bs)| s > bs || (s == bs && pos < bp)) {
            best = Some((pos, s));
        }
    }

    match best {
        Some((pos, s)) if s > current + config.move_margin => Some(pos),
        _ if !any_engagement || battle.visible_enemies(unit.id).is_empty() => {
            // Nothing interesting nearby: advance towards the closest enemy.
            advance_towards_enemy(battle, unit, &range)
        }
        _ => None,
    }
}

/// Moves as far as possible along the way to the nearest enemy, preferring cover.
fn advance_towards_enemy(
    battle: &Battle,
    unit: &Unit,
    range: &crate::pathfinding::ReachableSet,
) -> Option<GridPos> {
    let nearest = battle
        .living(unit.team.opponent())
        .min_by(|a, b| unit.pos.distance(a.pos).total_cmp(&unit.pos.distance(b.pos)))?;
    let map = battle.map();
    range
        .iter()
        .map(|(pos, _)| {
            let cover_bonus = match crate::cover::best_cover_at(map, pos) {
                CoverLevel::Full => 1.5,
                CoverLevel::Half => 0.75,
                CoverLevel::None => 0.0,
            };
            (pos, pos.distance(nearest.pos) - cover_bonus)
        })
        .filter(|(pos, _)| pos.distance(nearest.pos) < unit.pos.distance(nearest.pos))
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(pos, _)| pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario;

    #[test]
    fn ai_actions_are_always_legal_and_battle_terminates() {
        for seed in 0..5 {
            let mut battle = scenario::skirmish(seed);
            let config = AiConfig::default();
            let mut steps = 0;
            while !battle.is_over() && steps < 2000 {
                // Both sides use the AI in this test.
                let action = next_team_action(&battle, &config);
                battle
                    .apply(action.clone())
                    .unwrap_or_else(|e| panic!("AI chose illegal action {action:?}: {e}"));
                steps += 1;
            }
            assert!(battle.is_over(), "seed {seed}: battle did not finish");
        }
    }
}

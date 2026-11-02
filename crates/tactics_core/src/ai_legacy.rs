//! Test-only snapshot of the previous AI, used as a baseline to measure
//! the current one (`ai::behaviour_tests`). Not part of the library.
#![allow(dead_code)]
//! A small utility-based AI that works with **any** abilities.
//!
//! The AI answers one question: *"what should this unit do next?"* It returns
//! a single [`Action`]; the caller applies it, plays the resulting events, and
//! asks again. It never hard-codes ability ids — it looks at each ability's
//! [`AbilityEffect`], so moves added by modules are used automatically.
//!
//! Decision procedure for a unit with action points:
//! 1. **Empty gun?** Use a Reload ability.
//! 2. **Wounded ally in reach?** Use a Heal ability.
//! 3. **Good attack available?** Score every legal attack (any Attack ability,
//!    single-target or area) by expected damage — friendly fire counts against
//!    it — and take the best one if its hit chance clears
//!    [`AiConfig::shoot_threshold`] (area attacks need
//!    [`AiConfig::area_min_score`]).
//! 4. **Reposition.** Score tiles reachable with one AP by cover against
//!    visible enemies, the best shot they'd offer and distance; advance if
//!    nothing is in sight.
//! 5. **Otherwise** take any attack, else Overwatch, else Defend, else wait.
//!
//! The AI is omniscient (knows where every enemy is). For fog of war, filter
//! `battle.living(...)` through `battle.team_can_see`.

use crate::action::{Action, Target};
use crate::battle::Battle;
use crate::combat::preview_attack;
use crate::content::{AbilityDef, AbilityEffect, Targeting};
use crate::cover::cover_against;
use crate::grid::{Climb, CoverLevel, GridPos, Terrain};
use crate::pathfinding::ReachableSet;
use crate::unit::{Unit, UnitId};
use crate::visibility;

/// Tunable knobs. Modules can register named variants as
/// [`AiProfile`](crate::content::AiProfile)s.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AiConfig {
    /// Minimum hit chance (percent) to take a shot instead of repositioning.
    pub shoot_threshold: i32,
    /// Minimum expected-damage score for an area attack to be worth it.
    pub area_min_score: f32,
    /// Weight of friendly-fire damage (subtracted from an attack's score).
    pub friendly_fire_weight: f32,
    /// Heal allies at or below this fraction of max HP.
    pub heal_below: f32,
    /// Score weight of cover at a destination.
    pub cover_weight: f32,
    /// Score weight of the best hit chance a destination offers.
    pub offense_weight: f32,
    /// Penalty per tile of distance from the weapon's optimal range.
    pub distance_weight: f32,
    /// How much better a tile must score before the unit bothers to move.
    pub move_margin: f32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            shoot_threshold: 55,
            area_min_score: 4.5,
            friendly_fire_weight: 2.0,
            heal_below: 0.5,
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

/// The unit's ability definitions, in its own order.
fn abilities<'a>(battle: &'a Battle, unit: &'a Unit) -> impl Iterator<Item = &'a AbilityDef> + 'a {
    unit.abilities.iter().filter_map(|id| battle.ability(id))
}

/// First legal use of an ability matching `pred` with the given target.
fn first_legal(battle: &Battle, unit: &Unit, target: Target, pred: impl Fn(&AbilityEffect) -> bool) -> Option<Action> {
    abilities(battle, unit)
        .filter(|d| pred(&d.effect))
        .map(|d| Action::UseAbility { unit: unit.id, ability: d.id.clone(), target })
        .find(|a| battle.validate(a).is_ok())
}

/// Chooses the next action for a single unit.
pub fn choose_action(battle: &Battle, id: UnitId, config: &AiConfig) -> Action {
    let Some(unit) = battle.unit(id) else {
        return Action::EndTurn;
    };
    if !unit.can_act() {
        return Action::EndTurn;
    }

    // 0. Dormant units hold position or walk their patrol.
    if unit.ai.dormant {
        return patrol_step(battle, unit).map_or(Action::Wait { unit: id }, |to| Action::Move { unit: id, to });
    }

    // 1. Reload when empty.
    if unit.weapon.ammo <= 0
        && let Some(a) = first_legal(battle, unit, Target::None, |e| matches!(e, AbilityEffect::Reload)) {
            return a;
        }

    // 2. Heal the most wounded ally in reach.
    if let Some(a) = best_heal(battle, unit, config) {
        return a;
    }

    // 3. Take a good attack.
    let best = best_attack(battle, unit, config);
    if let Some((action, hit, score, area)) = &best {
        let good = if *area { *score >= config.area_min_score } else { *hit >= config.shoot_threshold };
        if good {
            return action.clone();
        }
    }

    // 4. Reposition.
    if let Some(to) = best_destination(battle, unit, config) {
        return Action::Move { unit: id, to };
    }

    // 5. Fallbacks.
    if let Some((action, _, score, _)) = best
        && score > 0.0 {
            return action;
        }
    let stance = |pred: fn(&AbilityEffect) -> bool| first_legal(battle, unit, Target::None, pred);
    stance(|e| matches!(e, AbilityEffect::Overwatch(_)))
        .or_else(|| stance(|e| matches!(e, AbilityEffect::Defend { .. })))
        .unwrap_or(Action::Wait { unit: id })
}

fn best_heal(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<Action> {
    let mut wounded: Vec<&Unit> = battle
        .living(unit.team)
        .filter(|a| (a.hp as f32) <= a.stats.max_hp as f32 * config.heal_below)
        .collect();
    wounded.sort_by_key(|a| (a.hp, a.id));
    wounded.into_iter().find_map(|ally| {
        first_legal(battle, unit, Target::Unit(ally.id), |e| matches!(e, AbilityEffect::Heal { .. }))
    })
}

/// Best attack as (action, hit chance of main target, score, is_area).
fn best_attack(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<(Action, i32, f32, bool)> {
    let enemies: Vec<&Unit> = battle.living(unit.team.opponent()).collect();
    let mut best: Option<(Action, i32, f32, bool)> = None;

    for def in abilities(battle, unit) {
        let AbilityEffect::Attack(profile) = &def.effect else { continue };
        let area = profile.area_radius > 0.0;
        let targets: Vec<Target> = match def.targeting {
            Targeting::Enemy => enemies.iter().map(|e| Target::Unit(e.id)).collect(),
            Targeting::Tile => enemies.iter().map(|e| Target::Tile(e.pos)).collect(),
            _ => continue,
        };
        for target in targets {
            let action = Action::UseAbility { unit: unit.id, ability: def.id.clone(), target };
            if battle.validate(&action).is_err() {
                continue;
            }
            let Ok(previews) = battle.preview(unit.id, &def.id, target) else { continue };
            let mut score = 0.0;
            let mut main_hit = 0;
            for (victim, p) in &previews {
                let Some(v) = battle.unit(*victim) else { continue };
                let mut value = p.expected_damage();
                if v.team == unit.team {
                    score -= value * config.friendly_fire_weight;
                    continue;
                }
                // Likely kills are worth a lot.
                if p.damage.0 >= v.hp {
                    value += 10.0 * p.hit_chance as f32 / 100.0;
                }
                score += value;
                main_hit = main_hit.max(p.hit_chance);
            }
            // Deterministic tie-break: keep the first best.
            if best.as_ref().is_none_or(|b| score > b.2) {
                best = Some((action, main_hit, score, area));
            }
        }
    }
    best
}

/// Scores standing on `pos` (cover vs. threats + offensive potential + distance).
fn score_tile(battle: &Battle, unit: &Unit, pos: GridPos, config: &AiConfig) -> f32 {
    let map = battle.map();
    let primary = battle.primary_attack(unit).and_then(|d| d.effect.attack_profile());
    let mut ghost = unit.clone();
    ghost.pos = pos;

    let mut score = 0.0;
    let mut nearest = f32::MAX;
    let mut best_hit = 0;
    for enemy in battle.living(unit.team.opponent()) {
        let d = pos.distance(enemy.pos);
        let sees = d <= enemy.stats.sight.max(enemy.weapon.max_range)
            && visibility::has_line_of_sight(map, enemy.pos, pos);
        if !sees {
            continue;
        }
        // Only engaged enemies matter; unseen ones are handled by advancing.
        nearest = nearest.min(d);
        let cover = cover_against(map, pos, enemy.pos);
        score += config.cover_weight
            * match cover.level {
                CoverLevel::Full => 30.0,
                CoverLevel::Half => 15.0,
                CoverLevel::None => -25.0,
            };
        if let Some(profile) = primary
            && d <= ghost.weapon.max_range && d <= ghost.stats.sight {
                let p = preview_attack(&battle.rules().combat, map, pos, &ghost, enemy, profile);
                best_hit = best_hit.max(p.hit_chance);
            }
    }
    score += config.offense_weight * best_hit as f32;
    if nearest < f32::MAX {
        score -= config.distance_weight * (nearest - unit.weapon.optimal_range).abs();
    }
    score
}

/// Picks a destination worth moving to, or `None` to stay put.
fn best_destination(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<GridPos> {
    let range = battle.movement_range(unit.id);
    let current = score_tile(battle, unit, unit.pos, config);

    // Prefer tiles that leave one AP for acting.
    let one_ap = unit.stats.mobility;
    let mut best: Option<(GridPos, f32)> = None;
    let mut any_engagement = false;
    for (pos, cost) in range.iter() {
        if cost > one_ap && unit.action_points > 1 {
            continue;
        }
        let s = score_tile(battle, unit, pos, config);
        any_engagement |= s != 0.0;
        if best.is_none_or(|(bp, bs)| s > bs || (s == bs && pos < bp)) {
            best = Some((pos, s));
        }
    }

    match best {
        Some((pos, s)) if s > current + config.move_margin => Some(pos),
        _ if !any_engagement || battle.visible_enemies(unit.id).is_empty() => match battle.ai_goal(unit.team) {
            // A mission goal (defend/attack a point) wins over hunting.
            Some(goal) if unit.pos.chebyshev(goal) > 1 => advance_towards(battle, unit, &range, goal),
            Some(_) => None,
            None => advance_towards_enemy(battle, unit, &range),
        },
        _ => None,
    }
}

/// Moves as far as possible towards the nearest enemy, preferring cover.
fn advance_towards_enemy(battle: &Battle, unit: &Unit, range: &crate::pathfinding::ReachableSet) -> Option<GridPos> {
    let nearest = battle
        .living(unit.team.opponent())
        .min_by(|a, b| unit.pos.distance(a.pos).total_cmp(&unit.pos.distance(b.pos)))?;
    advance_towards(battle, unit, range, nearest.pos)
}

/// Walking distance (movement points) from every tile to `goal`, treating
/// closed but unlocked doors as open (stepping next to one opens it) and
/// ignoring units. Lets the AI find its way through rooms and corridors.
pub fn distance_field(battle: &Battle, goal: GridPos, climb: Climb) -> ReachableSet {
    let mut map = battle.map().clone();
    for p in battle.props() {
        if p.alive && p.door.is_some_and(|d| !d.open && !d.locked) {
            map.set_terrain(p.pos, Terrain::Floor);
        }
    }
    // The goal itself may be blocked (a beacon prop): flood from it anyway.
    map.set_terrain(goal, Terrain::Floor);
    // Flood from the goal; climbing is treated as symmetric (a close
    // approximation: units mostly climb down what they climbed up).
    crate::pathfinding::reachable(&map, goal, u32::MAX / 4, climb, |_| true)
}

/// Moves as far as possible towards `goal` by walking distance, preferring cover.
fn advance_towards(battle: &Battle, unit: &Unit, range: &ReachableSet, goal: GridPos) -> Option<GridPos> {
    let field = distance_field(battle, goal, Climb { max_up: unit.stats.climb.max(1) });
    let map = battle.map();
    let here = field.cost(unit.pos).map_or(unit.pos.distance(goal) * 2.0, |c| c as f32);
    range
        .iter()
        .map(|(pos, _)| {
            let cover_bonus = match crate::cover::best_cover_at(map, pos) {
                CoverLevel::Full => 3.0,
                CoverLevel::Half => 1.5,
                CoverLevel::None => 0.0,
            };
            let d = field.cost(pos).map_or(pos.distance(goal) * 2.0 + 1000.0, |c| c as f32);
            (pos, d, d - cover_bonus)
        })
        .filter(|(_, d, _)| *d < here)
        .min_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)))
        .map(|(pos, _, _)| pos)
}

/// Next patrol move for a dormant unit: up to one AP towards its waypoint.
fn patrol_step(battle: &Battle, unit: &Unit) -> Option<GridPos> {
    let ai = &unit.ai;
    if ai.patrol.is_empty() {
        return None;
    }
    let mut index = ai.patrol_index % ai.patrol.len();
    if unit.pos.chebyshev(ai.patrol[index]) <= 1 {
        index = (index + 1) % ai.patrol.len();
    }
    let goal = ai.patrol[index];
    let one_ap = unit.stats.mobility;
    let range = battle.movement_range(unit.id);
    let field = distance_field(battle, goal, Climb { max_up: unit.stats.climb.max(1) });
    let here = field.cost(unit.pos)?;
    range
        .iter()
        .filter(|(_, cost)| *cost <= one_ap)
        .filter_map(|(pos, _)| field.cost(pos).map(|d| (pos, d)))
        .filter(|(_, d)| *d < here)
        .min_by_key(|(pos, d)| (*d, *pos))
        .map(|(pos, _)| pos)
}


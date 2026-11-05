//! A utility-based AI that works with **any** abilities and reacts to fear.
//!
//! The AI answers one question: *"what should this unit do next?"* It returns
//! a single [`Action`]; the caller applies it, plays the resulting events, and
//! asks again. It never hard-codes ability ids — it looks at each ability's
//! [`AbilityEffect`], so moves added by modules are used automatically.
//!
//! ## Mood: fear changes behaviour
//! Every decision starts from the unit's [`Mood`] (see [`mood`]):
//!
//! | mood | when | behaviour ([`adapted`]) |
//! |---|---|---|
//! | **Confident** | calm, healthy, and its side outnumbers the enemies in sight | takes riskier shots, hunts flanks, worries less about exposure |
//! | **Steady** | default | balanced |
//! | **Shaken** | fear ≥ `shaken_at` | wants better odds, values cover and safety, falls back when it could lose half its HP where it stands |
//! | **Panicked** | fear ≥ `panicked_at` | runs for the safest tile in reach, else hunkers down (which also calms it) |
//!
//! When the team as a whole is frightened its units **huddle** (allies nearby
//! are a plus instead of a grenade risk; nearby allies also cut fear from
//! being outnumbered). Fearless units (drones, robots) are always Steady or
//! Confident. [`AiConfig::fear_response`] scales all of this (0 = ignore fear).
//!
//! ## Decision procedure
//! 0. **Dormant?** Hold or walk the patrol route.
//! 1. **Empty gun?** Reload.
//! 2. **Panicked?** Flee to safety, else hunker.
//! 3. **Wounded ally in reach?** Heal (the most hurt first).
//! 4. **Good attack?** Every legal attack is scored by expected damage, plus
//!    likely kills, the target's own threat and how hurt it already is (focus
//!    fire); friendly fire counts against it. Taken if the hit chance clears
//!    the mood-adjusted [`AiConfig::shoot_threshold`] (area attacks need
//!    [`AiConfig::area_min_score`]).
//! 5. **Reposition.** Score tiles in reach by: cover against enemies that can
//!    see them, **danger** (the damage every enemy who can see the tile would
//!    expect to deal there), the best shot they'd offer, **flanking** an enemy
//!    that's in cover, spacing from allies (grenades) and range. With no enemy
//!    in sight: follow the mission's AI goal, else advance by real path.
//! 6. **Otherwise** take any worthwhile attack, else Overwatch, else Defend,
//!    else top up the magazine, else wait.
//!
//! The AI is omniscient (knows where every enemy is). For fog of war, filter
//! `battle.living(...)` through `battle.team_can_see`.

use crate::action::{Action, Target};
use crate::battle::Battle;
use crate::combat::preview_attack;
use crate::content::{AbilityDef, AbilityEffect, FearLevel, Targeting};
use crate::cover::{best_cover_at, cover_against};
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
    /// Penalty per point of damage enemies could expect to deal at a tile.
    pub danger_weight: f32,
    /// Bonus for a tile that flanks an enemy in cover.
    pub flank_bonus: f32,
    /// Penalty per ally right next to a tile (one grenade, two victims).
    pub cluster_penalty: f32,
    /// Attack bonus for finishing off wounded targets (× missing HP fraction).
    pub focus_fire_weight: f32,
    /// Attack bonus per point of the target's own average damage.
    pub threat_weight: f32,
    /// At or below this HP fraction a unit weighs danger 50% more.
    pub wounded_below: f32,
    /// How strongly fear changes behaviour (0 = ignore fear, 1 = normal).
    pub fear_response: f32,
    /// Fear under which a healthy unit on the winning side is Confident.
    pub confident_below: i32,
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
            danger_weight: 1.0,
            flank_bonus: 15.0,
            cluster_penalty: 4.0,
            focus_fire_weight: 3.0,
            threat_weight: 0.5,
            wounded_below: 0.34,
            fear_response: 1.0,
            confident_below: 15,
        }
    }
}

/// A unit's state of mind, from its fear and the situation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mood {
    Confident,
    Steady,
    Shaken,
    Panicked,
}

/// Works out a unit's [`Mood`].
pub fn mood(battle: &Battle, unit: &Unit, config: &AiConfig) -> Mood {
    if config.fear_response <= 0.0 {
        return Mood::Steady;
    }
    let morale = &battle.rules().combat.morale;
    let level = if morale.enabled { morale.level(unit.fear) } else { FearLevel::Steady };
    match level {
        FearLevel::Panicked => Mood::Panicked,
        FearLevel::Shaken => Mood::Shaken,
        FearLevel::Steady => {
            let healthy = (unit.hp as f32) > unit.stats.max_hp as f32 * 0.6;
            let foes = battle.living(unit.team.opponent()).filter(|e| battle.team_can_see(unit.team, e.pos)).count();
            let friends = battle.living(unit.team).count();
            if unit.fear < config.confident_below && healthy && foes > 0 && friends > foes {
                Mood::Confident
            } else {
                Mood::Steady
            }
        }
    }
}

/// The config a unit in `mood` actually uses.
pub fn adapted(config: &AiConfig, mood: Mood) -> AiConfig {
    let r = config.fear_response.max(0.0);
    let mut c = *config;
    match mood {
        Mood::Steady => {}
        Mood::Confident => {
            c.shoot_threshold -= (10.0 * r) as i32;
            c.offense_weight *= 1.0 + 0.3 * r;
            c.danger_weight *= (1.0 - 0.3 * r).max(0.1);
            c.flank_bonus *= 1.0 + 0.5 * r;
            c.move_margin *= (1.0 - 0.2 * r).max(0.1);
        }
        Mood::Shaken | Mood::Panicked => {
            let k = if mood == Mood::Panicked { 2.0 } else { 1.0 } * r;
            c.shoot_threshold += (8.0 * k) as i32;
            c.danger_weight *= 1.0 + 0.7 * k;
            c.cover_weight *= 1.0 + 0.5 * k;
            c.offense_weight *= (1.0 - 0.3 * k).max(0.1);
            c.flank_bonus *= (1.0 - 0.5 * k).max(0.0);
        }
    }
    c
}

/// Is `unit`'s team frightened as a whole (average fear of units that can
/// feel it at or above the shaken threshold)?
fn team_frightened(battle: &Battle, unit: &Unit) -> bool {
    let m = &battle.rules().combat.morale;
    let (sum, n) = battle.living(unit.team).filter(|u| !u.fearless).fold((0, 0), |(s, n), u| (s + u.fear, n + 1));
    m.enabled && n > 0 && sum / n >= m.shaken_at
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
    let action = choose_action_inner(battle, id, config);
    // Machines never cook themselves: anything that would overheat is
    // swapped for venting or holding still.
    if battle.overheats(&action)
        && let Some(unit) = battle.unit(id)
    {
        return heat_relief(battle, unit).unwrap_or(Action::Wait { unit: id });
    }
    action
}

/// For a hot machine: a coolant flush, or a shutdown if no enemy can see it
/// (being hit while shut down is fatal).
fn heat_relief(battle: &Battle, unit: &Unit) -> Option<Action> {
    let cool = first_legal(battle, unit, Target::None, |e| matches!(e, AbilityEffect::Cool { .. }));
    cool.or_else(|| {
        let watched = battle.living(unit.team.opponent()).any(|e| battle.can_unit_see(e, unit.pos));
        (!watched).then(|| first_legal(battle, unit, Target::None, |e| matches!(e, AbilityEffect::Shutdown { .. }))).flatten()
    })
}

fn choose_action_inner(battle: &Battle, id: UnitId, config: &AiConfig) -> Action {
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

    // Hordes (zombies) don't do tactics: close in and strike.
    if unit.ai.horde {
        return horde_action(battle, unit);
    }

    let mood = mood(battle, unit, config);
    let mut config = adapted(config, mood);
    // Self-preservation grows as HP drops (robots too: it's tactics, not fear).
    let hp_frac = unit.hp as f32 / unit.stats.max_hp.max(1) as f32;
    if hp_frac <= config.wounded_below {
        config.danger_weight *= 1.5;
    }
    if team_frightened(battle, unit) {
        // Frightened teams stick together.
        config.cluster_penalty = -config.cluster_penalty.abs() * 0.5;
    }
    let config = &config;
    let stance = |pred: fn(&AbilityEffect) -> bool| first_legal(battle, unit, Target::None, pred);

    // 0b. Running hot (machines): vent before it gets critical.
    if unit.heat.is_some()
        && unit.heat() * 10 >= unit.stats.max_hp * 6
        && let Some(a) = heat_relief(battle, unit)
    {
        return a;
    }

    // 1. Reload when empty.
    if unit.weapon.ammo <= 0
        && let Some(a) = stance(|e| matches!(e, AbilityEffect::Reload))
    {
        return a;
    }

    // 2. Panic: run for safety, else hunker down.
    if mood == Mood::Panicked {
        if let Some(to) = retreat_destination(battle, unit, config) {
            return Action::Move { unit: id, to };
        }
        return stance(|e| matches!(e, AbilityEffect::Defend { .. })).unwrap_or(Action::Wait { unit: id });
    }

    // 3. Heal the most wounded ally in reach.
    if let Some(a) = best_heal(battle, unit, config) {
        return a;
    }

    // 4. Take a good attack — unless a step to a better spot first is
    //    clearly worth more (flank, clear line, less exposure).
    let best = best_attack(battle, unit, config);
    let shoot_now = best.as_ref().map_or(0.0, |b| if b.3 { 0.0 } else { b.2 });
    if let Some(to) = move_then_shoot(battle, unit, config, shoot_now) {
        return Action::Move { unit: id, to };
    }
    if let Some((action, hit, score, area)) = &best {
        let good = if *area { *score >= config.area_min_score } else { *hit >= config.shoot_threshold };
        if good {
            return action.clone();
        }
    }

    // 5. Reposition. A shaken unit that could lose half its HP where it
    //    stands backs off to safety first.
    if mood == Mood::Shaken
        && eval_tile(battle, unit, unit.pos, config).danger * 2.0 >= unit.hp as f32
        && let Some(to) = retreat_destination(battle, unit, config)
    {
        return Action::Move { unit: id, to };
    }
    if let Some(to) = best_destination(battle, unit, config) {
        return Action::Move { unit: id, to };
    }

    // 6. Fallbacks. Shaken units only take long shots if they're worth a lot.
    let min_score = if mood == Mood::Shaken { 3.0 } else { 0.0 };
    if let Some((action, _, score, _)) = best
        && score > min_score
    {
        return action;
    }
    let sees_enemy = !battle.visible_enemies(id).is_empty();
    // Machine gunners pin down the most dangerous enemy they can see when
    // there's no good shot (and no friend in the way).
    if sees_enemy
        && let Some(a) = suppress_action(battle, unit, config)
    {
        return a;
    }
    if sees_enemy {
        if let Some(a) = stance(|e| matches!(e, AbilityEffect::Overwatch(_))) {
            return a;
        }
        if let Some(a) = stance(|e| matches!(e, AbilityEffect::Defend { .. })) {
            return a;
        }
    }
    // Quiet moment: top up the magazine.
    if unit.weapon.ammo < unit.weapon.clip_size
        && let Some(a) = stance(|e| matches!(e, AbilityEffect::Reload))
    {
        return a;
    }
    stance(|e| matches!(e, AbilityEffect::Overwatch(_)))
        .or_else(|| stance(|e| matches!(e, AbilityEffect::Defend { .. })))
        .unwrap_or(Action::Wait { unit: id })
}

/// Suppressive fire on the most dangerous visible, unsuppressed enemy.
fn suppress_action(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<Action> {
    let _ = config;
    let def = abilities(battle, unit).find(|d| matches!(d.effect, AbilityEffect::Suppress(_)))?;
    let mut targets: Vec<&Unit> = battle
        .visible_enemies(unit.id)
        .into_iter()
        .filter_map(|e| battle.unit(e))
        .filter(|e| e.status.suppressed_by.is_none())
        .filter(|e| {
            let (from, to) = battle.shot_line(unit.pos, e.pos);
            battle.units_in_line_of_fire(from, to, unit.team).iter().all(|u| *u == unit.id)
        })
        .collect();
    targets.sort_by(|a, b| threat_of(battle, b).total_cmp(&threat_of(battle, a)).then(a.id.cmp(&b.id)));
    targets
        .into_iter()
        .map(|t| Action::UseAbility { unit: unit.id, ability: def.id.clone(), target: Target::Unit(t.id) })
        .find(|a| battle.validate(a).is_ok())
}

fn best_heal(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<Action> {
    let mut wounded: Vec<&Unit> = battle
        .living(unit.team)
        .filter(|a| a.heat.is_none() && (a.hp as f32) <= a.stats.max_hp as f32 * config.heal_below)
        .collect();
    wounded.sort_by_key(|a| (a.hp, a.id));
    wounded.into_iter().find_map(|ally| {
        first_legal(battle, unit, Target::Unit(ally.id), |e| matches!(e, AbilityEffect::Heal { .. }))
    })
}

/// Average damage of a unit's primary attack (how dangerous it is).
fn threat_of(battle: &Battle, unit: &Unit) -> f32 {
    battle
        .primary_attack(unit)
        .and_then(|d| d.effect.attack_profile())
        .map_or(0.0, |p| match p.damage {
            crate::content::DamageSource::Weapon => (unit.weapon.damage_min + unit.weapon.damage_max) as f32 / 2.0,
            crate::content::DamageSource::Fixed { min, max } => (min + max) as f32 / 2.0,
        } + p.damage_bonus as f32)
}

/// Value of hitting `v` with preview `p`: expected damage, likely kills,
/// focus fire on the wounded and priority on dangerous targets.
fn target_value(battle: &Battle, v: &Unit, p: &crate::combat::ShotPreview, config: &AiConfig) -> f32 {
    let p_hit = p.hit_chance as f32 / 100.0;
    let mut value = p.expected_damage();
    // A shut-down machine dies to any hit.
    let hp = if v.is_shut_down() { 1 } else { v.hp };
    // Likely kills are worth a lot (the whole volley's expected damage).
    value += 10.0 * (value / hp.max(1) as f32).min(1.0) * if value >= hp as f32 { 1.0 } else { 0.5 };
    let missing = 1.0 - hp as f32 / v.stats.max_hp.max(1) as f32;
    value + p_hit * (config.focus_fire_weight * missing + config.threat_weight * threat_of(battle, v))
}

/// Best value of the unit's primary (single-target) attack if it stood on `pos`.
fn attack_value_from(battle: &Battle, unit: &Unit, pos: GridPos, config: &AiConfig) -> f32 {
    let Some(def) = battle.primary_attack(unit) else { return 0.0 };
    let Some(profile) = def.effect.attack_profile() else { return 0.0 };
    if unit.weapon.ammo < def.ammo_cost {
        return 0.0;
    }
    let mut ghost = unit.clone();
    ghost.pos = pos;
    let range = battle.ability_range(unit, def);
    let map = battle.map();
    battle
        .living(unit.team.opponent())
        .filter(|e| pos.distance(e.pos) <= range && visibility::has_sight(map, pos, e.pos))
        .map(|e| {
            let from = battle.shot_line(pos, e.pos).0;
            let p = preview_attack(&battle.rules().combat, map, from, &ghost, e, profile);
            target_value(battle, e, &p, config)
        })
        .fold(0.0, f32::max)
}

/// Expected reaction-fire damage from enemies on overwatch while walking
/// to `pos` (any step they can see triggers one shot each).
fn overwatch_risk(battle: &Battle, unit: &Unit, range: &ReachableSet, pos: GridPos) -> f32 {
    let map = battle.map();
    let watchers: Vec<&Unit> = battle
        .living(unit.team.opponent())
        .filter(|w| w.status.overwatch.is_some() && w.weapon.ammo > 0)
        .collect();
    if watchers.is_empty() {
        return 0.0;
    }
    let path = range.path_to(pos).unwrap_or_default();
    let mut ghost = unit.clone();
    watchers
        .iter()
        .filter_map(|w| {
            let def = battle.ability(w.status.overwatch.as_ref()?)?;
            let profile = def.effect.attack_profile()?;
            let reach = battle.ability_range(w, def).min(w.stats.sight);
            let step = path.iter().find(|s| w.pos.distance(**s) <= reach && visibility::has_sight(map, w.pos, **s))?;
            ghost.pos = *step;
            Some(preview_attack(&battle.rules().combat, map, w.pos, w, &ghost, profile).expected_damage())
        })
        .sum()
}

/// "Step to a better spot, then shoot" vs "shoot from here": the tile (one
/// AP away) whose shot value minus the danger of standing there beats the
/// current spot by more than `move_margin` / 4. Needs 2+ AP.
fn move_then_shoot(battle: &Battle, unit: &Unit, config: &AiConfig, shoot_now: f32) -> Option<GridPos> {
    if unit.action_points < 2 || battle.primary_attack(unit).is_none() {
        return None;
    }
    // Damage dealt counts more than damage risked: the risk is only a chance.
    let lambda = config.danger_weight / 6.0;
    let here = shoot_now - lambda * eval_tile(battle, unit, unit.pos, config).danger;
    let one_ap = unit.stats.mobility;
    let range = battle.movement_range(unit.id);
    range
        .iter()
        .filter(|(_, cost)| *cost <= one_ap)
        .filter_map(|(pos, _)| {
            let shot = attack_value_from(battle, unit, pos, config);
            (shot > 0.0).then(|| {
                let e = eval_tile(battle, unit, pos, config);
                let shot = shot - lambda * overwatch_risk(battle, unit, &range, pos) * 1.5;
                let spacing = battle.living(unit.team).filter(|a| a.id != unit.id && a.pos.distance(pos) < 1.6).count() as f32;
                (pos, shot - lambda * e.danger - 0.1 * config.cluster_penalty * spacing)
            })
        })
        .filter(|(_, v)| *v > here + config.move_margin / 4.0)
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(pos, _)| pos)
}

/// Best attack as (action, hit chance of main target, score, is_area).
fn best_attack(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<(Action, i32, f32, bool)> {
    let enemies: Vec<&Unit> = battle.living(unit.team.opponent()).collect();
    let mut best: Option<(Action, i32, f32, bool)> = None;

    for def in abilities(battle, unit) {
        let AbilityEffect::Attack(profile) = &def.effect else { continue };
        let area = profile.is_area();
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
            // Bullets fly on: don't shoot through your own people.
            if profile.is_ballistic()
                && let Target::Unit(t) = target
                && let Some(tp) = battle.unit(t).map(|t| t.pos)
            {
                let (from, to) = battle.shot_line(unit.pos, tp);
                let allies = battle.units_in_line_of_fire(from, to, unit.team).iter().filter(|u| **u != unit.id).count() as f32;
                score -= allies * config.friendly_fire_weight * 3.0;
            }
            let mut main_hit = 0;
            for (victim, p) in &previews {
                let Some(v) = battle.unit(*victim) else { continue };
                if v.team == unit.team {
                    score -= p.expected_damage() * config.friendly_fire_weight;
                    continue;
                }
                score += target_value(battle, v, p, config);
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

/// What standing on a tile would be like.
struct TileEval {
    /// Total desirability (higher is better).
    score: f32,
    /// Damage the enemies who can see the tile would expect to deal there.
    danger: f32,
    /// Some enemy can see the tile.
    exposed: bool,
    /// Distance to the nearest enemy that can see the tile.
    nearest: f32,
}

/// Scores standing on `pos`: cover and danger vs. enemies that see it,
/// offensive potential, flanking, spacing and range.
fn eval_tile(battle: &Battle, unit: &Unit, pos: GridPos, config: &AiConfig) -> TileEval {
    let map = battle.map();
    let rules = &battle.rules().combat;
    let primary = battle.primary_attack(unit).and_then(|d| d.effect.attack_profile());
    let mut ghost = unit.clone();
    ghost.pos = pos;

    let mut score = 0.0;
    let mut danger = 0.0;
    let mut nearest = f32::MAX;
    let mut best_hit = 0;
    let mut flanks = false;
    for enemy in battle.living(unit.team.opponent()) {
        let d = pos.distance(enemy.pos);
        let sees = d <= enemy.stats.sight.max(enemy.weapon.max_range) && visibility::has_sight(map, enemy.pos, pos);
        if !sees {
            continue;
        }
        // Only engaged enemies matter; unseen ones are handled by advancing.
        nearest = nearest.min(d);
        let cover = cover_against(map, pos, enemy.pos);
        score += config.cover_weight
            * match cover.level {
                CoverLevel::Full => 30.0,
                CoverLevel::Half if unit.no_half_cover => -25.0,
                CoverLevel::Half => 15.0,
                CoverLevel::None => -25.0,
            };
        // Danger: what this enemy's main attack would expect to do here.
        // Dormant enemies aren't shooting yet.
        if !enemy.ai.dormant
            && let Some(theirs) = battle.primary_attack(enemy).and_then(|a| a.effect.attack_profile())
            && d <= enemy.weapon.max_range
        {
            danger += preview_attack(rules, map, enemy.pos, enemy, &ghost, theirs).expected_damage();
        }
        if let Some(profile) = primary
            && d <= ghost.weapon.max_range
            && d <= ghost.stats.sight
        {
            let p = preview_attack(rules, map, pos, &ghost, enemy, profile);
            best_hit = best_hit.max(p.hit_chance);
            // Flanking an enemy who is relying on cover.
            if p.cover.flanked && best_cover_at(map, enemy.pos) != CoverLevel::None {
                flanks = true;
            }
        }
    }
    let exposed = nearest < f32::MAX;
    score += config.offense_weight * best_hit as f32;
    score -= config.danger_weight * danger;
    if flanks {
        score += config.flank_bonus;
    }
    if exposed {
        score -= config.distance_weight * (nearest - unit.weapon.optimal_range).abs();
        let huddle = battle.living(unit.team).filter(|a| a.id != unit.id && a.pos.distance(pos) < 1.6).count();
        score -= config.cluster_penalty * huddle as f32;
    }
    // Armed charges (particle grenades) about to go off.
    if battle.armed_charges().iter().any(|c| battle.charge_tiles(c).contains(&pos)) {
        danger += 5.0;
        score -= 80.0;
    }
    // Fire: burns at the start of every turn spent there.
    if battle.on_fire(pos) {
        danger += rules.fire_damage as f32 * 2.0;
        score -= 60.0;
    }
    TileEval { score, danger, exposed, nearest }
}

/// Picks a destination worth moving to, or `None` to stay put.
fn best_destination(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<GridPos> {
    let range = battle.movement_range(unit.id);
    let current = eval_tile(battle, unit, unit.pos, config).score;

    // Prefer tiles that leave one AP for acting.
    let one_ap = unit.stats.mobility;
    let mut best: Option<(GridPos, f32)> = None;
    let mut any_engagement = false;
    for (pos, cost) in range.iter() {
        if cost > one_ap && unit.action_points > 1 {
            continue;
        }
        let e = eval_tile(battle, unit, pos, config);
        any_engagement |= e.exposed;
        let score = e.score - config.danger_weight * 1.5 * overwatch_risk(battle, unit, &range, pos);
        if best.is_none_or(|(bp, bs)| score > bs || (score == bs && pos < bp)) {
            best = Some((pos, score));
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

/// Safest tile in reach for a frightened unit: least danger, out of sight
/// if possible, away from enemies, in cover. `None` if staying is as good.
fn retreat_destination(battle: &Battle, unit: &Unit, config: &AiConfig) -> Option<GridPos> {
    let safety = |pos: GridPos| {
        let e = eval_tile(battle, unit, pos, config);
        let cover = match best_cover_at(battle.map(), pos) {
            CoverLevel::Full => 6.0,
            CoverLevel::Half => 3.0,
            CoverLevel::None => 0.0,
        };
        let hidden = if e.exposed { 0.0 } else { 25.0 };
        let away = if e.exposed { e.nearest.min(20.0) } else { 20.0 };
        -e.danger * config.danger_weight + hidden + away + cover
    };
    let here = safety(unit.pos);
    let range = battle.movement_range(unit.id);
    range
        .iter()
        .map(|(pos, cost)| (pos, safety(pos) - cost as f32 * 0.05))
        .filter(|(_, s)| *s > here + 4.0)
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(pos, _)| pos)
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

/// Horde brain: strike the weakest enemy in reach, else shamble towards the
/// nearest one. Costs one small path search, so hundreds of zombies stay cheap.
fn horde_action(battle: &Battle, unit: &Unit) -> Action {
    let id = unit.id;
    let mut foes: Vec<&Unit> = battle.living(unit.team.opponent()).collect();
    foes.sort_by(|a, b| unit.pos.distance(a.pos).total_cmp(&unit.pos.distance(b.pos)).then(a.id.cmp(&b.id)));

    // 1. Strike: the weakest enemy any attack can legally reach.
    let mut strikes: Vec<(i32, Action)> = Vec::new();
    for def in abilities(battle, unit).filter(|d| matches!(d.effect, AbilityEffect::Attack(_))) {
        for foe in foes.iter().take_while(|f| unit.pos.distance(f.pos) <= 25.0).take(6) {
            let a = Action::UseAbility { unit: id, ability: def.id.clone(), target: Target::Unit(foe.id) };
            if battle.validate(&a).is_ok() {
                strikes.push((foe.hp, a));
            }
        }
    }
    if let Some((_, a)) = strikes.into_iter().min_by_key(|(hp, _)| *hp) {
        return a;
    }

    // 2. Close in on the nearest enemy.
    let Some(prey) = foes.first() else { return Action::Wait { unit: id } };
    let goal = prey.pos;
    if unit.pos.chebyshev(goal) <= 1 {
        return Action::Wait { unit: id }; // in reach but can't strike (no AP)
    }
    let range = battle.movement_range(id);
    let one_ap = unit.stats.mobility;
    let d = unit.pos.distance(goal);
    // Walking distance to the prey, searched only around the chase (a
    // bounded flood from the prey), so it stays cheap on huge maps.
    let field = (d <= 40.0).then(|| {
        let budget = ((d * 1.6 + 12.0) * crate::pathfinding::DIAGONAL_COST as f32) as u32;
        crate::pathfinding::reachable(battle.map(), goal, budget, Climb { max_up: unit.stats.climb.max(1) }, |p| battle.map().is_walkable(p))
    });
    let score = |p: GridPos| -> f32 {
        match field.as_ref().and_then(|f| f.cost(p)) {
            Some(c) => c as f32,
            None => 10_000.0 + p.distance(goal) * 10.0,
        }
    };
    let here = score(unit.pos);
    // Prefer a tile next to the prey within one AP (so it can still bite),
    // else the closest it can get with all its AP.
    let next_to = range
        .iter()
        .filter(|(p, c)| *c <= one_ap && p.chebyshev(goal) <= 1)
        .min_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    if let Some((p, _)) = next_to {
        return Action::Move { unit: id, to: p };
    }
    range
        .iter()
        .filter(|(p, _)| *p != unit.pos)
        .map(|(p, c)| (p, score(p), c))
        .filter(|(_, s, _)| *s < here)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)).then(a.0.cmp(&b.0)))
        .map_or(Action::Wait { unit: id }, |(p, _, _)| Action::Move { unit: id, to: p })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Ruleset;
    use crate::modules::GrenadierModule;
    use crate::scenario;

    #[test]
    fn ai_actions_are_always_legal_and_battles_finish() {
        let mut rules = Ruleset::standard();
        rules.add_module(&GrenadierModule);
        for seed in 0..6 {
            let mut battle = scenario::skirmish(&rules, seed);
            let config = AiConfig::default();
            let mut steps = 0;
            while !battle.is_over() && steps < 3000 {
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

#[cfg(test)]
mod behaviour_tests {
    use super::*;
    use crate::content::Ruleset;
    use crate::grid::Map;
    use crate::modules::GrenadierModule;
    use crate::scenario;
    use crate::unit::Team;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    /// Who decides for one side.
    #[derive(Clone, Copy)]
    pub enum Brain {
        Current(AiConfig),
        /// The AI as of the previous commit (`ai_legacy.rs`).
        Previous,
    }

    impl Brain {
        fn act(&self, b: &crate::battle::Battle) -> Action {
            match self {
                Brain::Current(c) => next_team_action(b, c),
                Brain::Previous => crate::ai_legacy::next_team_action(b, &crate::ai_legacy::AiConfig::default()),
            }
        }
    }

    /// Mirror matches: the skirmish map, the same four soldiers on both
    /// sides. Returns (games won by `a`, games) over both side assignments.
    fn duel(a: Brain, b: Brain, seeds: std::ops::Range<u64>) -> (u32, u32) {
        let rules = rules();
        let classes = ["rifleman", "assault", "sniper", "grenadier"];
        let (mut wins, mut games) = (0, 0);
        for seed in seeds {
            let base = scenario::skirmish(&rules, seed);
            for a_is_player in [true, false] {
                let mut bt = crate::battle::Battle::new(base.map().clone(), rules.clone(), seed);
                for team in [Team::Player, Team::Enemy] {
                    let tiles: Vec<GridPos> = base.living(team).map(|u| u.pos).collect();
                    for (i, class) in classes.iter().enumerate() {
                        if let Some(p) = bt.free_tile_near(tiles[i % tiles.len()]) {
                            bt.spawn(class, format!("{class}{i}"), team, p).unwrap();
                        }
                    }
                }
                for _ in 0..4000 {
                    if bt.is_over() || bt.round() > 30 {
                        break;
                    }
                    let mine = (bt.active_team() == Team::Player) == a_is_player;
                    let act = if mine { a.act(&bt) } else { b.act(&bt) };
                    bt.apply(act.clone()).unwrap_or_else(|e| panic!("illegal {act:?}: {e}"));
                }
                let a_team = if a_is_player { Team::Player } else { Team::Enemy };
                games += 1;
                wins += (bt.winner() == Some(a_team)) as u32;
            }
        }
        (wins, games)
    }

    pub fn duel_pub(a: Brain, b: Brain, seeds: std::ops::Range<u64>) -> (u32, u32) {
        duel(a, b, seeds)
    }

    #[test]
    fn new_ai_beats_the_old_one() {
        // Deterministic: fixed seeds. Measured 69/120 (57.5%) when tuned; the
        // previous AI scores 60/120 against itself.
        let (wins, games) = duel(Brain::Current(AiConfig::default()), Brain::Previous, 0..60);
        eprintln!("new AI won {wins}/{games} mirror matches against the previous AI");
        assert!(wins * 100 >= games * 55, "the improved AI should win most mirror matches ({wins}/{games})");
    }

    #[test]
    fn panicked_units_flee_or_hunker_and_robots_never_panic() {
        let mut b = crate::battle::Battle::new(
            Map::from_ascii(
                "..............\n\
                 ......H.......\n\
                 ..............",
            ),
            rules(),
            1,
        );
        let e = b.spawn("sectoid", "Scared", Team::Enemy, GridPos::new(7, 0)).unwrap();
        b.spawn("rifleman", "A", Team::Player, GridPos::new(2, 0)).unwrap();
        b.spawn("rifleman", "B", Team::Player, GridPos::new(2, 2)).unwrap();
        b.debug_set_fear(e, 90);
        let unit = b.unit(e).unwrap();
        assert_eq!(mood(&b, unit, &AiConfig::default()), Mood::Panicked);
        b.apply(crate::action::Action::EndTurn).unwrap();
        let unit = b.unit(e).unwrap().clone();
        let action = choose_action(&b, e, &AiConfig::default());
        match action {
            Action::Move { to, .. } => {
                let before = b.living(Team::Player).map(|p| p.pos.distance(unit.pos)).fold(f32::MAX, f32::min);
                let after = b.living(Team::Player).map(|p| p.pos.distance(to)).fold(f32::MAX, f32::min);
                assert!(after > before || !b.team_can_see(Team::Player, to), "fled towards danger: {to}");
            }
            Action::UseAbility { ability, .. } => assert_eq!(ability.as_str(), "hunker"),
            other => panic!("panicked unit did {other:?}"),
        }
        // Ignoring fear: the same unit fights normally.
        let calm = AiConfig { fear_response: 0.0, ..AiConfig::default() };
        assert_eq!(mood(&b, &unit, &calm), Mood::Steady);
    }

    #[test]
    fn confident_when_winning_cautious_when_wounded() {
        let mut b = crate::battle::Battle::new(Map::from_ascii("............"), rules(), 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        b.spawn("rifleman", "B", Team::Player, GridPos::new(1, 0)).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(8, 0)).unwrap();
        let c = AiConfig::default();
        assert_eq!(mood(&b, b.unit(a).unwrap(), &c), Mood::Confident);
        assert!(adapted(&c, Mood::Confident).shoot_threshold < c.shoot_threshold);
        assert!(adapted(&c, Mood::Shaken).shoot_threshold > c.shoot_threshold);
        // Badly hurt: no longer confident (it also weighs danger more).
        b.debug_set_hp(a, 2);
        assert_eq!(mood(&b, b.unit(a).unwrap(), &c), Mood::Steady);
        // Frightened: shaken.
        b.debug_set_hp(a, 8);
        b.debug_set_fear(a, 50);
        assert_eq!(mood(&b, b.unit(a).unwrap(), &c), Mood::Shaken);
    }
}

#[cfg(test)]
mod ablation {
    use super::*;
    #[test]
    #[ignore]
    /// Tuning tool: `cargo test --release -p tactics_core --all-features
    /// ablation -- --ignored --nocapture` plays variants against the previous AI.
    fn ablation() {
        let d = AiConfig::default();
        let variants: Vec<(&str, AiConfig)> = vec![
            ("d1+all", AiConfig { danger_weight: 1.0, ..d }),
            ("d1.5+all", AiConfig { danger_weight: 1.5, ..d }),
            ("d1+all-nofear", AiConfig { danger_weight: 1.0, fear_response: 0.0, ..d }),
            ("d1+all-noflank", AiConfig { danger_weight: 1.0, flank_bonus: 0.0, ..d }),
            ("d1+all+cl8", AiConfig { danger_weight: 1.0, cluster_penalty: 8.0, ..d }),
        ];
        use super::behaviour_tests::Brain;
        let (w, g) = super::behaviour_tests::duel_pub(Brain::Previous, Brain::Previous, 0..60);
        eprintln!("baseline   {w}/{g}");
        for (name, v) in variants {
            let (w, g) = super::behaviour_tests::duel_pub(Brain::Current(v), Brain::Previous, 0..60);
            eprintln!("{name:10} {w}/{g}");
        }
    }
}

//! Immediate-mode overlays drawn with gizmos: movement range, path preview,
//! hover cursor, cover shields, targeting lines, health bars and tracers.
//!
//! Gizmos are redrawn every frame from the current state, so there is no
//! overlay state to keep in sync. They work identically on every backend
//! (Vulkan, DX12, Metal, WebGL2/WebGPU, GLES).

use bevy::prelude::*;
use tactics_core::cover::cover_around;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::{TILE_SIZE, tile_top};
use crate::input::player_can_act;
use crate::playback::{Anim, Playback};
use crate::state::{BattleState, Selection};
use crate::theme;
use crate::units::{UnitEntities, UnitVisual};

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                (draw_movement, draw_targets, draw_targeting).run_if(player_can_act),
                draw_hover,
                draw_armed_charges,
                draw_health_bars,
                draw_zones,
                draw_tracer,
            )
                .in_set(FrameSet::Present),
        );
    }
}

const LIFT: f32 = 0.02;

/// A flat square outline on a tile.
fn tile_rect(gizmos: &mut Gizmos, map: &Map, pos: GridPos, scale: f32, color: Color) {
    let iso = Isometry3d::new(
        tile_top(map, pos) + Vec3::Y * LIFT,
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    );
    gizmos.rect(iso, Vec2::splat(TILE_SIZE * scale), color);
}

fn draw_movement(
    mut gizmos: Gizmos,
    time: Res<Time<Real>>,
    state: Res<BattleState>,
    selection: Res<Selection>,
    touch: Res<crate::touch::TouchMode>,
) {
    if selection.pending.is_some() {
        return; // targeting mode replaces the movement overlay
    }
    // On a touchscreen, moving starts with the Move button: until then only
    // the selected soldier is marked.
    let show_moves = !touch.active || selection.move_mode;
    let (Some(id), Some(range)) = (selection.unit, selection.range.as_ref()) else { return };
    let battle = &state.battle;
    let Some(unit) = battle.unit(id) else { return };

    // Blue = move and still act; yellow = dash (uses every AP).
    for (pos, cost) in range.iter().filter(|_| show_moves) {
        let color = if battle.move_cost_ap(id, cost) < unit.action_points {
            theme::MOVE_NEAR
        } else {
            theme::MOVE_FAR
        };
        tile_rect(&mut gizmos, state.battle.map(), pos, 0.8, color);
    }
    tile_rect(&mut gizmos, state.battle.map(), unit.pos, 0.95, theme::SELECTED);
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 3.0).sin();
    ground_circle(&mut gizmos, crate::coords::unit_spot(battle.map(), unit), (0.36 + 0.05 * pulse) * unit.side() as f32, theme::SELECTED.with_alpha(0.5 + 0.5 * pulse));

    // Path preview + cover shields at the hovered destination.
    let Some(hover) = selection.hovered.filter(|_| show_moves) else { return };
    if let Some(path) = range.path_to(hover) {
        // Blue while the soldier could still act afterwards, yellow for the dash.
        let mut prev = unit.pos;
        for &step in &path {
            let ap = battle.move_cost_ap(id, range.cost(step).unwrap_or(0));
            let color = if ap < unit.action_points { theme::MOVE_NEAR } else { theme::MOVE_FAR };
            let lift = Vec3::Y * 0.08;
            gizmos.line(tile_top(battle.map(), prev) + lift, tile_top(battle.map(), step) + lift, color);
            prev = step;
        }
        ground_circle(&mut gizmos, tile_top(battle.map(), hover), 0.3, theme::HOVER);
        draw_cover_shields(&mut gizmos, battle.map(), hover);
        // Look-ahead: who you could shoot from there (gold = you'd flank
        // them) and overwatchers that would fire during the walk.
        let path = range.path_to(hover).unwrap_or_default();
        let intel = crate::tooltip::move_intel(battle, unit, hover, &path);
        for id in &intel.targets {
            let Some(e) = battle.unit(*id) else { continue };
            let flank = intel.flanked.contains(id);
            let color = if flank { theme::TRACER } else { theme::TARGET };
            let top = tile_top(battle.map(), e.pos) + Vec3::Y * (theme::UNIT_HEIGHT + 0.9);
            let d = if flank { 0.22 } else { 0.15 };
            let diamond = [top + Vec3::Y * d, top + Vec3::X * d, top - Vec3::Y * d, top - Vec3::X * d, top + Vec3::Y * d];
            gizmos.linestrip(diamond, color);
            if flank {
                ground_circle(&mut gizmos, tile_top(battle.map(), e.pos), 0.48, color);
            }
        }
        for id in &intel.overwatchers {
            let Some(e) = battle.unit(*id) else { continue };
            let eye = tile_top(battle.map(), e.pos) + Vec3::Y * theme::UNIT_HEIGHT * 0.7;
            gizmos.line(eye, tile_top(battle.map(), hover) + Vec3::Y * 0.6, theme::TRACER.with_alpha(0.7));
        }
    }
}

/// Small walls on the sides of `pos` that offer cover (blue = half, dark blue = full).
fn draw_cover_shields(gizmos: &mut Gizmos, map: &Map, pos: GridPos) {
    let center = tile_top(map, pos);
    for (dir, level) in Direction::CARDINAL.into_iter().zip(cover_around(map, pos)) {
        let (color, h) = match level {
            CoverLevel::None => continue,
            CoverLevel::Half => (theme::COVER_HALF, 0.4),
            CoverLevel::Full => (theme::COVER_FULL, 0.8),
        };
        let (dx, dy) = dir.delta();
        let normal = Vec3::new(dx as f32, 0.0, -dy as f32) * 0.45;
        let side = Vec3::new(-normal.z, 0.0, normal.x) * 0.7;
        let a = center + normal - side;
        let b = center + normal + side;
        gizmos.linestrip([a, a + Vec3::Y * h, b + Vec3::Y * h, b, a], color);
    }
}

fn draw_targets(mut gizmos: Gizmos, state: Res<BattleState>, selection: Res<Selection>) {
    if selection.pending.is_some() {
        return;
    }
    // Hovering a seen enemy: how far it could move on its next turn.
    if let Some(enemy) = selection
        .hovered
        .and_then(|h| state.battle.unit_at(h))
        .filter(|e| e.team == Team::Enemy && state.battle.is_revealed(e.id))
    {
        let battle = &state.battle;
        let ap = battle.rules().combat.action_points as u32;
        let climb = Climb { max_up: enemy.stats.climb };
        let reach = tactics_core::pathfinding::reachable(battle.map(), enemy.pos, enemy.stats.mobility * ap, climb, |p| {
            battle.unit_at(p).is_none()
        });
        for (pos, cost) in reach.iter() {
            let near = cost <= enemy.stats.mobility;
            let c = theme::ENEMY.with_alpha(if near { 0.45 } else { 0.2 });
            tile_rect(&mut gizmos, battle.map(), pos, 0.55, c);
        }
    }
    let Some(id) = selection.unit else { return };
    let battle = &state.battle;
    let Some(unit) = battle.unit(id) else { return };
    for target in battle.visible_enemies(id) {
        let Some(t) = battle.unit(target) else { continue };
        let hovered = selection.hovered == Some(t.pos);
        let color = if hovered { theme::TARGET } else { theme::TARGET.with_alpha(0.35) };
        tile_rect(&mut gizmos, state.battle.map(), t.pos, 0.9, color);
        if hovered {
            let up = Vec3::Y * theme::UNIT_HEIGHT * 0.65;
            gizmos.line(tile_top(battle.map(), unit.pos) + up, tile_top(battle.map(), t.pos) + up, color);
        }
    }
}

/// A flat circle on the ground.
fn ground_circle(gizmos: &mut Gizmos, center: Vec3, radius: f32, color: Color) {
    let iso = Isometry3d::new(center + Vec3::Y * LIFT, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
    gizmos.circle(iso, radius, color).resolution(48);
}

/// Targeting mode: ability range, valid targets and the area of effect.
fn draw_targeting(mut gizmos: Gizmos, time: Res<Time<Real>>, state: Res<BattleState>, selection: Res<Selection>) {
    let (Some(id), Some(ability)) = (selection.unit, selection.pending.as_ref()) else { return };
    let battle = &state.battle;
    let (Some(unit), Some(def)) = (battle.unit(id), battle.ability(ability)) else { return };

    // The aimed target: a pulsing ring and a sight line, waiting for "fire".
    if let Some(aim) = selection.aim {
        let at = match aim {
            Target::Unit(t) => battle.unit(t).map(|u| u.pos),
            Target::Tile(p) => Some(p),
            Target::None => None,
        };
        if let Some(at) = at {
            let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 6.0).sin();
            let center = tile_top(battle.map(), at);
            let color = if def.targeting == Targeting::Ally { theme::HEAL } else { theme::TARGET };
            ground_circle(&mut gizmos, center, 0.45 + 0.08 * pulse, color);
            ground_circle(&mut gizmos, center, 0.3, color.with_alpha(0.5 + 0.5 * pulse));
            let up = Vec3::Y * theme::UNIT_HEIGHT * 0.65;
            gizmos.line(tile_top(battle.map(), unit.pos) + up, center + up, color.with_alpha(0.8));
        }
    }

    tile_rect(&mut gizmos, state.battle.map(), unit.pos, 0.95, theme::SELECTED);
    ground_circle(&mut gizmos, tile_top(battle.map(), unit.pos), battle.ability_range(unit, def), theme::MOVE_FAR.with_alpha(0.5));

    // Highlight every unit this ability could legally be used on.
    if matches!(def.targeting, Targeting::Enemy | Targeting::Ally) {
        for other in battle.units().iter().filter(|u| u.is_alive()) {
            let action = Action::UseAbility { unit: id, ability: ability.clone(), target: Target::Unit(other.id) };
            if battle.validate(&action).is_ok() {
                let color = if def.targeting == Targeting::Enemy { theme::TARGET } else { theme::HEAL };
                tile_rect(&mut gizmos, state.battle.map(), other.pos, 0.9, color);
            }
        }
    }

    // Area preview under the cursor.
    // (At the aimed tile once aimed, so the blast stays put while you confirm.)
    let area_at = match selection.aim {
        Some(Target::Tile(p)) => Some(p),
        _ => selection.hovered,
    };

    // Jump / boost: the line (or arc) to the landing tile, green if it can land there.
    if let (tactics_core::content::AbilityEffect::Leap { over_walls, .. }, Some(to)) = (&def.effect, area_at) {
        let ok = battle.validate(&Action::UseAbility { unit: id, ability: ability.clone(), target: Target::Tile(to) }).is_ok();
        let color = if ok { theme::SELECTED } else { theme::UI_DIM };
        let from = tile_top(battle.map(), unit.pos) + Vec3::Y * 0.3;
        let land = tile_top(battle.map(), to) + Vec3::Y * 0.3;
        let height = if *over_walls { (from.distance(land) * 0.3).clamp(1.2, 3.5) } else { 0.0 };
        let at = |k: f32| from.lerp(land, k) + Vec3::Y * (4.0 * height * k * (1.0 - k));
        for i in (0..24).step_by(2) {
            gizmos.line(at(i as f32 / 24.0), at((i + 1) as f32 / 24.0), color.with_alpha(0.85));
        }
        tile_rect(&mut gizmos, battle.map(), to, 0.9, color);
    }
    // (A flame weapon turns Shoot into a cone.)
    let def = &battle.with_weapon(unit, def);
    let area_at = match (selection.aim, def.targeting) {
        (Some(Target::Unit(t)), _) => battle.unit(t).map(|u| u.pos),
        _ => area_at,
    };
    if let (Some(profile), Some(hover)) = (def.effect.attack_profile(), area_at)
        && profile.is_area() {
            let target = match (def.targeting, battle.unit_at(hover)) {
                (Targeting::Enemy | Targeting::Ally, Some(u)) => Target::Unit(u.id),
                _ => Target::Tile(hover),
            };
            let ok = battle.validate(&Action::UseAbility { unit: id, ability: ability.clone(), target }).is_ok();
            let color = if ok { theme::EXPLOSION } else { theme::UI_DIM };
            // Rockets go off at the first thing in their way: show where.
            let point = battle.blast_point(unit.pos, profile, hover);
            let tiles = battle.area_tiles(unit.pos, profile, point);
            if profile.cone > 0.0 || profile.square {
                // A cone or a square blast: every tile it will hit.
                for t in &tiles {
                    tile_rect(&mut gizmos, battle.map(), *t, 0.9, color.with_alpha(0.7));
                }
            } else {
                ground_circle(&mut gizmos, tile_top(battle.map(), point), profile.area_radius, color);
            }
            if profile.direct {
                let from = tile_top(battle.map(), unit.pos) + Vec3::Y * theme::UNIT_HEIGHT * 0.7;
                let to = tile_top(battle.map(), point) + Vec3::Y * 0.5;
                for i in (0..20).step_by(2) {
                    gizmos.line(from.lerp(to, i as f32 / 20.0), from.lerp(to, (i + 1) as f32 / 20.0), color.with_alpha(0.8));
                }
            }
            // Throw preview: the arc the grenade would fly.
            let lobbed = battle.rules().presentation_of(&def.id).is_some_and(|p| p.projectile == tactics_core::content::ProjectileStyle::Arc);
            if lobbed {
                let from = tile_top(battle.map(), unit.pos) + Vec3::Y * theme::UNIT_HEIGHT * 0.8;
                let to = tile_top(battle.map(), hover) + Vec3::Y * 0.2;
                let height = (from.distance(to) * 0.35).max(0.8);
                let at = |k: f32| from.lerp(to, k) + Vec3::Y * (4.0 * height * k * (1.0 - k));
                // Dashed: every other segment.
                for i in (0..24).step_by(2) {
                    gizmos.line(at(i as f32 / 24.0), at((i + 1) as f32 / 24.0), color.with_alpha(0.8));
                }
            }
            for u in battle.units().iter().filter(|u| u.is_alive() && tiles.contains(&u.pos)) {
                tile_rect(&mut gizmos, state.battle.map(), u.pos, 0.7, color);
            }
        }
}

/// Armed charges (particle grenades): the area they'll hit, pulsing, and a
/// glowing marker per turn left on the grenade itself.
fn draw_armed_charges(mut gizmos: Gizmos, state: Res<BattleState>, time: Res<Time>) {
    let battle = &state.battle;
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 6.0).sin();
    for c in battle.armed_charges() {
        let color = theme::TARGET.with_alpha(0.35 + 0.5 * pulse);
        for t in battle.charge_tiles(c) {
            tile_rect(&mut gizmos, battle.map(), t, 0.92, color);
        }
        let at = tile_top(battle.map(), c.pos);
        gizmos.sphere(Isometry3d::from_translation(at + Vec3::Y * 0.12), 0.1 + 0.04 * pulse, theme::BEAM);
        for i in 0..c.turns {
            gizmos.sphere(Isometry3d::from_translation(at + Vec3::Y * (0.45 + 0.18 * i as f32)), 0.05, theme::TARGET);
        }
    }
}

fn draw_hover(mut gizmos: Gizmos, selection: Res<Selection>, state: Res<BattleState>) {
    if let Some(pos) = selection.hovered {
        tile_rect(&mut gizmos, state.battle.map(), pos, 1.0, theme::HOVER.with_alpha(0.6));
    }
}

/// HP pips above every living unit, oriented to face the camera.
fn draw_health_bars(
    mut gizmos: Gizmos,
    state: Res<BattleState>,
    entities: Res<UnitEntities>,
    units: Query<(&UnitVisual, &Transform, &Visibility)>,
    camera: Single<&Transform, With<CameraRig>>,
    selection: Res<Selection>,
    time: Res<Time<Real>>,
) {
    let right = camera.right().as_vec3();
    let stealth = state.battle.stealth();
    let preview = damage_preview(&state.battle, &selection);
    let flash = 0.5 + 0.5 * (time.elapsed_secs() * 8.0).sin();
    for (id, entity) in &entities.0 {
        let (Some(unit), Ok((visual, tr, vis))) = (state.battle.unit(*id), units.get(*entity)) else { continue };
        if !unit.is_alive() || *vis == Visibility::Hidden {
            continue;
        }
        // While the squad is concealed, show how close each enemy notices it.
        if stealth.concealed && unit.team == Team::Enemy {
            let r = stealth.detection.min(unit.stats.sight) * crate::coords::TILE_SIZE;
            gizmos.circle(
                Isometry3d::new(tr.translation + Vec3::Y * 0.05, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                r,
                Color::srgba(1.0, 0.3, 0.2, 0.55),
            );
        }
        // Top of the unit (flyers hover higher).
        let head = theme::UNIT_HEIGHT.max(unit.body.span().1);
        // Fear: a yellow (shaken) or violet (panicked) ring above the head.
        if !unit.fearless {
            let morale = &state.battle.rules().combat.morale;
            let ring = match morale.level(unit.fear) {
                tactics_core::content::FearLevel::Steady => None,
                tactics_core::content::FearLevel::Shaken => Some(theme::TRACER),
                tactics_core::content::FearLevel::Panicked => Some(theme::BEAM),
            };
            if let Some(color) = ring {
                let top = tr.translation + Vec3::Y * (head + 0.55);
                gizmos.circle(Isometry3d::new(top, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.22, color);
                gizmos.circle(Isometry3d::new(top, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.18, color);
            }
        }
        if unit.ai.dormant && unit.team == Team::Enemy {
            gizmos.circle(
                Isometry3d::new(tr.translation + Vec3::Y * (head + 0.65), Quat::IDENTITY),
                0.06,
                theme::UI_DIM,
            );
        }
        let base = tr.translation + Vec3::Y * (head + 0.35);
        let max = unit.stats.max_hp.max(1);
        let pip = 0.09;
        let start = base - right * (pip * max as f32 / 2.0);
        let color = match visual.team {
            Team::Player => theme::PLAYER,
            Team::Enemy => theme::ENEMY,
        };
        // Pips a hit would take: certain (min damage) and possible (max).
        let (dmin, dmax) = preview.iter().find(|(v, _, _)| *v == *id).map_or((0, 0), |(_, lo, hi)| (*lo, *hi));
        // Machines: a heat gauge filling up from the left (orange), with
        // the heat a hit would add flashing after it; blue while shut down.
        let heat = unit.heat.map(|_| unit.heat());
        for i in 0..max {
            let a = start + right * (pip * i as f32 + 0.01);
            let b = a + right * (pip - 0.02);
            let c = if let Some(heat) = heat {
                if i < heat {
                    if unit.is_shut_down() { theme::COOL } else { theme::HEAT.mix(&theme::TARGET, i as f32 / max as f32) }
                } else if i < heat + dmin {
                    theme::TARGET.with_alpha(0.4 + 0.6 * flash)
                } else if i < heat + dmax {
                    theme::EXPLOSION.with_alpha(0.3 + 0.5 * flash)
                } else {
                    Color::srgb(0.22, 0.24, 0.28)
                }
            } else if i >= unit.hp {
                Color::srgb(0.2, 0.2, 0.2)
            } else if i >= unit.hp - dmin {
                theme::TARGET.with_alpha(0.4 + 0.6 * flash)
            } else if i >= unit.hp - dmax {
                theme::EXPLOSION.with_alpha(0.3 + 0.5 * flash)
            } else {
                color
            };
            gizmos.line(a, b, c);
            gizmos.line(a + Vec3::Y * 0.03, b + Vec3::Y * 0.03, c);
        }
        // Action points left (player's turn, own soldiers): small gold dots.
        if unit.team == Team::Player && state.battle.active_team() == Team::Player && !state.battle.is_over() {
            let ap = unit.action_points as i32;
            let max_ap = (state.battle.rules().combat.action_points as i32).max(ap); // more after an adrenaline shot
            for i in 0..max_ap {
                let at = base + Vec3::Y * 0.12 + right * ((i as f32 - (max_ap - 1) as f32 / 2.0) * 0.12);
                let c = if i < ap { theme::TRACER } else { Color::srgba(0.3, 0.3, 0.3, 0.6) };
                gizmos.sphere(Isometry3d::from_translation(at), 0.03, c);
            }
        }
        // Mind-controlled: a pulsing purple ring.
        if unit.status.controlled > 0 {
            gizmos.circle(
                Isometry3d::new(base + Vec3::Y * 0.05, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                0.45 * unit.side() as f32,
                Color::srgb(0.75, 0.3, 1.0),
            );
            gizmos.circle(Isometry3d::new(base + Vec3::Y * 0.3, Quat::IDENTITY), 0.12, Color::srgb(0.75, 0.3, 1.0));
        }
        if unit.status.overwatch.is_some() {
            gizmos.circle(
                Isometry3d::new(base + Vec3::Y * 0.2, Quat::IDENTITY),
                0.08,
                theme::TRACER,
            );
        }
    }
}

/// Draws projectiles and explosions of the animation being played back.
fn draw_tracer(mut gizmos: Gizmos, playback: Res<Playback>) {
    use crate::playback::{AIM_TIME, ARC_TIME, EXPLOSION_TIME, SHOT_TIME};
    use tactics_core::content::ProjectileStyle;
    match playback.current() {
        Some(Anim::Explosion { center, radius, t }) => {
            let k = (t / EXPLOSION_TIME).min(1.0);
            for i in 0..3 {
                let r = radius * (k + i as f32 * 0.15).min(1.0);
                ground_circle(&mut gizmos, *center + Vec3::Y * (0.1 + i as f32 * 0.2), r.max(0.05), theme::EXPLOSION.with_alpha(1.0 - k));
            }
            gizmos.sphere(Isometry3d::from_translation(*center + Vec3::Y * 0.4), radius * k * 0.6, theme::TRACER.with_alpha(1.0 - k));
        }
        Some(Anim::Rocket { from, to, t }) => {
            // A rocket body with a flame at the tail and a smoke trail.
            let k = (t / crate::playback::rocket_time(*from, *to)).min(1.0);
            let dir = (*to - *from).normalize_or_zero();
            let head = from.lerp(*to, k);
            let tail = head - dir * 0.35;
            for off in [Vec3::ZERO, Vec3::Y * 0.02, -Vec3::Y * 0.02] {
                gizmos.line(tail + off, head + off, Color::srgb(0.75, 0.78, 0.8));
            }
            gizmos.sphere(Isometry3d::from_translation(tail), 0.06, theme::TRACER);
            gizmos.sphere(Isometry3d::from_translation(tail - dir * 0.08), 0.09, theme::EXPLOSION);
            let travelled = from.distance(head);
            let mut d = 0.3;
            while d < travelled {
                let p = from.lerp(head, (d / travelled.max(1e-3)).min(1.0));
                let age = (travelled - d) / crate::playback::ROCKET_SPEED;
                let fade = (1.0 - age * 1.5).clamp(0.0, 1.0);
                if fade > 0.0 {
                    gizmos.sphere(Isometry3d::from_translation(p + Vec3::Y * age * 0.6), 0.06 + age * 0.25, Color::srgba(0.7, 0.7, 0.72, 0.5 * fade));
                }
                d += 0.35;
            }
        }
        Some(Anim::Projectile { from, to, t }) => {
            // Parabola from `from` to `to`, apex proportional to distance.
            let height = (from.distance(*to) * 0.35).max(0.8);
            let at = |k: f32| from.lerp(*to, k) + Vec3::Y * (4.0 * height * k * (1.0 - k));
            let k = (t / ARC_TIME).min(1.0);
            gizmos.linestrip((0..=12).map(|i| at(k * i as f32 / 12.0)), theme::EXPLOSION.with_alpha(0.6));
            gizmos.sphere(Isometry3d::from_translation(at(k)), 0.09, theme::TRACER);
        }
        Some(Anim::Volley { muzzle, bullets, style, aim, interval, per_round, t }) => {
            if *t < *aim {
                // Aim line to where the first round is going.
                if let Some(b) = bullets.first() {
                    gizmos.line(*muzzle, b.end, theme::TARGET.with_alpha(0.25));
                }
                return;
            }
            for (i, b) in bullets.iter().enumerate() {
                let (fire, land) = b.timing(i, *per_round, *interval, *muzzle, *aim);
                if *t < fire {
                    continue;
                }
                let fly = (land - fire).max(1e-3);
                let k = ((*t - fire) / fly).min(1.0);
                match style {
                    ProjectileStyle::Beam => {
                        // A beam snaps out and lingers briefly.
                        let fade = 1.0 - ((*t - land) / 0.25).clamp(0.0, 1.0);
                        if fade <= 0.0 {
                            continue;
                        }
                        let head = muzzle.lerp(b.end, k);
                        for (j, off) in [Vec3::ZERO, Vec3::Y * 0.025, -Vec3::Y * 0.025, Vec3::X * 0.025].iter().enumerate() {
                            let c = if j == 0 { Color::WHITE } else { theme::BEAM };
                            gizmos.line(*muzzle + *off, head + *off, c.with_alpha(fade));
                        }
                    }
                    _ => {
                        if k >= 1.0 {
                            // Impact spark for a moment after landing.
                            let age = *t - land;
                            if age < 0.12 {
                                let c = if b.hit.is_some() { theme::TARGET } else { theme::TRACER };
                                gizmos.sphere(Isometry3d::from_translation(b.end), 0.05 + age, c.with_alpha(1.0 - age / 0.12));
                            }
                            continue;
                        }
                        let head = muzzle.lerp(b.end, k);
                        let tail = muzzle.lerp(b.end, (k - 0.3).max(0.0));
                        gizmos.line(tail, head, theme::TRACER);
                    }
                }
            }
        }
        Some(Anim::Shot { shooter, target, outcome, style, t, .. }) => {
            let mut end = *target;
            if matches!(outcome, ShotOutcome::Miss) {
                // Misses fly past the target.
                let dir = (*target - *shooter).normalize_or_zero();
                let side = dir.cross(Vec3::Y).normalize_or_zero();
                end = *target + side * 0.6 + dir * 2.0;
            }
            if *t < AIM_TIME {
                gizmos.line(*shooter, *target, theme::TARGET.with_alpha(0.3));
                return;
            }
            let k = ((*t - AIM_TIME) / SHOT_TIME).min(1.0);
            match style {
                ProjectileStyle::Beam => {
                    // A beam extends quickly and lingers, drawn as a bundle of lines.
                    let head = shooter.lerp(end, (k * 2.5).min(1.0));
                    let fade = 1.0 - ((k - 0.6) / 0.4).clamp(0.0, 1.0);
                    for (i, off) in [Vec3::ZERO, Vec3::Y * 0.025, -Vec3::Y * 0.025, Vec3::X * 0.025].iter().enumerate() {
                        let c = if i == 0 { Color::WHITE } else { theme::BEAM };
                        gizmos.line(*shooter + *off, head + *off, c.with_alpha(fade));
                    }
                }
                _ => {
                    let head = shooter.lerp(end, k);
                    let tail = shooter.lerp(end, (k - 0.25).max(0.0));
                    gizmos.line(tail, head, theme::TRACER);
                }
            }
        }
        _ => {}
    }
}

/// Marked zones: a pulsing outline, corner posts and a light beam, so evac
/// pads and objectives read from any zoom level.
fn draw_zones(mut gizmos: Gizmos, time: Res<Time<Real>>, state: Res<BattleState>) {
    let battle = &state.battle;
    let map = battle.map();
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 2.5).sin();
    for z in battle.zones().iter().filter(|z| z.visible) {
        let color = match z.kind.as_str() {
            "evac" => theme::ZONE_EVAC,
            "danger" => theme::ZONE_DANGER,
            _ => theme::ZONE_OBJECTIVE,
        };
        // Outline around the whole rectangle (world corners of the tiles).
        let h = TILE_SIZE / 2.0;
        let y = (z.min.y..=z.max.y)
            .flat_map(|gy| (z.min.x..=z.max.x).map(move |gx| GridPos::new(gx, gy)))
            .map(|p| tile_top(map, p).y)
            .fold(f32::MIN, f32::max)
            + 0.06;
        let lo = crate::coords::grid_to_world(z.min);
        let hi = crate::coords::grid_to_world(z.max);
        let (x0, x1) = (lo.x.min(hi.x) - h, lo.x.max(hi.x) + h);
        let (z0, z1) = (lo.z.min(hi.z) - h, lo.z.max(hi.z) + h);
        let corners = [Vec3::new(x0, y, z0), Vec3::new(x1, y, z0), Vec3::new(x1, y, z1), Vec3::new(x0, y, z1)];
        let c = color.with_alpha(0.55 + 0.45 * pulse);
        gizmos.linestrip(corners.iter().copied().chain(std::iter::once(corners[0])), c);
        let inset = 0.12 + 0.06 * pulse;
        let inner = [
            Vec3::new(x0 + inset, y, z0 + inset),
            Vec3::new(x1 - inset, y, z0 + inset),
            Vec3::new(x1 - inset, y, z1 - inset),
            Vec3::new(x0 + inset, y, z1 - inset),
        ];
        gizmos.linestrip(inner.iter().copied().chain(std::iter::once(inner[0])), color.with_alpha(0.35));
        for corner in corners {
            gizmos.line(corner, corner + Vec3::Y * 0.8, c);
        }
        // A beam of light from the centre, visible from far away.
        let center = Vec3::new((x0 + x1) / 2.0, y, (z0 + z1) / 2.0);
        let top = center + Vec3::Y * (4.0 + pulse);
        for off in [Vec3::ZERO, Vec3::X * 0.06, Vec3::Z * 0.06] {
            gizmos.line(center + off, top + off, color.with_alpha(0.7));
        }
        gizmos.circle(Isometry3d::new(top, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.25 + 0.1 * pulse, color);
    }
}

/// Damage ranges (victim, min, max) of the attack being lined up: the aimed
/// one, the pending ability on the hovered target, or the primary attack on
/// a hovered enemy.
fn damage_preview(battle: &Battle, selection: &Selection) -> Vec<(UnitId, i32, i32)> {
    let Some(sel) = selection.unit else { return Vec::new() };
    let Some(unit) = battle.unit(sel) else { return Vec::new() };
    let (ability, target) = match (selection.pending.as_ref(), selection.aim) {
        (Some(a), Some(t)) => (a.clone(), Some(t)),
        (Some(a), None) => {
            let t = battle.ability(a).zip(selection.hovered).and_then(|(d, h)| crate::input::target_for(battle, d, h));
            (a.clone(), t)
        }
        (None, _) => {
            let Some(enemy) = selection.hovered.and_then(|h| battle.unit_at(h)).filter(|e| e.team != unit.team) else {
                return Vec::new();
            };
            let Some(def) = battle.primary_attack(unit) else { return Vec::new() };
            (def.id.clone(), Some(Target::Unit(enemy.id)))
        }
    };
    let Some(target) = target else { return Vec::new() };
    if battle.validate(&Action::UseAbility { unit: sel, ability: ability.clone(), target }).is_err() {
        return Vec::new();
    }
    battle
        .preview(sel, &ability, target)
        .map(|v| v.into_iter().map(|(id, p)| (id, p.damage.0, p.damage.1)).collect())
        .unwrap_or_default()
}

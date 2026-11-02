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

fn draw_movement(mut gizmos: Gizmos, state: Res<BattleState>, selection: Res<Selection>) {
    if selection.pending.is_some() {
        return; // targeting mode replaces the movement overlay
    }
    let (Some(id), Some(range)) = (selection.unit, selection.range.as_ref()) else { return };
    let battle = &state.battle;
    let Some(unit) = battle.unit(id) else { return };

    // Blue = move and still act; yellow = dash (uses every AP).
    for (pos, cost) in range.iter() {
        let color = if battle.move_cost_ap(id, cost) < unit.action_points {
            theme::MOVE_NEAR
        } else {
            theme::MOVE_FAR
        };
        tile_rect(&mut gizmos, state.battle.map(), pos, 0.8, color);
    }
    tile_rect(&mut gizmos, state.battle.map(), unit.pos, 0.95, theme::SELECTED);

    // Path preview + cover shields at the hovered destination.
    let Some(hover) = selection.hovered else { return };
    if let Some(path) = range.path_to(hover) {
        let points = std::iter::once(unit.pos)
            .chain(path)
            .map(|p| tile_top(battle.map(), p) + Vec3::Y * 0.08);
        gizmos.linestrip(points, theme::HOVER);
        draw_cover_shields(&mut gizmos, battle.map(), hover);
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
    if let (Some(profile), Some(hover)) = (def.effect.attack_profile(), area_at)
        && profile.area_radius > 0.0 {
            let ok = battle
                .validate(&Action::UseAbility { unit: id, ability: ability.clone(), target: Target::Tile(hover) })
                .is_ok();
            let color = if ok { theme::EXPLOSION } else { theme::UI_DIM };
            ground_circle(&mut gizmos, tile_top(battle.map(), hover), profile.area_radius, color);
            for u in battle.units().iter().filter(|u| u.is_alive() && u.pos.distance(hover) <= profile.area_radius) {
                tile_rect(&mut gizmos, state.battle.map(), u.pos, 0.7, color);
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
) {
    let right = camera.right().as_vec3();
    let stealth = state.battle.stealth();
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
        // Fear: a yellow (shaken) or violet (panicked) ring above the head.
        if !unit.fearless {
            let morale = &state.battle.rules().combat.morale;
            let ring = match morale.level(unit.fear) {
                tactics_core::content::FearLevel::Steady => None,
                tactics_core::content::FearLevel::Shaken => Some(theme::TRACER),
                tactics_core::content::FearLevel::Panicked => Some(theme::BEAM),
            };
            if let Some(color) = ring {
                let top = tr.translation + Vec3::Y * (theme::UNIT_HEIGHT + 0.55);
                gizmos.circle(Isometry3d::new(top, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.22, color);
                gizmos.circle(Isometry3d::new(top, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.18, color);
            }
        }
        if unit.ai.dormant && unit.team == Team::Enemy {
            gizmos.circle(
                Isometry3d::new(tr.translation + Vec3::Y * (theme::UNIT_HEIGHT + 0.65), Quat::IDENTITY),
                0.06,
                theme::UI_DIM,
            );
        }
        let base = tr.translation + Vec3::Y * (theme::UNIT_HEIGHT + 0.35);
        let max = unit.stats.max_hp.max(1);
        let pip = 0.09;
        let start = base - right * (pip * max as f32 / 2.0);
        let color = match visual.team {
            Team::Player => theme::PLAYER,
            Team::Enemy => theme::ENEMY,
        };
        for i in 0..max {
            let a = start + right * (pip * i as f32 + 0.01);
            let b = a + right * (pip - 0.02);
            let c = if i < unit.hp { color } else { Color::srgb(0.2, 0.2, 0.2) };
            gizmos.line(a, b, c);
            gizmos.line(a + Vec3::Y * 0.03, b + Vec3::Y * 0.03, c);
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
        Some(Anim::Projectile { from, to, t }) => {
            // Parabola from `from` to `to`, apex proportional to distance.
            let height = (from.distance(*to) * 0.35).max(0.8);
            let at = |k: f32| from.lerp(*to, k) + Vec3::Y * (4.0 * height * k * (1.0 - k));
            let k = (t / ARC_TIME).min(1.0);
            gizmos.linestrip((0..=12).map(|i| at(k * i as f32 / 12.0)), theme::EXPLOSION.with_alpha(0.6));
            gizmos.sphere(Isometry3d::from_translation(at(k)), 0.09, theme::TRACER);
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

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
use crate::coords::{TILE_SIZE, grid_to_world};
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
                (draw_movement, draw_targets).run_if(player_can_act),
                draw_hover,
                draw_health_bars,
                draw_tracer,
            )
                .in_set(FrameSet::Present),
        );
    }
}

const LIFT: f32 = 0.02;

/// A flat square outline on a tile.
fn tile_rect(gizmos: &mut Gizmos, pos: GridPos, scale: f32, color: Color) {
    let iso = Isometry3d::new(
        grid_to_world(pos) + Vec3::Y * LIFT,
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    );
    gizmos.rect(iso, Vec2::splat(TILE_SIZE * scale), color);
}

fn draw_movement(mut gizmos: Gizmos, state: Res<BattleState>, selection: Res<Selection>) {
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
        tile_rect(&mut gizmos, pos, 0.8, color);
    }
    tile_rect(&mut gizmos, unit.pos, 0.95, theme::SELECTED);

    // Path preview + cover shields at the hovered destination.
    let Some(hover) = selection.hovered else { return };
    if let Some(path) = range.path_to(hover) {
        let points = std::iter::once(unit.pos)
            .chain(path)
            .map(|p| grid_to_world(p) + Vec3::Y * 0.08);
        gizmos.linestrip(points, theme::HOVER);
        draw_cover_shields(&mut gizmos, battle.map(), hover);
    }
}

/// Small walls on the sides of `pos` that offer cover (blue = half, dark blue = full).
fn draw_cover_shields(gizmos: &mut Gizmos, map: &Map, pos: GridPos) {
    let center = grid_to_world(pos);
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
    let Some(id) = selection.unit else { return };
    let battle = &state.battle;
    let Some(unit) = battle.unit(id) else { return };
    for target in battle.visible_enemies(id) {
        let Some(t) = battle.unit(target) else { continue };
        let hovered = selection.hovered == Some(t.pos);
        let color = if hovered { theme::TARGET } else { theme::TARGET.with_alpha(0.35) };
        tile_rect(&mut gizmos, t.pos, 0.9, color);
        if hovered {
            let up = Vec3::Y * theme::UNIT_HEIGHT * 0.65;
            gizmos.line(grid_to_world(unit.pos) + up, grid_to_world(t.pos) + up, color);
        }
    }
}

fn draw_hover(mut gizmos: Gizmos, selection: Res<Selection>) {
    if let Some(pos) = selection.hovered {
        tile_rect(&mut gizmos, pos, 1.0, theme::HOVER.with_alpha(0.6));
    }
}

/// HP pips above every living unit, oriented to face the camera.
fn draw_health_bars(
    mut gizmos: Gizmos,
    state: Res<BattleState>,
    entities: Res<UnitEntities>,
    units: Query<(&UnitVisual, &Transform)>,
    camera: Single<&Transform, With<CameraRig>>,
) {
    let right = camera.right().as_vec3();
    for (id, entity) in &entities.0 {
        let (Some(unit), Ok((visual, tr))) = (state.battle.unit(*id), units.get(*entity)) else { continue };
        if !unit.is_alive() {
            continue;
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
        if unit.status.overwatch {
            gizmos.circle(
                Isometry3d::new(base + Vec3::Y * 0.2, Quat::IDENTITY),
                0.08,
                theme::TRACER,
            );
        }
    }
}

/// Draws the projectile of the shot currently being played back.
fn draw_tracer(mut gizmos: Gizmos, playback: Res<Playback>) {
    const AIM: f32 = 0.35;
    const FLY: f32 = 0.45;
    if let Some(Anim::Shot { shooter, target, outcome, t, .. }) = playback.current() {
        let mut end = *target;
        if matches!(outcome, ShotOutcome::Miss) {
            // Misses fly past the target.
            let dir = (*target - *shooter).normalize_or_zero();
            let side = dir.cross(Vec3::Y).normalize_or_zero();
            end = *target + side * 0.6 + dir * 2.0;
        }
        if *t < AIM {
            gizmos.line(*shooter, *target, theme::TARGET.with_alpha(0.3));
        } else {
            let k = ((*t - AIM) / FLY).min(1.0);
            let head = shooter.lerp(end, k);
            let tail = shooter.lerp(end, (k - 0.25).max(0.0));
            gizmos.line(tail, head, theme::TRACER);
        }
    }
}

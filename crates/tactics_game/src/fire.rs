//! Fire on the battlefield: flickering flames (and a few lights) on every
//! burning tile of the battle (`Battle::fires`), scorch marks where a fire
//! burned out, and a quick sweep of flame for cone attacks (flamethrowers,
//! [`FlameBurst`], spawned by playback).

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use tactics_core::grid::GridPos;

use crate::FrameSet;
use crate::coords::tile_top;
use crate::state::{BattleReset, BattleState};

pub struct FirePlugin;

impl Plugin for FirePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FireVisuals>()
            .add_systems(Startup, setup)
            .add_systems(Update, (sync_fires, flicker, flame_bursts, burst_flames, sync_gas, drift_gas, sync_vines, writhe).chain().in_set(FrameSet::Present));
    }
}

/// A flamethrower sweep to show: flames rush along `tiles` (world points,
/// nearest first) and die down.
#[derive(Component, Debug, Clone)]
pub struct FlameBurst {
    pub tiles: Vec<Vec3>,
    pub t: f32,
}

/// Burning tiles with lights at most (lights are expensive).
const MAX_LIGHTS: usize = 10;

#[derive(Resource)]
struct FireAssets {
    flame: Handle<Mesh>,
    scorch: Handle<Mesh>,
    outer: Handle<StandardMaterial>,
    inner: Handle<StandardMaterial>,
    scorch_mat: Handle<StandardMaterial>,
    puff: Handle<Mesh>,
    gas_mat: Handle<StandardMaterial>,
    vine_mat: Handle<StandardMaterial>,
    tendril: Handle<Mesh>,
    tendril_mat: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct FireVisuals {
    fires: HashMap<GridPos, Entity>,
    scorches: HashMap<GridPos, Entity>,
    gas: HashMap<GridPos, Entity>,
    vines: HashMap<GridPos, Entity>,
}

/// A tendril sprouting from a vine tile (sways slowly).
#[derive(Component)]
struct Tendril {
    phase: f32,
    base: Quat,
}

/// One drifting puff of a gas cloud.
#[derive(Component)]
struct GasPuff {
    phase: f32,
    base: Vec3,
}

/// One flame tongue (flickers around `base` scale).
#[derive(Component)]
struct Flame {
    phase: f32,
    base: Vec3,
}

#[derive(Component)]
struct FireLight {
    phase: f32,
}

/// A flame of a [`FlameBurst`]: appears after `delay`, lives `life` seconds.
#[derive(Component)]
struct BurstFlame {
    age: f32,
    delay: f32,
    life: f32,
    base: Vec3,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let glow = |c: Color, e: LinearRgba, a: f32| StandardMaterial {
        base_color: c.with_alpha(a),
        emissive: e,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };
    commands.insert_resource(FireAssets {
        flame: meshes.add(Cone { radius: 0.18, height: 0.7 }),
        scorch: meshes.add(Circle::new(0.48)),
        outer: materials.add(glow(Color::srgb(1.0, 0.42, 0.08), LinearRgba::rgb(3.0, 0.9, 0.1), 0.8)),
        inner: materials.add(glow(Color::srgb(1.0, 0.85, 0.3), LinearRgba::rgb(4.0, 3.0, 0.8), 0.9)),
        scorch_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.04, 0.03, 0.03, 0.7),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        puff: meshes.add(Sphere::new(0.42)),
        vine_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.28, 0.06, 0.24, 0.85),
            emissive: LinearRgba::rgb(0.12, 0.0, 0.1),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.4,
            ..default()
        }),
        tendril: meshes.add(Capsule3d::new(0.035, 0.32)),
        tendril_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.08, 0.34),
            emissive: LinearRgba::rgb(0.35, 0.02, 0.3),
            perceptual_roughness: 0.3,
            ..default()
        }),
        gas_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.62, 0.78, 0.22, 0.28),
            emissive: LinearRgba::rgb(0.06, 0.09, 0.0),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    });
}

/// Small deterministic hash of a tile (flame layout and phase).
fn jitter(p: GridPos, k: u32) -> f32 {
    let h = (p.x as u32).wrapping_mul(73_856_093) ^ (p.y as u32).wrapping_mul(19_349_663) ^ k.wrapping_mul(83_492_791);
    (h % 1000) as f32 / 1000.0
}

fn sync_fires(
    mut commands: Commands,
    state: Res<BattleState>,
    assets: Res<FireAssets>,
    mut visuals: ResMut<FireVisuals>,
    mut resets: MessageReader<BattleReset>,
    settings: Res<crate::settings::GameSettings>,
) {
    let low = settings.low_effects();
    if resets.read().count() > 0 {
        let visuals = &mut *visuals;
        for (_, e) in visuals.fires.drain().chain(visuals.scorches.drain()).chain(visuals.gas.drain()).chain(visuals.vines.drain()) {
            commands.entity(e).despawn();
        }
    }
    let b = &state.battle;
    let fires = b.fires();
    // Burned out: leave a scorch mark.
    let out: Vec<GridPos> = visuals.fires.keys().copied().filter(|p| !fires.contains_key(p)).collect();
    for p in out {
        if let Some(e) = visuals.fires.remove(&p) {
            commands.entity(e).despawn();
        }
        visuals.scorches.entry(p).or_insert_with(|| {
            commands
                .spawn((
                    Mesh3d(assets.scorch.clone()),
                    MeshMaterial3d(assets.scorch_mat.clone()),
                    Transform::from_translation(tile_top(b.map(), p) + Vec3::Y * 0.015)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                    NotShadowCaster,
                ))
                .id()
        });
    }
    // New fires.
    let lights = visuals.fires.len();
    let new: Vec<GridPos> = fires.keys().copied().filter(|p| !visuals.fires.contains_key(p)).collect();
    for (i, p) in new.into_iter().enumerate() {
        let top = tile_top(b.map(), p);
        let root = commands.spawn((Transform::from_translation(top), Visibility::default())).id();
        for k in 0..if low { 2 } else { 4u32 } {
            let off = Vec3::new(jitter(p, k) - 0.5, 0.0, jitter(p, k + 7) - 0.5) * 0.55;
            let s = 0.7 + jitter(p, k + 13) * 0.6;
            let base = Vec3::new(s, s * (0.8 + jitter(p, k + 3) * 0.6), s);
            let mat = if k == 0 { assets.inner.clone() } else { assets.outer.clone() };
            commands.spawn((
                Mesh3d(assets.flame.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(off + Vec3::Y * 0.3 * base.y).with_scale(base),
                Flame { phase: jitter(p, k + 21) * std::f32::consts::TAU, base },
                NotShadowCaster,
                ChildOf(root),
            ));
        }
        if !low && lights + i < MAX_LIGHTS {
            commands.spawn((
                PointLight { color: Color::srgb(1.0, 0.55, 0.2), intensity: 60_000.0, range: 4.0, shadow_maps_enabled: false, ..default() },
                Transform::from_translation(Vec3::Y * 0.8),
                FireLight { phase: jitter(p, 99) * std::f32::consts::TAU },
                ChildOf(root),
            ));
        }
        visuals.fires.insert(p, root);
    }
}

fn flicker(time: Res<Time>, mut flames: Query<(&Flame, &mut Transform)>, mut lights: Query<(&FireLight, &mut PointLight)>) {
    let t = time.elapsed_secs();
    for (f, mut tr) in &mut flames {
        let k = 0.85 + 0.2 * (t * 9.0 + f.phase).sin() + 0.08 * (t * 23.0 + f.phase * 2.0).sin();
        let sway = 0.08 * (t * 5.0 + f.phase).sin();
        tr.scale = Vec3::new(f.base.x * (1.05 - 0.1 * k), f.base.y * k, f.base.z * (1.05 - 0.1 * k));
        tr.rotation = Quat::from_rotation_z(sway) * Quat::from_rotation_x(sway * 0.6);
    }
    for (l, mut light) in &mut lights {
        light.intensity = 60_000.0 * (0.8 + 0.2 * (t * 11.0 + l.phase).sin() + 0.1 * (t * 29.0 + l.phase).sin());
    }
}

/// Turns each new [`FlameBurst`] into a rush of flames along its tiles.
fn flame_bursts(mut commands: Commands, assets: Res<FireAssets>, bursts: Query<(Entity, &FlameBurst)>) {
    for (e, burst) in &bursts {
        let n = burst.tiles.len().max(1) as f32;
        for (i, at) in burst.tiles.iter().enumerate() {
            for k in 0..2u32 {
                let p = GridPos::new(at.x.round() as i32, at.z.round() as i32);
                let off = Vec3::new(jitter(p, k + 40) - 0.5, 0.0, jitter(p, k + 50) - 0.5) * 0.5;
                let s = 0.9 + jitter(p, k + 60) * 0.6;
                commands.spawn((
                    Mesh3d(assets.flame.clone()),
                    MeshMaterial3d(if k == 0 { assets.inner.clone() } else { assets.outer.clone() }),
                    Transform::from_translation(*at + off).with_scale(Vec3::ZERO),
                    BurstFlame { age: 0.0, delay: 0.35 * i as f32 / n, life: 0.55, base: Vec3::new(s, s * 1.4, s) },
                    NotShadowCaster,
                ));
            }
        }
        commands.entity(e).despawn();
    }
}

fn burst_flames(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut BurstFlame, &mut Transform)>) {
    for (e, mut f, mut tr) in &mut q {
        f.age += time.delta_secs();
        let k = (f.age - f.delay) / f.life;
        if k >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        // Flare up, then die down.
        let s = if k < 0.0 { 0.0 } else { (k * std::f32::consts::PI).sin() };
        tr.scale = f.base * s;
    }
}

/// Gas clouds (`Battle::gas`): a few slow, see-through yellow-green puffs on
/// every gassed tile.
fn sync_gas(
    mut commands: Commands,
    state: Res<BattleState>,
    assets: Res<FireAssets>,
    mut visuals: ResMut<FireVisuals>,
    settings: Res<crate::settings::GameSettings>,
) {
    let low = settings.low_effects();
    let b = &state.battle;
    let gas = b.gas();
    let gone: Vec<GridPos> = visuals.gas.keys().copied().filter(|p| !gas.contains_key(p)).collect();
    for p in gone {
        if let Some(e) = visuals.gas.remove(&p) {
            commands.entity(e).despawn();
        }
    }
    let new: Vec<GridPos> = gas.keys().copied().filter(|p| !visuals.gas.contains_key(p)).collect();
    for p in new {
        let root = commands.spawn((Transform::from_translation(tile_top(b.map(), p)), Visibility::default())).id();
        for k in 0..if low { 1 } else { 3u32 } {
            let base = Vec3::new(jitter(p, k + 40) - 0.5, 0.35 + jitter(p, k + 50) * 0.5, jitter(p, k + 60) - 0.5) * Vec3::new(0.6, 1.0, 0.6);
            commands.spawn((
                Mesh3d(assets.puff.clone()),
                MeshMaterial3d(assets.gas_mat.clone()),
                Transform::from_translation(base).with_scale(Vec3::splat(0.8 + jitter(p, k + 70) * 0.5)),
                GasPuff { phase: jitter(p, k + 80) * std::f32::consts::TAU, base },
                NotShadowCaster,
                ChildOf(root),
            ));
        }
        visuals.gas.insert(p, root);
    }
}

fn drift_gas(time: Res<Time>, mut puffs: Query<(&GasPuff, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (g, mut tr) in &mut puffs {
        let w = t * 0.6 + g.phase;
        tr.translation = g.base + Vec3::new(0.12 * w.sin(), 0.06 * (w * 1.7).sin(), 0.12 * (w * 0.8).cos());
    }
}

/// Symbiote vines (`Battle::vines`): a glistening purple mat on the deck with a
/// few tendrils sprouting from it.
fn sync_vines(
    mut commands: Commands,
    state: Res<BattleState>,
    assets: Res<FireAssets>,
    mut visuals: ResMut<FireVisuals>,
    settings: Res<crate::settings::GameSettings>,
) {
    let low = settings.low_effects();
    let b = &state.battle;
    let vines = b.vines();
    let gone: Vec<GridPos> = visuals.vines.keys().copied().filter(|p| !vines.contains(p)).collect();
    for p in gone {
        if let Some(e) = visuals.vines.remove(&p) {
            commands.entity(e).despawn();
        }
    }
    let new: Vec<GridPos> = vines.iter().copied().filter(|p| !visuals.vines.contains_key(p)).collect();
    for p in new {
        let root = commands
            .spawn((
                Mesh3d(assets.scorch.clone()),
                MeshMaterial3d(assets.vine_mat.clone()),
                Transform::from_translation(tile_top(b.map(), p) + Vec3::Y * 0.02)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(1.05)),
                NotShadowCaster,
            ))
            .id();
        for k in 0..2u32 {
            if low || jitter(p, k + 90) < 0.35 {
                continue;
            }
            let off = Vec3::new(jitter(p, k + 91) - 0.5, jitter(p, k + 92) - 0.5, 0.0) * 0.7;
            let base = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2 + (jitter(p, k + 93) - 0.5) * 0.9)
                * Quat::from_rotation_z((jitter(p, k + 94) - 0.5) * 0.9);
            commands.spawn((
                Mesh3d(assets.tendril.clone()),
                MeshMaterial3d(assets.tendril_mat.clone()),
                // Child of the (rotated) mat: local -Z is up out of the deck.
                Transform::from_translation(off + Vec3::new(0.0, 0.0, 0.18)).with_rotation(base),
                Tendril { phase: jitter(p, k + 95) * std::f32::consts::TAU, base },
                NotShadowCaster,
                ChildOf(root),
            ));
        }
        visuals.vines.insert(p, root);
    }
}

fn writhe(time: Res<Time>, mut tendrils: Query<(&Tendril, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (v, mut tr) in &mut tendrils {
        let w = t * 1.3 + v.phase;
        tr.rotation = v.base * Quat::from_rotation_x(0.25 * w.sin()) * Quat::from_rotation_y(0.2 * (w * 0.7).cos());
    }
}

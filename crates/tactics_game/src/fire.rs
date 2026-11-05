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
            .add_systems(Update, (sync_fires, flicker, flame_bursts, burst_flames).chain().in_set(FrameSet::Present));
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
}

#[derive(Resource, Default)]
struct FireVisuals {
    fires: HashMap<GridPos, Entity>,
    scorches: HashMap<GridPos, Entity>,
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
) {
    if resets.read().count() > 0 {
        let visuals = &mut *visuals;
        for (_, e) in visuals.fires.drain().chain(visuals.scorches.drain()) {
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
        for k in 0..4u32 {
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
        if lights + i < MAX_LIGHTS {
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

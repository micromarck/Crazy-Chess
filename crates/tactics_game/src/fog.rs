//! Fog of war on the board, in missions with fog.
//!
//! Every tile is in one of three states, from what the squad can see:
//! * **in sight** — drawn normally;
//! * **explored** — seen before, now out of sight: dimmed;
//! * **unexplored** — never seen: dark.
//!
//! Each state is a translucent dark layer laid over the tile (on top of a
//! wall or cover block, so they darken too). Like the board, the layer is
//! built per [`CHUNK`](crate::board::CHUNK) with one merged mesh per shade,
//! and only chunks whose tiles changed state are rebuilt (a few per frame),
//! so it stays cheap on huge maps. Hidden enemies are handled separately
//! (`units.rs`); this is only the look of the ground.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use tactics_core::grid::{GridPos, Terrain};
use tactics_core::prelude::*;

use crate::board::CHUNK;
use crate::coords::tile_top;
use crate::state::{BattleReset, BattleState};
use crate::theme;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Fog>().add_systems(Startup, setup).add_systems(Update, (update_fog, build_fog_chunks).chain());
    }
}

const VISIBLE: u8 = 0;
const EXPLORED: u8 = 1;
const UNEXPLORED: u8 = 2;
/// Not drawn yet (forces the first build).
const UNKNOWN: u8 = 255;

#[derive(Resource)]
struct FogAssets {
    dim: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

/// Per-tile fog state and the chunks drawing it.
#[derive(Resource, Default)]
struct Fog {
    size: (i32, i32),
    /// What each tile currently shows (VISIBLE / EXPLORED / UNEXPLORED).
    state: Vec<u8>,
    /// Tiles the squad has ever seen.
    explored: Vec<bool>,
    /// What the visibility was last computed for.
    key: Option<u64>,
    chunks: std::collections::HashMap<IVec2, Entity>,
    pending: std::collections::BTreeSet<(i32, i32)>,
}

#[derive(Component)]
struct FogChunk;

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mut shade = |alpha: f32| {
        materials.add(StandardMaterial {
            base_color: Color::srgba(0.02, 0.03, 0.06, alpha),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })
    };
    commands.insert_resource(FogAssets { dim: shade(0.42), dark: shade(0.78) });
}

/// Recomputes what the squad sees when it (or the map) changed, and queues
/// the chunks whose tiles changed state.
fn update_fog(mut commands: Commands, state: Res<BattleState>, mut fog: ResMut<Fog>, mut resets: MessageReader<BattleReset>) {
    let b = &state.battle;
    let map = b.map();
    let (w, h) = (map.width(), map.height());
    let reset = resets.read().count() > 0;
    if reset || fog.size != (w, h) || !b.fog() {
        for (_, e) in fog.chunks.drain() {
            commands.entity(e).despawn();
        }
        fog.pending.clear();
        fog.size = (w, h);
        fog.state = vec![UNKNOWN; (w * h) as usize];
        fog.explored = vec![false; (w * h) as usize];
        fog.key = None;
    }
    if !b.fog() {
        return;
    }
    // Recompute only when the squad or the terrain changed.
    let mut key = b.map_version() as u64 ^ 0x9E37_79B9;
    for u in b.living(Team::Player) {
        key = key.rotate_left(13) ^ (((u.pos.x as u64) << 32) | u.pos.y as u32 as u64) ^ (u.stats.sight.to_bits() as u64);
    }
    if fog.key == Some(key) {
        return;
    }
    fog.key = Some(key);

    let index = |p: GridPos| (p.y * w + p.x) as usize;
    let mut visible = vec![false; (w * h) as usize];
    for u in b.living(Team::Player) {
        visible[index(u.pos)] = true;
        for p in tactics_core::visibility::visible_tiles(map, u.pos, u.stats.sight) {
            visible[index(p)] = true;
        }
    }
    let mut changed = std::collections::BTreeSet::new();
    for (i, seen) in visible.iter().enumerate() {
        if *seen {
            fog.explored[i] = true;
        }
        let s = if *seen {
            VISIBLE
        } else if fog.explored[i] {
            EXPLORED
        } else {
            UNEXPLORED
        };
        if fog.state[i] != s {
            fog.state[i] = s;
            let (x, y) = (i as i32 % w, i as i32 / w);
            changed.insert((x.div_euclid(CHUNK), y.div_euclid(CHUNK)));
        }
    }
    fog.pending.extend(changed);
}

/// Builds queued fog chunks, nearest to the camera first, within a few
/// milliseconds per frame.
fn build_fog_chunks(
    mut commands: Commands,
    mut fog: ResMut<Fog>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<FogAssets>,
    state: Res<BattleState>,
    rig: Option<Single<&crate::camera::CameraRig>>,
) {
    if fog.pending.is_empty() {
        return;
    }
    let map = state.battle.map();
    let near = crate::coords::world_to_grid(rig.map_or(Vec3::ZERO, |r| r.focus));
    let mut order: Vec<(i32, i32)> = fog.pending.iter().copied().collect();
    order.sort_by_key(|c| ((c.0 * CHUNK + CHUNK / 2 - near.x).abs().max((c.1 * CHUNK + CHUNK / 2 - near.y).abs()), *c));
    let started = bevy::platform::time::Instant::now();
    for c in order {
        if started.elapsed().as_secs_f32() > 0.004 {
            break;
        }
        fog.pending.remove(&c);
        let key = IVec2::new(c.0, c.1);
        if let Some(old) = fog.chunks.remove(&key) {
            commands.entity(old).despawn();
        }
        let mut quads: [Vec<Vec3>; 2] = [Vec::new(), Vec::new()];
        for y in c.1 * CHUNK..((c.1 + 1) * CHUNK).min(map.height()) {
            for x in c.0 * CHUNK..((c.0 + 1) * CHUNK).min(map.width()) {
                let p = GridPos::new(x, y);
                let s = fog.state[(y * map.width() + x) as usize];
                if s == VISIBLE || s == UNKNOWN || map.terrain(p) == Terrain::Void {
                    continue;
                }
                // Over whatever stands on the tile, so walls darken too.
                let lift = match map.terrain(p) {
                    Terrain::HighCover => theme::HIGH_COVER_HEIGHT,
                    Terrain::LowCover => theme::LOW_COVER_HEIGHT,
                    _ => 0.0,
                } + 0.04;
                quads[(s == UNEXPLORED) as usize].push(tile_top(map, p) + Vec3::Y * lift);
            }
        }
        if quads.iter().all(|q| q.is_empty()) {
            continue;
        }
        let parent = commands.spawn((FogChunk, Transform::IDENTITY, Visibility::default())).id();
        for (centres, material) in quads.iter().zip([&assets.dim, &assets.dark]) {
            if !centres.is_empty() {
                commands.spawn((Mesh3d(meshes.add(quad_mesh(centres))), MeshMaterial3d(material.clone()), ChildOf(parent)));
            }
        }
        fog.chunks.insert(key, parent);
    }
}

/// One mesh of flat, upward-facing tile-sized squares centred on `centres`.
fn quad_mesh(centres: &[Vec3]) -> Mesh {
    let h = 0.5;
    let mut positions = Vec::with_capacity(centres.len() * 4);
    let mut indices = Vec::with_capacity(centres.len() * 6);
    for (i, c) in centres.iter().enumerate() {
        let base = (i * 4) as u32;
        positions.extend([
            [c.x - h, c.y, c.z - h],
            [c.x + h, c.y, c.z - h],
            [c.x + h, c.y, c.z + h],
            [c.x - h, c.y, c.z + h],
        ]);
        // Counter-clockwise seen from above.
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let n = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; n])
        .with_inserted_indices(Indices::U32(indices))
}

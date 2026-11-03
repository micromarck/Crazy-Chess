//! Level geometry: floor tiles at their height, cliff columns, cover blocks,
//! ladders and props.
//!
//! Built from the core [`Map`](tactics_core::grid::Map) and the battle's
//! props with primitive meshes. Cut-out tiles (`Terrain::Void`) are simply not
//! drawn. The board is rebuilt when a battle starts ([`BattleReset`]) and when
//! scripts or explosions change the level ([`BoardDirty`]).
//!
//! The board is split into [`CHUNK`]-sized chunks built a few per frame
//! (nearest the camera first), each merging its static meshes per material;
//! a change only rebuilds the chunks around it. Walls tall enough for the
//! cutaway and closed doors stay separate entities.
//!
//! To use real art, replace [`tile_parts`] with code that spawns glTF scenes
//! per terrain/prop kind — nothing else depends on how the board looks.

use bevy::prelude::*;
use tactics_core::grid::{CoverLevel, Direction, GridPos, Surface, Terrain};

use crate::coords::{LEVEL_HEIGHT, TILE_SIZE, grid_to_world, tile_top};
use crate::state::{BattleReset, BattleState};
use crate::theme;

pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BoardDirty>()
            .add_systems(PreStartup, crate::scenery::setup_scenery)
            .init_resource::<BoardChunks>()
            .add_systems(Startup, (setup_assets, spawn_lights).chain())
            .add_systems(Update, (queue_chunks, build_chunks).chain());
    }
}

/// Ask for the board to be redrawn around a tile (terrain, height or a prop
/// changed there), or everywhere (`None`).
#[derive(Message, Debug, Clone, Copy)]
pub struct BoardDirty(pub Option<GridPos>);

/// Marks every entity belonging to the level so it can be cleared on rebuild.
#[derive(Component)]
pub struct BoardEntity;

/// Meshes and materials shared by all tiles (one GPU buffer per kind, not per tile).
#[derive(Resource)]
struct BoardAssets {
    floor: Handle<Mesh>,
    column: Handle<Mesh>,
    low: Handle<Mesh>,
    high: Handle<Mesh>,
    rail: Handle<Mesh>,
    rung: Handle<Mesh>,
    barrel: Handle<Mesh>,
    crate_box: Handle<Mesh>,
    console: Handle<Mesh>,
    door: Handle<Mesh>,
    door_post: Handle<Mesh>,
    beacon: Handle<Mesh>,
    zone_tile: Handle<Mesh>,
    supply: Handle<Mesh>,
    supply_lid: Handle<Mesh>,
    supply_ammo: Handle<StandardMaterial>,
    supply_ammo_lid: Handle<StandardMaterial>,
    supply_medkit: Handle<StandardMaterial>,
    supply_grenade: Handle<StandardMaterial>,
    floor_a: Handle<StandardMaterial>,
    floor_b: Handle<StandardMaterial>,
    low_mat: Handle<StandardMaterial>,
    high_mat: Handle<StandardMaterial>,
    ladder_mat: Handle<StandardMaterial>,
    barrel_mat: Handle<StandardMaterial>,
    crate_mat: Handle<StandardMaterial>,
    console_mat: Handle<StandardMaterial>,
    door_mat: Handle<StandardMaterial>,
    locked_mat: Handle<StandardMaterial>,
    beacon_mat: Handle<StandardMaterial>,
    zone_evac: Handle<StandardMaterial>,
    zone_objective: Handle<StandardMaterial>,
    zone_danger: Handle<StandardMaterial>,
}

fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let t = TILE_SIZE;
    // Supplies glow faintly so they read from far away.
    let supply_ammo = materials.add(StandardMaterial {
        base_color: Color::srgb(0.32, 0.4, 0.2),
        emissive: LinearRgba::rgb(0.25, 0.35, 0.05),
        ..default()
    });
    let supply_ammo_lid = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.8, 0.3),
        emissive: LinearRgba::rgb(0.9, 0.7, 0.1),
        ..default()
    });
    let supply_medkit = materials.add(StandardMaterial {
        base_color: Color::srgb(0.92, 0.92, 0.9),
        emissive: LinearRgba::rgb(0.9, 0.12, 0.1),
        ..default()
    });
    let supply_grenade = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.27, 0.22),
        emissive: LinearRgba::rgb(0.8, 0.35, 0.05),
        ..default()
    });
    let mut mat = |color: Color, rough: f32| materials.add(StandardMaterial { base_color: color, perceptual_roughness: rough, ..default() });
    let assets = BoardAssets {
        // The top face of a tile (4 vertices: big maps have a quarter of a
        // million), slightly smaller than the tile so grid lines show.
        floor: meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(t * 0.48)).mesh()),
        // Unit-height column, scaled per tile to its elevation.
        column: meshes.add(Cuboid::new(t, 1.0, t)),
        low: meshes.add(Cuboid::new(t * 0.9, theme::LOW_COVER_HEIGHT, t * 0.9)),
        high: meshes.add(Cuboid::new(t * 0.95, theme::HIGH_COVER_HEIGHT, t * 0.95)),
        rail: meshes.add(Cuboid::new(0.05, 1.0, 0.05)),
        rung: meshes.add(Cuboid::new(0.4, 0.04, 0.04)),
        barrel: meshes.add(Cylinder::new(0.3, 0.7)),
        crate_box: meshes.add(Cuboid::new(0.75, 0.6, 0.75)),
        console: meshes.add(Cuboid::new(0.8, 0.9, 0.4)),
        door: meshes.add(Cuboid::new(t * 0.9, theme::HIGH_COVER_HEIGHT * 0.95, t * 0.9)),
        door_post: meshes.add(Cuboid::new(0.12, theme::HIGH_COVER_HEIGHT, 0.12)),
        beacon: meshes.add(Cylinder::new(0.35, 1.4)),
        zone_tile: meshes.add(Cuboid::new(t * 0.9, 0.03, t * 0.9)),
        supply: meshes.add(Cuboid::new(0.55, 0.36, 0.4)),
        supply_lid: meshes.add(Cuboid::new(0.58, 0.04, 0.43)),
        supply_ammo,
        supply_ammo_lid,
        supply_medkit,
        supply_grenade,
        floor_a: mat(theme::FLOOR_A, 0.9),
        floor_b: mat(theme::FLOOR_B, 0.9),
        low_mat: mat(theme::LOW_COVER, 0.7),
        high_mat: mat(theme::HIGH_COVER, 0.6),
        ladder_mat: mat(Color::srgb(0.85, 0.7, 0.2), 0.5),
        barrel_mat: mat(Color::srgb(0.75, 0.2, 0.12), 0.5),
        crate_mat: mat(Color::srgb(0.6, 0.48, 0.3), 0.8),
        console_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.25, 0.3),
            emissive: LinearRgba::rgb(0.1, 0.6, 0.8),
            ..default()
        }),
        door_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.45, 0.5),
            emissive: LinearRgba::rgb(0.05, 0.25, 0.3),
            ..default()
        }),
        locked_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.2, 0.15),
            emissive: LinearRgba::rgb(0.4, 0.06, 0.03),
            ..default()
        }),
        zone_evac: zone_material(&mut materials, theme::ZONE_EVAC),
        zone_objective: zone_material(&mut materials, theme::ZONE_OBJECTIVE),
        zone_danger: zone_material(&mut materials, theme::ZONE_DANGER),
        beacon_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.8, 0.5),
            emissive: LinearRgba::rgb(0.2, 1.2, 0.5),
            ..default()
        }),
    };
    commands.insert_resource(assets);
}

/// Glowing, see-through floor tint for marked zones.
fn zone_material(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
    let lin = color.to_linear();
    materials.add(StandardMaterial {
        base_color: color.with_alpha(0.45),
        emissive: LinearRgba::rgb(lin.red * 0.9, lin.green * 0.9, lin.blue * 0.9),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    })
}

/// Tiles per side of a board chunk. The board is built (and rebuilt after
/// changes) a chunk at a time, and each chunk merges its static geometry
/// into one mesh per material, so huge maps stay cheap to draw.
pub const CHUNK: i32 = 16;

/// A built chunk (the parent of everything drawn on its tiles).
#[derive(Component)]
pub struct BoardChunk(pub IVec2);

/// Chunks waiting to be (re)built and the ones on screen.
#[derive(Resource, Default)]
struct BoardChunks {
    built: std::collections::HashMap<IVec2, Entity>,
    pending: std::collections::BTreeSet<(i32, i32)>,
}

fn chunk_of(p: GridPos) -> IVec2 {
    IVec2::new(p.x.div_euclid(CHUNK), p.y.div_euclid(CHUNK))
}

/// Everything static on a chunk's tiles, gathered for merging.
#[derive(Default)]
struct ChunkParts {
    merge: Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)>,
}

impl ChunkParts {
    fn add(&mut self, mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, transform: Transform) {
        self.merge.push((mesh.clone(), material.clone(), transform));
    }
    fn piece(&mut self, piece: crate::scenery::Piece) {
        self.merge.push((piece.mesh, piece.material, piece.transform));
    }
}

/// Builds one chunk: its parent entity, merged static meshes, and separate
/// entities for what moves or changes on its own (cutaway walls, doors).
#[allow(clippy::too_many_arguments)]
fn build_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    assets: &BoardAssets,
    scenery: &crate::scenery::Scenery,
    battle: &tactics_core::battle::Battle,
    props: &std::collections::HashMap<GridPos, usize>,
    chunk: IVec2,
) -> Entity {
    let map = battle.map();
    let parent = commands.spawn((BoardEntity, BoardChunk(chunk), Transform::IDENTITY, Visibility::default())).id();
    let mut parts = ChunkParts::default();
    let x0 = chunk.x * CHUNK;
    let y0 = chunk.y * CHUNK;
    for y in y0..(y0 + CHUNK).min(map.height()) {
        for x in x0..(x0 + CHUNK).min(map.width()) {
            tile_parts(commands, &mut parts, parent, assets, scenery, battle, props, GridPos::new(x, y));
        }
    }
    // Merge per material (meshes with other vertex layouts stay separate).
    let mut merged: Vec<(Handle<StandardMaterial>, Mesh)> = Vec::new();
    for (mesh, material, transform) in parts.merge {
        let Some(m) = meshes.get(&mesh) else { continue };
        let Ok(mut m) = m.clone().try_transformed_by(transform) else {
            commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), transform, ChildOf(parent)));
            continue;
        };
        if let Some(bevy::mesh::Indices::U16(v)) = m.indices() {
            let wide: Vec<u32> = v.iter().map(|&i| i as u32).collect();
            m.insert_indices(bevy::mesh::Indices::U32(wide));
        }
        match merged.iter_mut().find(|(mat, _)| *mat == material) {
            Some((_, acc)) => {
                if acc.merge(&m).is_err() {
                    merged.push((material, m));
                }
            }
            None => merged.push((material, m)),
        }
    }
    for (material, mesh) in merged {
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), Transform::IDENTITY, ChildOf(parent)));
    }
    parent
}

/// The drawable parts of one tile.
#[allow(clippy::too_many_arguments)]
fn tile_parts(
    commands: &mut Commands,
    parts: &mut ChunkParts,
    parent: Entity,
    assets: &BoardAssets,
    scenery: &crate::scenery::Scenery,
    battle: &tactics_core::battle::Battle,
    props: &std::collections::HashMap<GridPos, usize>,
    pos: GridPos,
) {
    let map = battle.map();
    let prop_at = |p: GridPos| props.get(&p).map(|i| &battle.props()[*i]);
    {
        let terrain = map.terrain(pos);
        let surface = map.surface(pos);
        if terrain == Terrain::Void {
            // Cut-outs are not drawn, except rivers, which get water.
            if surface == Surface::Water {
                parts.piece(scenery.water(grid_to_world(pos)));
            }
            return;
        }
        let top = tile_top(map, pos);
        let level = map.elevation(pos);

        // Checkerboard floor (in the surface's colours) keeps distances readable.
        let floor_mat = scenery
            .floor(surface, pos)
            .unwrap_or_else(|| if (pos.x + pos.y) % 2 == 0 { assets.floor_a.clone() } else { assets.floor_b.clone() });
        parts.add(&assets.floor, &floor_mat, Transform::from_translation(top));
        if let Some(detail) = scenery.floor_detail(surface, top) {
            parts.piece(detail);
        }

        // Raised ground: a solid column down to level 0, looking like what it is.
        if level > 0 {
            let h = level as f32 * LEVEL_HEIGHT;
            parts.add(
                &assets.column,
                &scenery.column(surface, pos),
                Transform::from_translation(grid_to_world(pos) + Vec3::Y * (h / 2.0 - 0.1)).with_scale(Vec3::new(1.0, h, 1.0)),
            );
        }

        if map.is_ladder(pos) {
            ladder_parts(parts, assets, map, pos);
        }

        // Marked zones (evac pads, objectives): a glowing tint on the floor.
        if let Some(zone) = battle.zones().iter().find(|z| z.visible && z.contains(pos)) {
            let mat = match zone.kind.as_str() {
                "evac" => &assets.zone_evac,
                "danger" => &assets.zone_danger,
                _ => &assets.zone_objective,
            };
            parts.add(&assets.zone_tile, mat, Transform::from_translation(top + Vec3::Y * 0.02));
        }

        if let Some(prop) = prop_at(pos) {
            // Doors: a panel while closed (red when locked), two posts when open.
            if let Some(door) = prop.door {
                if door.open {
                    for side in [-0.42, 0.42] {
                        let offset = Vec3::new(side, theme::HIGH_COVER_HEIGHT / 2.0, side);
                        parts.add(&assets.door_post, &assets.door_mat, Transform::from_translation(top + offset));
                    }
                } else {
                    let mat = if door.locked { &assets.locked_mat } else { &assets.door_mat };
                    commands.spawn((
                        Mesh3d(assets.door.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::from_translation(top + Vec3::Y * theme::HIGH_COVER_HEIGHT * 0.475),
                        Name::new(format!("door:{}", prop.tag)),
                        crate::cutaway::Occluder::new(top, theme::HIGH_COVER_HEIGHT * 0.95),
                        ChildOf(parent),
                    ));
                }
                return;
            }
            if prop.kind == "car" {
                for piece in scenery.car(top, pos) {
                    parts.piece(piece);
                }
                return;
            }
            // Supply caches: small glowing boxes, easy to spot.
            if let Some(mat) = match prop.kind.as_str() {
                "supply_ammo" => Some(&assets.supply_ammo),
                "supply_medkit" => Some(&assets.supply_medkit),
                "supply_grenade" => Some(&assets.supply_grenade),
                _ => None,
            } {
                parts.add(&assets.supply, mat, Transform::from_translation(top + Vec3::Y * 0.18));
                parts.add(&assets.supply_lid, &assets.supply_ammo_lid, Transform::from_translation(top + Vec3::Y * 0.37));
                return;
            }
            let (mesh, mat, h) = match prop.kind.as_str() {
                "beacon" | "uplink" => (&assets.beacon, &assets.beacon_mat, 1.4),
                "barrel" => (&assets.barrel, &assets.barrel_mat, 0.7),
                "console" | "terminal" => (&assets.console, &assets.console_mat, 0.9),
                _ if prop.cover == CoverLevel::Full => (&assets.high, &assets.high_mat, theme::HIGH_COVER_HEIGHT),
                _ => (&assets.crate_box, &assets.crate_mat, 0.6),
            };
            let transform = Transform::from_translation(top + Vec3::Y * h / 2.0);
            if h > theme::UNIT_HEIGHT {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    transform,
                    Name::new(format!("prop:{}", prop.tag)),
                    crate::cutaway::Occluder::new(top, h),
                    ChildOf(parent),
                ));
            } else {
                parts.add(mesh, mat, transform);
            }
            return;
        }

        // Styled cover (brick walls, containers, trees...).
        if matches!(terrain, Terrain::LowCover | Terrain::HighCover)
            && let Some((pieces, height)) = scenery.cover(terrain, surface, top, pos)
        {
            if height > theme::UNIT_HEIGHT {
                // Tall: its own entity, so the cutaway can lower it (anchored
                // at its base, height 0: scaled from the ground).
                let wall = commands
                    .spawn((Transform::from_translation(top), Visibility::default(), crate::cutaway::Occluder::new(top, 0.0), ChildOf(parent)))
                    .id();
                for mut piece in pieces {
                    piece.transform.translation -= top;
                    commands.spawn((Mesh3d(piece.mesh), MeshMaterial3d(piece.material), piece.transform, ChildOf(wall)));
                }
            } else {
                for piece in pieces {
                    parts.piece(piece);
                }
            }
            return;
        }

        let block = match terrain {
            Terrain::LowCover => Some((&assets.low, &assets.low_mat, theme::LOW_COVER_HEIGHT)),
            Terrain::HighCover => Some((&assets.high, &assets.high_mat, theme::HIGH_COVER_HEIGHT)),
            _ => None,
        };
        if let Some((mesh, mat, h)) = block {
            let transform = Transform::from_translation(top + Vec3::Y * h / 2.0);
            if h > theme::UNIT_HEIGHT {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), transform, crate::cutaway::Occluder::new(top, h), ChildOf(parent)));
            } else {
                parts.add(mesh, mat, transform);
            }
        }
    }
}

/// A ladder on `pos`, leaning against its highest neighbour.
fn ladder_parts(parts: &mut ChunkParts, assets: &BoardAssets, map: &tactics_core::grid::Map, pos: GridPos) {
    let level = map.elevation(pos);
    let (dir, high) = Direction::CARDINAL
        .iter()
        .map(|d| (*d, map.elevation(pos.step(*d))))
        .max_by_key(|(_, e)| *e)
        .unwrap_or((Direction::North, level));
    // Span from the lower of the two tiles to the higher.
    let (lo, hi) = (level.min(high), level.max(high).max(level + 1));
    let height = (hi - lo) as f32 * LEVEL_HEIGHT + 0.3;
    let (dx, dy) = dir.delta();
    // Against the edge facing the high neighbour (grid y -> world -z).
    let edge = Vec3::new(dx as f32, 0.0, -dy as f32) * 0.42;
    let base = grid_to_world(pos) + edge + Vec3::Y * (lo as f32 * LEVEL_HEIGHT);
    let frame = Transform::from_translation(base).with_rotation(Quat::from_rotation_y((dx as f32).atan2(-dy as f32)));
    for side in [-0.2, 0.2] {
        let rail = Transform::from_xyz(side, height / 2.0, 0.0).with_scale(Vec3::new(1.0, height, 1.0));
        parts.add(&assets.rail, &assets.ladder_mat, frame * rail);
    }
    let rungs = (height / 0.25) as i32;
    for i in 1..=rungs {
        parts.add(&assets.rung, &assets.ladder_mat, frame * Transform::from_xyz(0.0, i as f32 * 0.25, 0.0));
    }
}

fn spawn_lights(mut commands: Commands, state: Res<BattleState>) {
    let map = state.battle.map();
    let center = grid_to_world(GridPos::new(map.width() / 2, map.height() / 2));
    commands.insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 1.0), brightness: 350.0, ..default() });
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_translation(center + Vec3::new(-8.0, 20.0, 10.0)).looking_at(center, Vec3::Y),
    ));
}

/// Queues chunks for (re)building: all of them after a reset, the touched
/// ones (and their neighbours, for ladders and cliff edges) after a change.
fn queue_chunks(
    mut commands: Commands,
    mut resets: MessageReader<BattleReset>,
    mut dirty: MessageReader<BoardDirty>,
    mut chunks: ResMut<BoardChunks>,
    old: Query<Entity, With<BoardEntity>>,
    state: Res<BattleState>,
    mut first: Local<bool>,
) {
    let map = state.battle.map();
    let all = |chunks: &mut BoardChunks| {
        for cy in 0..(map.height() + CHUNK - 1) / CHUNK {
            for cx in 0..(map.width() + CHUNK - 1) / CHUNK {
                chunks.pending.insert((cx, cy));
            }
        }
    };
    if resets.read().count() > 0 || !*first {
        *first = true;
        for e in &old {
            commands.entity(e).despawn();
        }
        chunks.built.clear();
        chunks.pending.clear();
        all(&mut chunks);
    }
    for BoardDirty(at) in dirty.read() {
        match at {
            None => all(&mut chunks),
            Some(p) => {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let c = chunk_of(GridPos::new(p.x + dx, p.y + dy));
                        if c.x >= 0 && c.y >= 0 {
                            chunks.pending.insert((c.x, c.y));
                        }
                    }
                }
            }
        }
    }
}

/// Builds pending chunks, nearest to the camera first, a few milliseconds'
/// worth per frame (big maps fill in around the view).
#[allow(clippy::too_many_arguments)]
fn build_chunks(
    mut commands: Commands,
    mut chunks: ResMut<BoardChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<BoardAssets>,
    scenery: Res<crate::scenery::Scenery>,
    state: Res<BattleState>,
    rig: Option<Single<&crate::camera::CameraRig>>,
) {
    if chunks.pending.is_empty() {
        return;
    }
    let battle = &state.battle;
    let props: std::collections::HashMap<GridPos, usize> =
        battle.props().iter().enumerate().filter(|(_, p)| p.alive).map(|(i, p)| (p.pos, i)).collect();
    let focus = rig.map_or(Vec3::ZERO, |r| r.focus);
    let near = crate::coords::world_to_grid(focus);
    let mut order: Vec<(i32, i32)> = chunks.pending.iter().copied().collect();
    let dist = |c: &(i32, i32)| {
        let (mx, my) = (c.0 * CHUNK + CHUNK / 2, c.1 * CHUNK + CHUNK / 2);
        (mx - near.x).abs().max((my - near.y).abs())
    };
    order.sort_by_key(|c| (dist(c), *c));
    let started = bevy::platform::time::Instant::now();
    for c in order {
        if started.elapsed().as_secs_f32() > 0.008 {
            break;
        }
        chunks.pending.remove(&c);
        let key = IVec2::new(c.0, c.1);
        if let Some(old) = chunks.built.remove(&key) {
            commands.entity(old).despawn();
        }
        let e = build_chunk(&mut commands, &mut meshes, &assets, &scenery, battle, &props, key);
        chunks.built.insert(key, e);
    }
}

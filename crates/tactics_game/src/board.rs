//! Level geometry: floor tiles at their height, cliff columns, cover blocks,
//! ladders and props.
//!
//! Built from the core [`Map`](tactics_core::grid::Map) and the battle's
//! props with primitive meshes. Cut-out tiles (`Terrain::Void`) are simply not
//! drawn. The board is rebuilt when a battle starts ([`BattleReset`]) and when
//! scripts or explosions change the level ([`BoardDirty`]).
//!
//! To use real art, replace [`spawn_board`] with code that spawns glTF scenes
//! per terrain/prop kind — nothing else depends on how the board looks.

use bevy::prelude::*;
use tactics_core::grid::{CoverLevel, Direction, GridPos, Terrain};

use crate::coords::{LEVEL_HEIGHT, TILE_SIZE, grid_to_world, tile_top};
use crate::state::{BattleReset, BattleState};
use crate::theme;

pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BoardDirty>()
            .add_systems(Startup, (setup_assets, spawn_board, spawn_lights).chain())
            .add_systems(Update, rebuild_board);
    }
}

/// Ask for the board to be rebuilt (terrain, height or props changed).
#[derive(Message, Debug, Clone, Copy)]
pub struct BoardDirty;

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
    floor_a: Handle<StandardMaterial>,
    floor_b: Handle<StandardMaterial>,
    cliff: Handle<StandardMaterial>,
    low_mat: Handle<StandardMaterial>,
    high_mat: Handle<StandardMaterial>,
    ladder_mat: Handle<StandardMaterial>,
    barrel_mat: Handle<StandardMaterial>,
    crate_mat: Handle<StandardMaterial>,
    console_mat: Handle<StandardMaterial>,
    door_mat: Handle<StandardMaterial>,
    locked_mat: Handle<StandardMaterial>,
    beacon_mat: Handle<StandardMaterial>,
}

fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let t = TILE_SIZE;
    let mut mat = |color: Color, rough: f32| materials.add(StandardMaterial { base_color: color, perceptual_roughness: rough, ..default() });
    let assets = BoardAssets {
        // Slightly smaller than a tile so grid lines show between tiles.
        floor: meshes.add(Cuboid::new(t * 0.96, 0.1, t * 0.96)),
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
        floor_a: mat(theme::FLOOR_A, 0.9),
        floor_b: mat(theme::FLOOR_B, 0.9),
        cliff: mat(Color::srgb(0.16, 0.17, 0.19), 0.95),
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
        beacon_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.8, 0.5),
            emissive: LinearRgba::rgb(0.2, 1.2, 0.5),
            ..default()
        }),
    };
    commands.insert_resource(assets);
}

fn spawn_board(mut commands: Commands, assets: Res<BoardAssets>, state: Res<BattleState>) {
    let battle = &state.battle;
    let map = battle.map();
    // Tiles covered by a living prop draw the prop instead of a cover block.
    let prop_at = |p: GridPos| battle.props().iter().find(|pr| pr.alive && pr.pos == p);

    for pos in map.positions() {
        let terrain = map.terrain(pos);
        if terrain == Terrain::Void {
            continue; // Cut-outs are simply not drawn.
        }
        let top = tile_top(map, pos);
        let level = map.elevation(pos);

        // Checkerboard floor makes distances readable at a glance.
        let floor_mat = if (pos.x + pos.y) % 2 == 0 { &assets.floor_a } else { &assets.floor_b };
        commands.spawn((BoardEntity, Mesh3d(assets.floor.clone()), MeshMaterial3d(floor_mat.clone()), Transform::from_translation(top - Vec3::Y * 0.05)));

        // Raised ground: a solid column down to level 0.
        if level > 0 {
            let h = level as f32 * LEVEL_HEIGHT;
            commands.spawn((
                BoardEntity,
                Mesh3d(assets.column.clone()),
                MeshMaterial3d(assets.cliff.clone()),
                Transform::from_translation(grid_to_world(pos) + Vec3::Y * (h / 2.0 - 0.1)).with_scale(Vec3::new(1.0, h, 1.0)),
            ));
        }

        if map.is_ladder(pos) {
            spawn_ladder(&mut commands, &assets, map, pos);
        }

        if let Some(prop) = prop_at(pos) {
            // Doors: a panel while closed (red when locked), two posts when open.
            if let Some(door) = prop.door {
                if door.open {
                    for side in [-0.42, 0.42] {
                        let offset = Vec3::new(side, theme::HIGH_COVER_HEIGHT / 2.0, side);
                        commands.spawn((
                            BoardEntity,
                            Mesh3d(assets.door_post.clone()),
                            MeshMaterial3d(assets.door_mat.clone()),
                            Transform::from_translation(top + offset),
                        ));
                    }
                } else {
                    let mat = if door.locked { &assets.locked_mat } else { &assets.door_mat };
                    commands.spawn((
                        BoardEntity,
                        Mesh3d(assets.door.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::from_translation(top + Vec3::Y * theme::HIGH_COVER_HEIGHT * 0.475),
                        Name::new(format!("door:{}", prop.tag)),
                    ));
                }
                continue;
            }
            let (mesh, mat, h) = match prop.kind.as_str() {
                "beacon" | "uplink" => (&assets.beacon, &assets.beacon_mat, 1.4),
                "barrel" => (&assets.barrel, &assets.barrel_mat, 0.7),
                "console" | "terminal" => (&assets.console, &assets.console_mat, 0.9),
                _ if prop.cover == CoverLevel::Full => (&assets.high, &assets.high_mat, theme::HIGH_COVER_HEIGHT),
                _ => (&assets.crate_box, &assets.crate_mat, 0.6),
            };
            commands.spawn((
                BoardEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(top + Vec3::Y * h / 2.0),
                Name::new(format!("prop:{}", prop.tag)),
            ));
            continue;
        }

        let block = match terrain {
            Terrain::LowCover => Some((&assets.low, &assets.low_mat, theme::LOW_COVER_HEIGHT)),
            Terrain::HighCover => Some((&assets.high, &assets.high_mat, theme::HIGH_COVER_HEIGHT)),
            _ => None,
        };
        if let Some((mesh, mat, h)) = block {
            commands.spawn((BoardEntity, Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), Transform::from_translation(top + Vec3::Y * h / 2.0)));
        }
    }
}

/// A ladder on `pos`, leaning against its highest neighbour.
fn spawn_ladder(commands: &mut Commands, assets: &BoardAssets, map: &tactics_core::grid::Map, pos: GridPos) {
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
    let rotation = Quat::from_rotation_y((dx as f32).atan2(-dy as f32));
    let parent = commands.spawn((BoardEntity, Transform::from_translation(base).with_rotation(rotation), Visibility::default())).id();
    for side in [-0.2, 0.2] {
        commands.spawn((
            Mesh3d(assets.rail.clone()),
            MeshMaterial3d(assets.ladder_mat.clone()),
            Transform::from_xyz(side, height / 2.0, 0.0).with_scale(Vec3::new(1.0, height, 1.0)),
            ChildOf(parent),
        ));
    }
    let rungs = (height / 0.25) as i32;
    for i in 1..=rungs {
        commands.spawn((
            Mesh3d(assets.rung.clone()),
            MeshMaterial3d(assets.ladder_mat.clone()),
            Transform::from_xyz(0.0, i as f32 * 0.25, 0.0),
            ChildOf(parent),
        ));
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

/// Despawns and recreates the board after a reset or a level change.
fn rebuild_board(
    mut commands: Commands,
    mut resets: MessageReader<BattleReset>,
    mut dirty: MessageReader<BoardDirty>,
    old: Query<Entity, With<BoardEntity>>,
    assets: Res<BoardAssets>,
    state: Res<BattleState>,
) {
    // Read both (so neither piles up), rebuild once if either arrived.
    let changed = resets.read().count() + dirty.read().count() > 0;
    if !changed {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    spawn_board(commands, assets, state);
}

//! Static level geometry: floor tiles and cover blocks.
//!
//! Built from the core [`Map`](tactics_core::grid::Map) with primitive meshes.
//! To use real art, replace [`spawn_board`] with code that spawns glTF scenes
//! per terrain type — nothing else depends on how the board looks.

use bevy::prelude::*;
use tactics_core::grid::{GridPos, Terrain};

use crate::coords::{TILE_SIZE, grid_to_world};
use crate::state::{BattleReset, BattleState};
use crate::theme;

pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_assets, spawn_board, spawn_lights).chain())
            .add_systems(Update, rebuild_on_reset.run_if(on_message::<BattleReset>));
    }
}

/// Marks every entity belonging to the level so it can be cleared on restart.
#[derive(Component)]
pub struct BoardEntity;

/// Meshes and materials shared by all tiles (one GPU buffer per kind, not per tile).
#[derive(Resource)]
struct BoardAssets {
    floor: Handle<Mesh>,
    low: Handle<Mesh>,
    high: Handle<Mesh>,
    floor_a: Handle<StandardMaterial>,
    floor_b: Handle<StandardMaterial>,
    low_mat: Handle<StandardMaterial>,
    high_mat: Handle<StandardMaterial>,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = TILE_SIZE;
    let mat = |materials: &mut Assets<StandardMaterial>, color: Color, rough: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: rough,
            ..default()
        })
    };
    commands.insert_resource(BoardAssets {
        // Slightly smaller than a tile so grid lines show between tiles.
        floor: meshes.add(Cuboid::new(t * 0.96, 0.1, t * 0.96)),
        low: meshes.add(Cuboid::new(t * 0.9, theme::LOW_COVER_HEIGHT, t * 0.9)),
        high: meshes.add(Cuboid::new(t * 0.95, theme::HIGH_COVER_HEIGHT, t * 0.95)),
        floor_a: mat(&mut materials, theme::FLOOR_A, 0.9),
        floor_b: mat(&mut materials, theme::FLOOR_B, 0.9),
        low_mat: mat(&mut materials, theme::LOW_COVER, 0.7),
        high_mat: mat(&mut materials, theme::HIGH_COVER, 0.6),
    });
}

fn spawn_board(mut commands: Commands, assets: Res<BoardAssets>, state: Res<BattleState>) {
    let map = state.battle.map();
    for pos in map.positions() {
        let base = grid_to_world(pos);
        let terrain = map.terrain(pos);
        if terrain == Terrain::Void {
            continue; // Holes in the map are simply not drawn.
        }
        // Checkerboard floor makes distances readable at a glance.
        let floor_mat = if (pos.x + pos.y) % 2 == 0 { &assets.floor_a } else { &assets.floor_b };
        commands.spawn((
            BoardEntity,
            Mesh3d(assets.floor.clone()),
            MeshMaterial3d(floor_mat.clone()),
            Transform::from_translation(base - Vec3::Y * 0.05),
        ));
        let block = match terrain {
            Terrain::LowCover => Some((&assets.low, &assets.low_mat, theme::LOW_COVER_HEIGHT)),
            Terrain::HighCover => Some((&assets.high, &assets.high_mat, theme::HIGH_COVER_HEIGHT)),
            _ => None,
        };
        if let Some((mesh, mat, h)) = block {
            commands.spawn((
                BoardEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(base + Vec3::Y * h / 2.0),
            ));
        }
    }
}

fn spawn_lights(mut commands: Commands, state: Res<BattleState>) {
    let map = state.battle.map();
    let center = grid_to_world(GridPos::new(map.width() / 2, map.height() / 2));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.75, 0.8, 1.0),
        brightness: 350.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(center + Vec3::new(-8.0, 20.0, 10.0)).looking_at(center, Vec3::Y),
    ));
}

/// Despawns and recreates the board when a new battle starts.
fn rebuild_on_reset(
    mut commands: Commands,
    old: Query<Entity, With<BoardEntity>>,
    assets: Res<BoardAssets>,
    state: Res<BattleState>,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    spawn_board(commands, assets, state);
}

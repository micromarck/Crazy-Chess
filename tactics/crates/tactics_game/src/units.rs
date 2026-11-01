//! Unit visuals: one entity tree per core unit.
//!
//! The mapping [`UnitEntities`] goes from core [`UnitId`] to the Bevy entity,
//! so animation code can find the right mesh for an event. Positions are
//! driven by [`playback`](crate::playback) while animating and snapped back to
//! the authoritative grid position whenever playback is idle.

use std::collections::HashMap;

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::coords::grid_to_world;
use crate::playback::Playback;
use crate::state::{BattleReset, BattleState};
use crate::theme;

pub struct UnitsPlugin;

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnitEntities>()
            .add_systems(Startup, (setup_assets, spawn_units).chain())
            .add_systems(
                Update,
                (
                    respawn_on_reset.run_if(on_message::<BattleReset>),
                    snap_when_idle,
                )
                    .chain()
                    .in_set(FrameSet::Present),
            );
    }
}

/// Links a visual entity to its unit in the core battle.
#[derive(Component, Debug, Clone, Copy)]
pub struct UnitVisual {
    pub id: UnitId,
    pub team: Team,
}

/// `UnitId -> Entity` lookup.
#[derive(Resource, Default)]
pub struct UnitEntities(pub HashMap<UnitId, Entity>);

#[derive(Resource)]
struct UnitAssets {
    body: Handle<Mesh>,
    head: Handle<Mesh>,
    big_head: Handle<Mesh>,
    gun: Handle<Mesh>,
    player: Handle<StandardMaterial>,
    enemy: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(UnitAssets {
        body: meshes.add(Capsule3d::new(0.22, theme::UNIT_HEIGHT - 0.44 - 0.3)),
        head: meshes.add(Sphere::new(0.17)),
        big_head: meshes.add(Sphere::new(0.27)),
        gun: meshes.add(Cuboid::new(0.08, 0.08, 0.5)),
        player: materials.add(StandardMaterial { base_color: theme::PLAYER, perceptual_roughness: 0.5, ..default() }),
        enemy: materials.add(StandardMaterial { base_color: theme::ENEMY, perceptual_roughness: 0.4, ..default() }),
        dark: materials.add(StandardMaterial { base_color: Color::srgb(0.1, 0.1, 0.12), ..default() }),
    });
}

/// Spawns the visual hierarchy for one unit. Children: head and gun.
fn spawn_unit(commands: &mut Commands, assets: &UnitAssets, unit: &Unit) -> Entity {
    // Team decides the colour; the class's `model` hint decides the shape.
    let mat = match unit.team {
        Team::Player => &assets.player,
        Team::Enemy => &assets.enemy,
    };
    let head = if unit.model == "alien" { &assets.big_head } else { &assets.head };
    let body_h = theme::UNIT_HEIGHT - 0.3;
    commands
        .spawn((
            UnitVisual { id: unit.id, team: unit.team },
            Name::new(unit.name.clone()),
            Transform::from_translation(grid_to_world(unit.pos)),
            Visibility::default(),
            children![
                (
                    Mesh3d(assets.body.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, body_h / 2.0, 0.0),
                ),
                (
                    Mesh3d(head.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, body_h + 0.12, 0.0),
                ),
                (
                    // The gun points along local -Z ("forward"), so facing the
                    // target is just `Transform::look_at`.
                    Mesh3d(assets.gun.clone()),
                    MeshMaterial3d(assets.dark.clone()),
                    Transform::from_xyz(0.2, body_h * 0.6, -0.25),
                ),
            ],
        ))
        .id()
}

fn spawn_units(
    mut commands: Commands,
    assets: Res<UnitAssets>,
    state: Res<BattleState>,
    mut entities: ResMut<UnitEntities>,
) {
    entities.0.clear();
    for unit in state.battle.units().iter().filter(|u| u.is_alive()) {
        let e = spawn_unit(&mut commands, &assets, unit);
        entities.0.insert(unit.id, e);
    }
}

fn respawn_on_reset(
    mut commands: Commands,
    assets: Res<UnitAssets>,
    state: Res<BattleState>,
    entities: ResMut<UnitEntities>,
    old: Query<Entity, With<UnitVisual>>,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    spawn_units(commands, assets, state, entities);
}

/// When nothing is animating, make every visual match the authoritative state.
/// This self-heals any drift (e.g. an animation skipped by a restart).
fn snap_when_idle(
    playback: Res<Playback>,
    state: Res<BattleState>,
    mut q: Query<(&UnitVisual, &mut Transform)>,
) {
    if playback.is_busy() {
        return;
    }
    for (visual, mut transform) in &mut q {
        if let Some(unit) = state.battle.unit(visual.id)
            && unit.is_alive() {
                transform.translation = grid_to_world(unit.pos);
            }
    }
}

//! Unit visuals: one entity tree per core unit.
//!
//! ```text
//! unit root   UnitVisual, UnitAnimator, Transform = tile position + facing
//! └─ pivot    UnitPivot, Transform = procedural animation pose
//!    ├─ fallback primitive (until/unless a glTF scene loads)
//!    └─ glTF scene (WorldAssetRoot) with its AnimationPlayer
//! ```
//!
//! [`UnitEntities`] maps core [`UnitId`]s to root entities, so playback can
//! find the right unit for an event. Positions are driven by
//! [`playback`](crate::playback) while animating and snapped back to the
//! authoritative grid position whenever playback is idle.
//! Which model a unit gets comes from its class's `model` id (see
//! [`models`](crate::models)).

use std::collections::HashMap;

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::animation::{UnitAnimator, UnitPivot};
use crate::coords::tile_top;
use crate::models::{ModelLibrary, PrimitiveAssets, spawn_model};
use crate::playback::Playback;
use crate::state::{BattleReset, BattleState};

pub struct UnitsPlugin;

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnitEntities>()
            .add_message::<SpawnUnitVisual>()
            .add_systems(Startup, spawn_units)
            .add_systems(
                Update,
                (respawn_on_reset.run_if(on_message::<BattleReset>), spawn_new_units, snap_when_idle)
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

/// Create the visual for a unit that appeared mid-battle (script spawns).
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnUnitVisual(pub UnitId);

/// `UnitId -> Entity` lookup.
#[derive(Resource, Default)]
pub struct UnitEntities(pub HashMap<UnitId, Entity>);

/// Spawns the entity tree for one unit.
fn spawn_unit(commands: &mut Commands, library: &ModelLibrary, primitives: &PrimitiveAssets, map: &Map, unit: &Unit) -> Entity {
    let root = commands
        .spawn((
            UnitVisual { id: unit.id, team: unit.team },
            UnitAnimator::new(unit.model.clone()),
            Name::new(unit.name.clone()),
            Transform::from_translation(tile_top(map, unit.pos)),
            Visibility::default(),
        ))
        .id();
    let pivot = commands.spawn((UnitPivot, Transform::default(), Visibility::default(), ChildOf(root))).id();
    spawn_model(commands, library, primitives, &unit.model, unit.team, root, pivot);
    root
}

fn spawn_units(
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    state: Res<BattleState>,
    mut entities: ResMut<UnitEntities>,
) {
    entities.0.clear();
    for unit in state.battle.units().iter().filter(|u| u.is_alive()) {
        let e = spawn_unit(&mut commands, &library, &primitives, state.battle.map(), unit);
        entities.0.insert(unit.id, e);
    }
}

fn respawn_on_reset(
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    state: Res<BattleState>,
    entities: ResMut<UnitEntities>,
    old: Query<Entity, With<UnitVisual>>,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    spawn_units(commands, library, primitives, state, entities);
}

fn spawn_new_units(
    mut requests: MessageReader<SpawnUnitVisual>,
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    state: Res<BattleState>,
    mut entities: ResMut<UnitEntities>,
) {
    for SpawnUnitVisual(id) in requests.read() {
        if entities.0.contains_key(id) {
            continue;
        }
        if let Some(unit) = state.battle.unit(*id).filter(|u| u.is_alive()) {
            let e = spawn_unit(&mut commands, &library, &primitives, state.battle.map(), unit);
            entities.0.insert(*id, e);
        }
    }
}

/// When nothing is animating, make every visual match the authoritative state.
/// This self-heals any drift (e.g. an animation skipped by a restart).
fn snap_when_idle(playback: Res<Playback>, state: Res<BattleState>, mut q: Query<(&UnitVisual, &mut Transform)>) {
    if playback.is_busy() {
        return;
    }
    for (visual, mut transform) in &mut q {
        if let Some(unit) = state.battle.unit(visual.id)
            && unit.is_alive()
        {
            transform.translation = tile_top(state.battle.map(), unit.pos);
        }
    }
}

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
                (
                    respawn_on_reset,
                    spawn_new_units,
                    snap_when_idle,
                    lean_out,
                    turn_units,
                    fog_of_war,
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

/// A world point the unit should turn to face (smoothly, around Y).
/// `None` = leave the rotation alone (e.g. while the death animation runs).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct FaceTarget(pub Option<Vec3>);

/// Turn speed: higher is snappier.
const TURN_RATE: f32 = 12.0;

/// Hides enemies the squad can't currently see, in missions with fog. Uses
/// the visual's current tile, so a walking enemy fades in/out as it crosses
/// the squad's sight lines.
fn fog_of_war(state: Res<BattleState>, mut q: Query<(&UnitVisual, &Transform, &mut Visibility)>) {
    let b = &state.battle;
    for (visual, tr, mut vis) in &mut q {
        let shown = !b.fog()
            || visual.team == Team::Player
            || b.unit(visual.id).is_none_or(|u| !u.is_alive())
            || b.team_can_see(Team::Player, crate::coords::world_to_grid(tr.translation));
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

/// Smoothly turns units towards their [`FaceTarget`].
fn turn_units(time: Res<Time>, mut q: Query<(&FaceTarget, &mut Transform), With<UnitVisual>>) {
    let k = 1.0 - (-TURN_RATE * time.delta_secs()).exp();
    for (face, mut tr) in &mut q {
        let Some(target) = face.0 else { continue };
        let d = target - tr.translation;
        if d.x * d.x + d.z * d.z < 1e-4 {
            continue;
        }
        // Rotation about Y whose forward (-Z) points along d.
        let desired = Quat::from_rotation_y((-d.x).atan2(-d.z));
        tr.rotation = tr.rotation.slerp(desired, k);
    }
}

/// Create the visual for a unit that appeared mid-battle (script spawns).
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnUnitVisual(pub UnitId);

/// `UnitId -> Entity` lookup.
#[derive(Resource, Default)]
pub struct UnitEntities(pub HashMap<UnitId, Entity>);

/// Spawns the entity tree for one unit.
fn spawn_unit(
    commands: &mut Commands,
    library: &ModelLibrary,
    primitives: &PrimitiveAssets,
    looks: &crate::looks::LooksAssets,
    battle: &Battle,
    unit: &Unit,
) -> Entity {
    let map = battle.map();
    let root = commands
        .spawn((
            UnitVisual { id: unit.id, team: unit.team },
            UnitAnimator::new(unit.model.clone()),
            FaceTarget::default(),
            Name::new(unit.name.clone()),
            Transform::from_translation(crate::coords::unit_spot(map, unit)),
            Visibility::default(),
        ))
        .id();
    let pivot = commands.spawn((UnitPivot, Transform::default(), Visibility::default(), ChildOf(root))).id();
    let outfit = crate::looks::Outfit::of(unit, battle.rules());
    spawn_model(commands, library, primitives, looks, &outfit, &unit.model, unit.team, root, pivot);
    root
}

fn spawn_units(
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    looks: Res<crate::looks::LooksAssets>,
    state: Res<BattleState>,
    mut entities: ResMut<UnitEntities>,
) {
    entities.0.clear();
    for unit in state.battle.units().iter().filter(|u| u.is_alive()) {
        let e = spawn_unit(&mut commands, &library, &primitives, &looks, &state.battle, unit);
        entities.0.insert(unit.id, e);
    }
}

/// Rebuilds every unit visual after a battle reset or a model change.
#[allow(clippy::too_many_arguments)]
fn respawn_on_reset(
    mut resets: MessageReader<BattleReset>,
    mut refresh: MessageReader<crate::characters::RefreshUnitVisuals>,
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    looks: Res<crate::looks::LooksAssets>,
    state: Res<BattleState>,
    entities: ResMut<UnitEntities>,
    old: Query<Entity, With<UnitVisual>>,
) {
    // Read both (so neither piles up); rebuild once if either arrived.
    if resets.read().count() + refresh.read().count() == 0 {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    spawn_units(commands, library, primitives, looks, state, entities);
}

fn spawn_new_units(
    mut requests: MessageReader<SpawnUnitVisual>,
    mut commands: Commands,
    library: Res<ModelLibrary>,
    primitives: Res<PrimitiveAssets>,
    looks: Res<crate::looks::LooksAssets>,
    state: Res<BattleState>,
    mut entities: ResMut<UnitEntities>,
) {
    for SpawnUnitVisual(id) in requests.read() {
        if entities.0.contains_key(id) {
            continue;
        }
        if let Some(unit) = state.battle.unit(*id).filter(|u| u.is_alive()) {
            let e = spawn_unit(&mut commands, &library, &primitives, &looks, &state.battle, unit);
            entities.0.insert(*id, e);
        }
    }
}

/// When nothing is animating, make every visual match the authoritative state.
/// This self-heals any drift (e.g. an animation skipped by a restart).
/// Corner peeking: a unit firing from a lean tile (around a wall corner)
/// leans its body part of the way there while the shot plays out. Removed
/// once playback is idle (the unit then snaps back to its tile).
#[derive(Component, Debug, Clone, Copy)]
pub struct Leaning {
    /// World point of the lean tile.
    pub toward: Vec3,
    pub elapsed: f32,
}

/// How far towards the lean tile the body moves (tiles).
const LEAN_FRACTION: f32 = 0.55;

fn lean_out(
    mut commands: Commands,
    time: Res<Time>,
    playback: Res<Playback>,
    state: Res<BattleState>,
    mut q: Query<(Entity, &UnitVisual, &mut Leaning, &mut Transform)>,
) {
    for (e, visual, mut lean, mut transform) in &mut q {
        let Some(unit) = state.battle.unit(visual.id).filter(|u| u.is_alive() && playback.is_busy()) else {
            commands.entity(e).remove::<Leaning>();
            continue;
        };
        lean.elapsed += time.delta_secs();
        let home = crate::coords::unit_spot(state.battle.map(), unit);
        let k = (lean.elapsed / 0.18).min(1.0);
        let k = k * k * (3.0 - 2.0 * k);
        let mut to = lean.toward;
        to.y = home.y;
        transform.translation = home.lerp(to, LEAN_FRACTION * k);
    }
}

fn snap_when_idle(playback: Res<Playback>, state: Res<BattleState>, mut q: Query<(&UnitVisual, &mut Transform)>) {
    if playback.is_busy() {
        return;
    }
    for (visual, mut transform) in &mut q {
        if let Some(unit) = state.battle.unit(visual.id)
            && unit.is_alive()
        {
            transform.translation = crate::coords::unit_spot(state.battle.map(), unit);
        }
    }
}

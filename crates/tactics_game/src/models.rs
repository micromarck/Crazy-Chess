//! Unit models: glTF scenes from modules, with primitive fallbacks.
//!
//! [`ModelLibrary`] is built once from the ruleset's [`ModelDef`]s. For each
//! model it:
//! * starts loading the glTF scene (if any),
//! * loads every animation clip and builds one [`AnimationGraph`] with a node
//!   per clip, so animation slots map to graph nodes.
//!
//! [`spawn_model`] puts a model under a unit's pivot entity. The primitive
//! fallback is spawned immediately; when the glTF scene finishes loading it
//! replaces the fallback and its `AnimationPlayer` is connected to the unit's
//! [`UnitAnimator`](crate::animation::UnitAnimator). If the file is missing,
//! the fallback simply stays, so a broken path never breaks the game.

use std::collections::HashMap;
use std::time::Duration;

use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use tactics_core::content::{ModelDef, PrimitiveShape};
use tactics_core::prelude::*;

use crate::animation::UnitAnimator;
use crate::state::BattleState;
use crate::theme;

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        // PreStartup: the library must exist before units are spawned.
        app.add_systems(PreStartup, (setup_primitives, build_library))
            .add_systems(Update, auto_fit)
            // Late in the frame, before PostUpdate's animation evaluation.
            .add_systems(Update, restore_rest_poses.after(crate::FrameSet::Present));
    }
}

/// One animation slot of a loaded model.
#[derive(Debug, Clone)]
pub struct LoadedClip {
    /// Name of the clip in its file (for the Character Lab).
    pub name: String,
    pub node: AnimationNodeIndex,
    pub clip: Handle<AnimationClip>,
    pub looping: bool,
    pub speed: f32,
}

/// A model ready to spawn.
#[derive(Debug, Clone)]
pub struct LoadedModel {
    pub def: ModelDef,
    pub scene: Option<Handle<WorldAsset>>,
    pub graph: Option<Handle<AnimationGraph>>,
    pub clips: HashMap<String, LoadedClip>,
    /// Scale the glTF scene so it is this tall, with its feet on the ground
    /// (drop-in characters without an explicit scale).
    pub auto_fit: Option<f32>,
    /// Cached auto-fit result: (scale, vertical offset).
    pub fit: Option<(f32, f32)>,
    /// Where the model came from, for the Character Lab.
    pub source: String,
    /// Draw headgear / backpack / gear on the glTF scene too (not just the weapon).
    pub show_gear: bool,
}

/// All models of the ruleset, by id.
#[derive(Resource, Default)]
pub struct ModelLibrary {
    models: HashMap<String, LoadedModel>,
    /// Skeletal rigs for models with a `skeleton`.
    rigs: HashMap<String, std::sync::Arc<crate::rig::RigDef>>,
}

impl ModelLibrary {
    pub fn get(&self, id: &str) -> Option<&LoadedModel> {
        self.models.get(id)
    }

    /// Adds or replaces a model at runtime (drop-in characters).
    pub fn insert(&mut self, id: impl Into<String>, model: LoadedModel) {
        self.models.insert(id.into(), model);
    }

    /// Uses `rig` (e.g. another model's) as this model's stand-in rig.
    pub fn insert_rig(&mut self, id: impl Into<String>, rig: std::sync::Arc<crate::rig::RigDef>) {
        self.rigs.insert(id.into(), rig);
    }

    /// All model ids, sorted.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.models.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Every animation slot a model can show and where it comes from
    /// (`clip 'Walk'`, `skeleton`), sorted by slot.
    pub fn slots(&self, id: &str) -> Vec<(String, String)> {
        let mut out: std::collections::BTreeMap<String, String> = Default::default();
        // For glTF models the skeleton rig is only a stand-in while loading:
        // slots without a clip use the procedural fallback.
        let gltf = self.get(id).is_some_and(|m| m.scene.is_some());
        if let Some(r) = self.rig(id) {
            for slot in r.animations.keys() {
                out.insert(slot.clone(), if gltf { "procedural".into() } else { "skeleton".into() });
            }
        }
        if let Some(m) = self.get(id) {
            for (slot, c) in &m.clips {
                out.insert(slot.clone(), format!("clip {}", c.name));
            }
        }
        out.into_iter().collect()
    }

    /// The rig of a model drawn on a skeleton.
    pub fn rig(&self, id: &str) -> Option<&std::sync::Arc<crate::rig::RigDef>> {
        self.rigs.get(id)
    }

    /// Length of `slot` for model `id`: its glTF clip (once loaded) or its
    /// skeleton animation. `None` = the model has no such animation.
    pub fn animation_length(&self, id: &str, slot: &str, clips: &Assets<AnimationClip>) -> Option<f32> {
        if let Some(c) = self.clip(id, slot) {
            return Some(clips.get(&c.clip).map_or(1.0, |clip| clip.duration() / c.speed.max(0.01)));
        }
        self.rig(id).and_then(|r| r.animations.get(slot)).map(|a| a.duration)
    }

    /// The clip for `slot` of model `id`, if the model provides one.
    pub fn clip(&self, id: &str, slot: &str) -> Option<&LoadedClip> {
        self.models.get(id)?.clips.get(slot)
    }
}

fn build_library(
    mut commands: Commands,
    state: Res<BattleState>,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let mut library = ModelLibrary::default();
    for def in state.battle.rules().models() {
        if let Some(sk) = &def.skeleton {
            match crate::rig::RigDef::build(state.battle.rules(), sk, &mut meshes) {
                Ok(rig) => {
                    library.rigs.insert(def.id.clone(), std::sync::Arc::new(rig));
                }
                Err(e) => warn!("model '{}': {e} (using the primitive fallback)", def.id),
            }
        }
        let scene = def.scene.as_ref().map(|path| assets.load::<WorldAsset>(path.clone()));
        let (graph, clips) = if def.animations.is_empty() {
            (None, HashMap::new())
        } else {
            // Stable order so node indices line up with slot names.
            let slots: Vec<_> = def.animations.iter().collect();
            let handles: Vec<Handle<AnimationClip>> =
                slots.iter().map(|(_, a)| assets.load::<AnimationClip>(a.clip.clone())).collect();
            let (graph, nodes) = AnimationGraph::from_clips(handles.clone());
            let clips = slots
                .iter()
                .zip(nodes)
                .zip(handles)
                .map(|(((slot, a), node), clip)| {
                    ((*slot).clone(), LoadedClip { name: a.clip.clone(), node, clip, looping: a.looping, speed: a.speed })
                })
                .collect();
            (Some(graphs.add(graph)), clips)
        };
        library
            .models
            .insert(def.id.clone(), LoadedModel {
                def: def.clone(),
                source: if def.scene.is_some() { "glTF".into() } else if def.skeleton.is_some() { "skeleton rig".into() } else { "primitive".into() },
                scene,
                graph,
                clips,
                auto_fit: None,
                fit: None,
                show_gear: false,
            });
    }
    commands.insert_resource(library);
}

// ----------------------------------------------------------------------
// Primitive fallbacks
// ----------------------------------------------------------------------

/// Shared meshes/materials for primitive models.
#[derive(Resource)]
pub struct PrimitiveAssets {
    body: Handle<Mesh>,
    head: Handle<Mesh>,
    big_head: Handle<Mesh>,
    gun: Handle<Mesh>,
    drone_body: Handle<Mesh>,
    drone_eye: Handle<Mesh>,
    rotor: Handle<Mesh>,
    player: Handle<StandardMaterial>,
    enemy: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    light: Handle<StandardMaterial>,
    eye: Handle<StandardMaterial>,
}

fn setup_primitives(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mat = |materials: &mut Assets<StandardMaterial>, c: Color, r: f32| {
        materials.add(StandardMaterial { base_color: c, perceptual_roughness: r, ..default() })
    };
    commands.insert_resource(PrimitiveAssets {
        body: meshes.add(Capsule3d::new(0.22, theme::UNIT_HEIGHT - 0.44 - 0.3)),
        head: meshes.add(Sphere::new(0.17)),
        big_head: meshes.add(Sphere::new(0.27)),
        gun: meshes.add(Cuboid::new(0.08, 0.08, 0.5)),
        drone_body: meshes.add(Cuboid::new(0.6, 0.35, 0.6)),
        drone_eye: meshes.add(Cuboid::new(0.2, 0.12, 0.06)),
        rotor: meshes.add(Cuboid::new(0.55, 0.04, 0.12)),
        player: mat(&mut materials, theme::PLAYER, 0.5),
        enemy: mat(&mut materials, theme::ENEMY, 0.4),
        dark: mat(&mut materials, Color::srgb(0.1, 0.1, 0.12), 0.6),
        light: mat(&mut materials, Color::srgb(0.82, 0.8, 0.74), 0.7),
        eye: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.2, 0.1),
            emissive: LinearRgba::rgb(4.0, 0.5, 0.2),
            ..default()
        }),
    });
}

/// Marks the primitive stand-in, removed once a glTF scene is ready.
#[derive(Component)]
pub struct FallbackModel;

/// On a glTF scene entity: the unit root it belongs to and its model id.
#[derive(Component)]
pub struct ModelOwner {
    pub unit_root: Entity,
    pub model: String,
}

/// Spawns the primitive shape as children of `parent`. The model faces -Z.
fn spawn_primitive(commands: &mut Commands, assets: &PrimitiveAssets, shape: PrimitiveShape, team: Team, parent: Entity) {
    let team_mat = match team {
        Team::Player => assets.player.clone(),
        Team::Enemy => assets.enemy.clone(),
    };
    let body_h = theme::UNIT_HEIGHT - 0.3;
    let root = commands.spawn((FallbackModel, Transform::default(), Visibility::default(), ChildOf(parent))).id();
    let mut part = |mesh: &Handle<Mesh>, mat: Handle<StandardMaterial>, t: Transform| {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat), t, ChildOf(root)));
    };
    match shape {
        PrimitiveShape::Soldier | PrimitiveShape::Alien => {
            let head = if shape == PrimitiveShape::Alien { &assets.big_head } else { &assets.head };
            part(&assets.body, team_mat.clone(), Transform::from_xyz(0.0, body_h / 2.0, 0.0));
            part(head, team_mat, Transform::from_xyz(0.0, body_h + 0.12, 0.0));
            part(&assets.gun, assets.dark.clone(), Transform::from_xyz(0.2, body_h * 0.6, -0.25));
        }
        PrimitiveShape::Drone => {
            let y = 0.95;
            part(&assets.drone_body, team_mat, Transform::from_xyz(0.0, y, 0.0));
            part(&assets.drone_eye, assets.eye.clone(), Transform::from_xyz(0.0, y + 0.03, -0.31));
            part(&assets.rotor, assets.dark.clone(), Transform::from_xyz(-0.42, y + 0.22, 0.0));
            part(&assets.rotor, assets.dark.clone(), Transform::from_xyz(0.42, y + 0.22, 0.0));
        }
    }
}

/// Spawns the model `model_id` under `pivot` (a child of `unit_root`),
/// dressed in `outfit` (see [`crate::looks`]).
#[allow(clippy::too_many_arguments)]
pub fn spawn_model(
    commands: &mut Commands,
    library: &ModelLibrary,
    primitives: &PrimitiveAssets,
    looks: &crate::looks::LooksAssets,
    outfit: &crate::looks::Outfit,
    model_id: &str,
    team: Team,
    unit_root: Entity,
    pivot: Entity,
) {
    let Some(model) = library.get(model_id) else {
        // Unknown model id: draw a soldier so the unit is still visible.
        spawn_primitive(commands, primitives, PrimitiveShape::Soldier, team, pivot);
        return;
    };
    // Skeleton rig if the model has one, else the primitive shape. Either is
    // replaced by the glTF scene once that has loaded.
    match library.rig(model_id) {
        Some(rig) => {
            let materials = crate::rig::RigMaterials {
                team: match team {
                    Team::Player => primitives.player.clone(),
                    Team::Enemy => primitives.enemy.clone(),
                },
                dark: primitives.dark.clone(),
                light: primitives.light.clone(),
                accent: primitives.eye.clone(),
            };
            let dress = |b: &tactics_core::content::BoneDef| looks.dress_bone(b, outfit);
            let (root, bones) = crate::rig::spawn_rig(commands, rig, &materials, &dress, unit_root, pivot);
            // Height from the look (uniform, so animated limbs don't shear).
            if let Some(a) = outfit.appearance {
                commands.entity(root).insert(Transform::from_scale(Vec3::splat(a.height_scale())));
            }
            // Two-handed weapons: the left hand follows the foregrip (IK).
            if !matches!(outfit.weapon_model, Some(_)) || rig.skeleton.socket("weapon").is_some() {
                if let Some(f) = crate::looks::foregrip(outfit.weapon) {
                    commands.entity(root).insert(crate::rig::TwoHanded(f));
                }
            }
            let sockets = crate::looks::rig_sockets(commands, &rig.skeleton, &bones);
            looks.dress_sockets(commands, library, &sockets, outfit, true);
            if model.scene.is_some() {
                commands.entity(root).insert(FallbackModel);
            }
        }
        None => spawn_primitive(commands, primitives, model.def.fallback, team, pivot),
    }

    if let Some(scene) = &model.scene {
        let [x, mut y, z] = model.def.offset;
        let mut scale = model.def.scale;
        if let Some((fit_scale, fit_y)) = model.fit {
            scale = fit_scale;
            y += fit_y;
        }
        let mut e = commands.spawn((
            WorldAssetRoot(scene.clone()),
            Transform::from_xyz(x, y, z)
                .with_rotation(Quat::from_rotation_y(model.def.yaw_degrees.to_radians()))
                .with_scale(Vec3::splat(scale)),
            ModelOwner { unit_root, model: model_id.to_string() },
            ChildOf(pivot),
        ));
        if let (Some(height), None) = (model.auto_fit, model.fit) {
            e.insert(AutoFit { height, base_offset: Vec3::new(x, model.def.offset[1], z) });
        }
        e.observe(on_scene_ready);
    }
}

/// Marks a glTF scene whose entities have spawned.
#[derive(Component)]
pub struct SceneReady;

/// The bind/rest transforms of a glTF scene's animated joints.
///
/// glTF clips only drive the joints they have tracks for; any other joint
/// keeps whatever the *previous* clip left there (e.g. a "Walk" without hip
/// rotation would keep a "Die" clip's fallen hips). Restoring the rest pose
/// every frame, before animations are applied, makes every clip start from
/// the same neutral pose.
#[derive(Component, Debug, Default)]
pub struct RestPose(pub Vec<(Entity, Transform)>);

/// Restores [`RestPose`]s; the animation system then overrides what it animates.
pub fn restore_rest_poses(poses: Query<&RestPose>, mut transforms: Query<&mut Transform>) {
    for pose in &poses {
        for (e, rest) in &pose.0 {
            if let Ok(mut t) = transforms.get_mut(*e)
                && *t != *rest {
                    *t = *rest;
                }
        }
    }
}

/// Scale a freshly spawned glTF scene to `height` with its lowest point on
/// the ground, once its meshes have bounding boxes. The result is cached in
/// the [`ModelLibrary`] so later spawns of the same model use it directly.
#[derive(Component, Debug, Clone, Copy)]
pub struct AutoFit {
    pub height: f32,
    base_offset: Vec3,
}

/// Measures and fits [`AutoFit`] scenes (see there).
pub fn auto_fit(
    mut commands: Commands,
    mut scenes: Query<(Entity, &AutoFit, &ModelOwner, &GlobalTransform, &mut Transform), With<SceneReady>>,
    children: Query<&Children>,
    meshes: Query<(&bevy::camera::primitives::Aabb, &GlobalTransform)>,
    mut library: ResMut<ModelLibrary>,
) {
    for (entity, fit, owner, scene_gt, mut tr) in &mut scenes {
        // Bounds of every mesh, in the scene's own (unscaled) space.
        let to_local = scene_gt.affine().inverse();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut found = false;
        for d in children.iter_descendants(entity) {
            let Ok((aabb, gt)) = meshes.get(d) else { continue };
            found = true;
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let world = gt.transform_point(c + h * Vec3::new(sx, sy, sz));
                        let y = to_local.transform_point3(world).y;
                        lo = lo.min(y);
                        hi = hi.max(y);
                    }
                }
            }
        }
        if !found {
            continue; // bounds not computed yet: try next frame
        }
        let height = (hi - lo).max(1e-3);
        let scale = fit.height / height;
        let y = -lo * scale;
        tr.scale = Vec3::splat(scale);
        tr.translation = fit.base_offset + Vec3::Y * y;
        commands.entity(entity).remove::<AutoFit>();
        if let Some(model) = library.models.get_mut(&owner.model) {
            model.fit = Some((scale, y));
            info!("auto-fit '{}': {:.2} m tall in the file -> scale {:.3}", owner.model, height, scale);
        }
    }
}

/// When a glTF scene has spawned: drop the fallback and hook up animations.
#[allow(clippy::too_many_arguments)]
fn on_scene_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    owners: Query<(&ModelOwner, &ChildOf)>,
    children: Query<&Children>,
    fallbacks: Query<Entity, With<FallbackModel>>,
    mut players: Query<&mut AnimationPlayer>,
    mut animators: Query<&mut UnitAnimator>,
    joints: Query<&Transform, With<bevy::animation::AnimationTargetId>>,
    library: Res<ModelLibrary>,
) {
    let scene = ready.entity;
    let Ok((owner, pivot)) = owners.get(scene) else { return };
    // Remember the rest pose of every animated joint (see `RestPose`).
    let rest: Vec<(Entity, Transform)> =
        children.iter_descendants(scene).filter_map(|e| joints.get(e).ok().map(|t| (e, *t))).collect();
    commands.entity(scene).insert((SceneReady, RestPose(rest)));

    // Remove the primitive stand-in (a sibling of the scene under the pivot).
    for sibling in children.iter_descendants(pivot.parent()) {
        if fallbacks.contains(sibling) {
            commands.entity(sibling).despawn();
        }
    }

    // Connect the first AnimationPlayer found in the scene.
    let Some(model) = library.get(&owner.model) else { return };
    let Some(graph) = &model.graph else { return };
    for e in children.iter_descendants(scene) {
        if let Ok(mut player) = players.get_mut(e) {
            let mut transitions = AnimationTransitions::new();
            if let Some(idle) = model.clips.get("idle") {
                transitions.play(&mut player, idle.node, Duration::ZERO).repeat();
            }
            commands.entity(e).insert((AnimationGraphHandle(graph.clone()), transitions));
            if let Ok(mut animator) = animators.get_mut(owner.unit_root) {
                animator.attach_player(e);
            }
            break;
        }
    }
}

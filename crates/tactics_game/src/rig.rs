//! Skeletal rigs: bone entities driven by keyframe animations with layering.
//!
//! A model with `skeleton: Some(id)` is drawn as a **rig**: one entity per
//! bone of the resolved [`ResolvedSkeleton`], each with a simple shape
//! (box/capsule/sphere) coloured by its [`BoneColor`]. The unit's
//! [`UnitAnimator`](crate::animation::UnitAnimator) decides what plays; this
//! module evaluates the pose:
//!
//! ```text
//!   base layer      idle / walk / hunker (looping), cross-faded on change
//!        +
//!   overlay layer   one-shot (shoot, throw, hit, death, custom...)
//!                   - replace: bones the clip animates, limited by its mask
//!                   - additive: added on top (flinches)
//!        =
//!   bone transform  rest offset + translation, rest rotation × rotation
//! ```
//!
//! Masks make layering useful: `shoot` only affects the upper body, so a unit
//! hit by overwatch mid-walk keeps walking while it fires back.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::prelude::*;
use tactics_core::content::{AnimationDef, BoneColor, BonePose, BoneShape, ResolvedSkeleton, Ruleset};

/// How long base-layer changes (idle → walk) cross-fade.
pub const BASE_FADE: f32 = 0.15;
/// Fade in / out time of one-shot overlays.
pub const OVERLAY_FADE_IN: f32 = 0.08;
pub const OVERLAY_FADE_OUT: f32 = 0.12;

/// Everything needed to spawn and animate one skeleton, built once.
#[derive(Debug)]
pub struct RigDef {
    pub skeleton: ResolvedSkeleton,
    /// Every animation available (own + inherited), by slot.
    pub animations: BTreeMap<String, AnimationDef>,
    /// Bone indices each slot affects (its mask's subtree, or all bones).
    masks: BTreeMap<String, BTreeSet<usize>>,
    /// Mesh per bone (`None` = joint without shape).
    pub meshes: Vec<Option<Handle<Mesh>>>,
}

impl RigDef {
    /// Resolves `skeleton` from the rules and builds meshes for its bones.
    pub fn build(rules: &Ruleset, skeleton: &str, meshes: &mut Assets<Mesh>) -> Result<Self, String> {
        let resolved = rules.resolve_skeleton(skeleton)?;
        let animations = rules.skeleton_animations(skeleton);
        let masks = animations
            .iter()
            .map(|(slot, a)| {
                let set = match &a.mask {
                    Some(roots) => resolved.subtree(roots),
                    None => (0..resolved.bones.len()).collect(),
                };
                (slot.clone(), set)
            })
            .collect();
        let mesh_handles = resolved
            .bones
            .iter()
            .map(|b| match b.shape {
                BoneShape::None => None,
                BoneShape::Box { size } => Some(meshes.add(Cuboid::new(size[0], size[1], size[2]))),
                BoneShape::Capsule { radius, length } => Some(meshes.add(Capsule3d::new(radius, length))),
                BoneShape::Sphere { radius } => Some(meshes.add(Sphere::new(radius))),
            })
            .collect();
        Ok(Self { skeleton: resolved, animations, masks, meshes: mesh_handles })
    }

    pub fn has(&self, slot: &str) -> bool {
        self.animations.contains_key(slot)
    }

    /// Samples `slot` at `time` into a per-bone pose (`None` = not animated).
    fn sample(&self, slot: &str, time: f32) -> Vec<Option<BonePose>> {
        let mut out = vec![None; self.skeleton.bones.len()];
        if let Some(anim) = self.animations.get(slot) {
            for (bone, pose) in anim.sample(time) {
                if let Some(i) = self.skeleton.index_of(bone) {
                    out[i] = Some(pose);
                }
            }
        }
        out
    }

    /// The layered pose. `base_prev` is the base slot being faded out (with
    /// how much of it remains, 1 → 0); `overlay` is the one-shot with its weight.
    pub fn pose(&self, base: (&str, f32), base_prev: Option<(&str, f32, f32)>, overlay: Option<(&str, f32, f32)>) -> Vec<BonePose> {
        let n = self.skeleton.bones.len();
        let mut pose: Vec<BonePose> = self.sample(base.0, base.1).into_iter().map(Option::unwrap_or_default).collect();

        if let Some((slot, time, remaining)) = base_prev {
            let prev = self.sample(slot, time);
            for i in 0..n {
                pose[i] = lerp(prev[i].unwrap_or_default(), pose[i], 1.0 - remaining.clamp(0.0, 1.0));
            }
        }

        if let Some((slot, time, weight)) = overlay
            && let Some(anim) = self.animations.get(slot)
        {
            let over = self.sample(slot, time);
            let mask = &self.masks[slot];
            for i in 0..n {
                let Some(o) = over[i] else { continue };
                if !mask.contains(&i) {
                    continue;
                }
                pose[i] = if anim.additive { add(pose[i], o, weight) } else { lerp(pose[i], o, weight) };
            }
        }
        pose
    }

    /// Local transform of bone `i` for an animated pose.
    pub fn bone_transform(&self, i: usize, pose: BonePose) -> Transform {
        let b = &self.skeleton.bones[i];
        let rest = euler(b.rotation);
        Transform::from_translation(Vec3::from(b.offset) + Vec3::from(pose.translation)).with_rotation(rest * euler(pose.rotation))
    }
}

fn euler(deg: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::XYZ, deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians())
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn lerp(a: BonePose, b: BonePose, t: f32) -> BonePose {
    BonePose { rotation: mix(a.rotation, b.rotation, t), translation: mix(a.translation, b.translation, t) }
}

fn add(a: BonePose, b: BonePose, w: f32) -> BonePose {
    BonePose {
        rotation: [a.rotation[0] + b.rotation[0] * w, a.rotation[1] + b.rotation[1] * w, a.rotation[2] + b.rotation[2] * w],
        translation: [
            a.translation[0] + b.translation[0] * w,
            a.translation[1] + b.translation[1] * w,
            a.translation[2] + b.translation[2] * w,
        ],
    }
}

/// Overlay weight: fade in at the start, out at the end (unless held).
pub fn overlay_weight(elapsed: f32, length: f32, hold: bool) -> f32 {
    let fade_in = (elapsed / OVERLAY_FADE_IN).min(1.0);
    let fade_out = if hold { 1.0 } else { ((length - elapsed) / OVERLAY_FADE_OUT).clamp(0.0, 1.0) };
    fade_in.min(fade_out)
}

/// On a rig's root entity: which model it shows and its bone entities.
#[derive(Component)]
pub struct Rig {
    pub def: Arc<RigDef>,
    pub bones: Vec<Entity>,
    /// The unit root this rig animates for.
    pub owner: Entity,
}

/// Materials per colour role.
pub struct RigMaterials {
    pub team: Handle<StandardMaterial>,
    pub dark: Handle<StandardMaterial>,
    pub light: Handle<StandardMaterial>,
    pub accent: Handle<StandardMaterial>,
}

/// Spawns the rig under `parent`. Returns the rig root.
pub fn spawn_rig(commands: &mut Commands, def: &Arc<RigDef>, materials: &RigMaterials, owner: Entity, parent: Entity) -> Entity {
    let root = commands.spawn((Transform::default(), Visibility::default(), ChildOf(parent))).id();
    let mut bones: Vec<Entity> = Vec::with_capacity(def.skeleton.bones.len());
    for (i, b) in def.skeleton.bones.iter().enumerate() {
        let attach = def.skeleton.parents[i].map_or(root, |p| bones[p]);
        let e = commands
            .spawn((def.bone_transform(i, BonePose::default()), Visibility::default(), Name::new(b.name.clone()), ChildOf(attach)))
            .id();
        if let Some(mesh) = &def.meshes[i] {
            let mat = match b.color {
                BoneColor::Team => &materials.team,
                BoneColor::Dark => &materials.dark,
                BoneColor::Light => &materials.light,
                BoneColor::Accent => &materials.accent,
            };
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), Transform::from_translation(Vec3::from(b.shape_offset)), ChildOf(e)));
        }
        bones.push(e);
    }
    commands.entity(root).insert(Rig { def: def.clone(), bones, owner });
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use tactics_core::content::Ruleset;

    fn rig() -> RigDef {
        let mut rules = Ruleset::standard();
        rules.add_module(&tactics_core::content::data::DataModule::from_ron(include_str!("../assets/modules/creatures.ron")).unwrap());
        let mut meshes = Assets::<Mesh>::default();
        RigDef::build(&rules, "humanoid", &mut meshes).unwrap()
    }

    fn rot(def: &RigDef, pose: &[BonePose], bone: &str) -> [f32; 3] {
        pose[def.skeleton.index_of(bone).unwrap()].rotation
    }

    #[test]
    fn masked_overlay_keeps_the_legs_walking() {
        let def = rig();
        let walk = def.pose(("walk", 0.0), None, None);
        let walk_and_shoot = def.pose(("walk", 0.0), None, Some(("shoot", 0.2, 1.0)));
        assert_eq!(rot(&def, &walk, "thigh_l"), rot(&def, &walk_and_shoot, "thigh_l"), "legs untouched");
        assert_ne!(rot(&def, &walk, "upper_arm_r"), rot(&def, &walk_and_shoot, "upper_arm_r"), "arms aim");
    }

    #[test]
    fn additive_overlay_adds_and_weight_scales() {
        let def = rig();
        let base = def.pose(("idle", 0.0), None, None);
        // The standard hit peaks at 0.06 s with chest X -22 degrees.
        let half = def.pose(("idle", 0.0), None, Some(("hit", 0.06, 0.5)));
        let chest_base = rot(&def, &base, "chest")[0];
        let chest_hit = rot(&def, &half, "chest")[0];
        assert!((chest_hit - (chest_base - 11.0)).abs() < 0.01, "half of the -22 flinch added: {chest_hit}");
    }

    #[test]
    fn base_crossfade_blends() {
        let def = rig();
        let idle = rot(&def, &def.pose(("idle", 0.0), None, None), "thigh_l")[0];
        let walk = rot(&def, &def.pose(("walk", 0.0), None, None), "thigh_l")[0];
        let mid = rot(&def, &def.pose(("walk", 0.0), Some(("idle", 0.0, 0.5)), None), "thigh_l")[0];
        assert!((mid - (idle + walk) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn weights() {
        assert_eq!(overlay_weight(0.0, 1.0, false), 0.0);
        assert_eq!(overlay_weight(0.5, 1.0, false), 1.0);
        assert!(overlay_weight(0.95, 1.0, false) < 0.5);
        assert_eq!(overlay_weight(5.0, 1.0, true), 1.0, "held (death) stays");
    }

    #[test]
    fn custom_skeleton_on_top() {
        let mut rules = Ruleset::standard();
        rules.add_module(&tactics_core::content::data::DataModule::from_ron(include_str!("../assets/modules/creatures.ron")).unwrap());
        let def = RigDef::build(&rules, "sectoid", &mut Assets::<Mesh>::default()).unwrap();
        assert!(def.has("walk") && def.has("taunt"));
        // Its custom cast moves antennae, which the humanoid doesn't have.
        let pose = def.pose(("idle", 0.0), None, Some(("cast", 0.5, 1.0)));
        assert_ne!(rot(&def, &pose, "antenna_l"), [0.0; 3]);
    }
}

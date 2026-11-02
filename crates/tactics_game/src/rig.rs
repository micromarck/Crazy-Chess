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

    /// Bone `i` in rig space (the rig root at the origin) for a pose.
    pub fn world_of(&self, pose: &[BonePose], i: usize) -> Transform {
        let mut chain = vec![i];
        let mut j = i;
        while let Some(p) = self.skeleton.parents[j] {
            chain.push(p);
            j = p;
        }
        chain.iter().rev().fold(Transform::IDENTITY, |t, &b| t.mul_transform(self.bone_transform(b, pose[b])))
    }

    /// Two-handed hold: bends the left arm so the left hand reaches the
    /// weapon's foregrip, `foregrip` metres down the barrel from the grip.
    /// `weight` blends it in (0 = leave the animation alone). Works on any
    /// skeleton with `upper_arm_l`, `forearm_l`, `hand_l` and a `weapon`
    /// bone; others are left untouched.
    pub fn apply_hand_ik(&self, pose: &mut [BonePose], foregrip: f32, weight: f32) {
        if weight <= 0.001 {
            return;
        }
        let idx = |n: &str| self.skeleton.index_of(n);
        let (Some(upper), Some(fore), Some(hand), Some(weapon)) = (idx("upper_arm_l"), idx("forearm_l"), idx("hand_l"), idx("weapon")) else {
            return;
        };
        let w = self.world_of(pose, weapon);
        // Just under the handguard.
        let target = w.transform_point(Vec3::new(0.0, -0.035, -foregrip));
        let palm = Vec3::new(0.0, -0.05, 0.0);
        let original = (pose[upper], pose[fore]);
        let error = |pose: &[BonePose]| self.world_of(pose, hand).transform_point(palm).distance(target);
        // Coordinate descent over shoulder (x, y, z) and elbow (x) angles.
        let mut best = error(pose);
        let mut step = 12.0;
        for _ in 0..60 {
            if best < 0.004 || step < 0.3 {
                break;
            }
            let mut improved = false;
            for (bone, axis) in [(upper, 0), (upper, 1), (upper, 2), (fore, 0)] {
                for dir in [1.0, -1.0] {
                    pose[bone].rotation[axis] += dir * step;
                    let e = error(pose);
                    if e + 1e-5 < best {
                        best = e;
                        improved = true;
                    } else {
                        pose[bone].rotation[axis] -= dir * step;
                    }
                }
            }
            if !improved {
                step *= 0.5;
            }
        }
        // Elbows don't bend backwards.
        pose[fore].rotation[0] = pose[fore].rotation[0].max(0.0);
        if weight < 1.0 {
            pose[upper] = lerp(original.0, pose[upper], weight);
            pose[fore] = lerp(original.1, pose[fore], weight);
        }
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

/// On a rig root holding a two-handed weapon: foregrip distance down the
/// barrel for the left-hand IK.
#[derive(Component, Debug, Clone, Copy)]
pub struct TwoHanded(pub f32);

/// Slots whose left-hand motion matters more than the grip (IK fades out).
pub const FREE_HAND_SLOTS: [&str; 6] = ["throw", "reload", "heal", "cast", "death", "taunt"];

/// Materials per colour role.
pub struct RigMaterials {
    pub team: Handle<StandardMaterial>,
    pub dark: Handle<StandardMaterial>,
    pub light: Handle<StandardMaterial>,
    pub accent: Handle<StandardMaterial>,
}

/// Per-bone override of the material and the shape's scale (individual looks).
pub type BoneDresser<'a> = &'a dyn Fn(&tactics_core::content::BoneDef) -> Option<(Handle<StandardMaterial>, Vec3)>;

/// Spawns the rig under `parent`. Returns the rig root and the bone entities
/// (indexed like `def.skeleton.bones`, for attaching things to sockets).
pub fn spawn_rig(
    commands: &mut Commands,
    def: &Arc<RigDef>,
    materials: &RigMaterials,
    dress: BoneDresser,
    owner: Entity,
    parent: Entity,
) -> (Entity, Vec<Entity>) {
    let root = commands.spawn((Transform::default(), Visibility::default(), ChildOf(parent))).id();
    let mut bones: Vec<Entity> = Vec::with_capacity(def.skeleton.bones.len());
    for (i, b) in def.skeleton.bones.iter().enumerate() {
        let attach = def.skeleton.parents[i].map_or(root, |p| bones[p]);
        let e = commands
            .spawn((def.bone_transform(i, BonePose::default()), Visibility::default(), Name::new(b.name.clone()), ChildOf(attach)))
            .id();
        if let Some(mesh) = &def.meshes[i] {
            let base = match b.color {
                BoneColor::Team => &materials.team,
                BoneColor::Dark => &materials.dark,
                BoneColor::Light => &materials.light,
                BoneColor::Accent => &materials.accent,
            };
            let (mat, scale) = dress(b).unwrap_or((base.clone(), Vec3::ONE));
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(Vec3::from(b.shape_offset)).with_scale(scale),
                ChildOf(e),
            ));
        }
        bones.push(e);
    }
    commands.entity(root).insert(Rig { def: def.clone(), bones: bones.clone(), owner });
    (root, bones)
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

#[cfg(test)]
mod hold_probe {
    use super::*;

    fn def() -> RigDef {
        let mut rules = tactics_core::content::Ruleset::standard();
        rules.add_module(&tactics_core::modules::GrenadierModule);
        let mut meshes = Assets::<Mesh>::default();
        RigDef::build(&rules, "humanoid", &mut meshes).unwrap()
    }

    fn report(def: &RigDef, slot: &str, pose: &mut [BonePose]) -> (Vec3, Vec3, f32) {
        let wi = def.skeleton.index_of("weapon").unwrap();
        def.apply_hand_ik(pose, 0.22, 1.0);
        let w = def.world_of(pose, wi);
        let hand = def.world_of(pose, def.skeleton.index_of("hand_l").unwrap()).transform_point(Vec3::new(0.0, -0.05, 0.0));
        let target = w.transform_point(Vec3::new(0.0, -0.035, -0.22));
        let barrel = w.rotation * Vec3::NEG_Z;
        eprintln!("{slot:10} grip {:.2?} barrel {:.2?} ik gap {:.3}", w.translation, barrel, hand.distance(target));
        (w.translation, barrel, hand.distance(target))
    }

    /// The left hand reaches the foregrip in every pose that holds the gun.
    #[test]
    fn left_hand_holds_the_foregrip() {
        let def = def();
        for (slot, t) in [("idle", 0.0), ("idle", 2.0), ("walk", 0.15), ("hunker", 0.5), ("shoot", 0.3), ("overwatch", 0.4)] {
            let mut pose = if matches!(slot, "idle" | "walk" | "hunker") {
                def.pose((slot, t), None, None)
            } else {
                def.pose(("idle", 0.0), None, Some((slot, t, 1.0)))
            };
            let (grip, barrel, gap) = report(&def, slot, &mut pose);
            assert!(gap < 0.03, "{slot}: left hand {gap:.3} m from the foregrip");
            assert!(barrel.z < -0.6, "{slot}: gun points forward ({barrel:?})");
            assert!(grip.x.abs() < 0.3 && grip.z < 0.0, "{slot}: gun held in front ({grip:?})");
        }
    }
}

#[cfg(test)]
mod pose_solver {
    use super::*;

    /// Finds right-arm angles putting the weapon grip at `grip` (rig space)
    /// with the barrel along `dir`. Prints the angles (authoring tool).
    fn solve(def: &RigDef, base: &str, grip: Vec3, dir: Vec3) -> [f32; 7] {
        let ids: Vec<usize> = ["upper_arm_r", "forearm_r", "hand_r"].iter().map(|n| def.skeleton.index_of(n).unwrap()).collect();
        let wi = def.skeleton.index_of("weapon").unwrap();
        let dir = dir.normalize();
        // params: upper x, y, z; forearm x; hand x, y, z
        let params = [(ids[0], 0), (ids[0], 1), (ids[0], 2), (ids[1], 0), (ids[2], 0), (ids[2], 1), (ids[2], 2)];
        let mut overall: Option<(f32, [f32; 7])> = None;
        for start in [[0.0, 0.0, 0.0, 90.0, 0.0, 0.0, 0.0], [30.0, 0.0, -20.0, 60.0, -30.0, 0.0, 0.0], [60.0, 0.0, -10.0, 100.0, -70.0, 0.0, 0.0], [-10.0, 20.0, -20.0, 110.0, -20.0, 10.0, 0.0], [45.0, -20.0, -30.0, 45.0, 0.0, 20.0, 10.0]] {
            let mut pose = def.pose((base, 0.0), None, None);
            for (k, &(b, a)) in params.iter().enumerate() {
                pose[b].rotation[a] = start[k];
            }
            let cost = |pose: &[BonePose]| {
                let w = def.world_of(pose, wi);
                w.translation.distance(grip) + 0.3 * (w.rotation * Vec3::NEG_Z).distance(dir)
            };
            let mut best = cost(&pose);
            let mut step = 16.0;
            for _ in 0..2000 {
                if step < 0.1 {
                    break;
                }
                let mut improved = false;
                for &(b, a) in &params {
                    for d in [1.0, -1.0] {
                        pose[b].rotation[a] += d * step;
                        let c = cost(&pose);
                        if c + 1e-6 < best {
                            best = c;
                            improved = true;
                        } else {
                            pose[b].rotation[a] -= d * step;
                        }
                    }
                }
                if !improved {
                    step *= 0.5;
                }
            }
            let out = params.map(|(b, a)| (pose[b].rotation[a] * 10.0).round() / 10.0);
            if overall.is_none_or(|(c, _)| best < c) {
                overall = Some((best, out));
            }
        }
        let (best, out) = overall.unwrap();
        eprintln!("{base} grip {grip:.2?} dir {dir:.2?} -> {:?} cost {best:.3}", out);
        out
    }

    #[test]
    #[ignore]
    fn print_hold_poses() {
        let mut rules = tactics_core::content::Ruleset::standard();
        rules.add_module(&tactics_core::modules::GrenadierModule);
        let mut meshes = Assets::<Mesh>::default();
        let def = RigDef::build(&rules, "humanoid", &mut meshes).unwrap();
        let sh = def.world_of(&def.pose(("idle", 0.0), None, None), def.skeleton.index_of("upper_arm_r").unwrap()).translation;
        eprintln!("right shoulder at {sh:?}");
        for (g, d) in [
            (Vec3::new(0.1, sh.y - 0.22, -0.15), Vec3::new(-0.4, -0.3, -0.87)),
            (Vec3::new(0.08, sh.y - 0.2, -0.17), Vec3::new(-0.35, -0.25, -0.9)),
            (Vec3::new(0.05, sh.y - 0.09, -0.2), Vec3::new(0.0, 0.0, -1.0)),
            (Vec3::new(0.06, sh.y - 0.1, -0.18), Vec3::new(-0.08, 0.0, -1.0)),
        ] {
            let angles = solve(&def, "idle", g, d);
            // Left-hand reach check with these angles.
            let mut pose = def.pose(("idle", 0.0), None, None);
            for (j, n) in ["upper_arm_r", "forearm_r", "hand_r"].iter().enumerate() {
                let i = def.skeleton.index_of(n).unwrap();
                pose[i].rotation = match j {
                    0 => [angles[0], angles[1], angles[2]],
                    1 => [angles[3], 0.0, 0.0],
                    _ => [angles[4], angles[5], angles[6]],
                };
            }
            def.apply_hand_ik(&mut pose, 0.22, 1.0);
            let w = def.world_of(&pose, def.skeleton.index_of("weapon").unwrap());
            let hand = def.world_of(&pose, def.skeleton.index_of("hand_l").unwrap()).transform_point(Vec3::new(0.0, -0.05, 0.0));
            eprintln!("   ik gap {:.3}", hand.distance(w.transform_point(Vec3::new(0.0, -0.035, -0.22))));
        }
    }
}

//! Skeletons and keyframe animations (presentation data).
//!
//! A [`SkeletonDef`] is a bone hierarchy; an [`AnimationDef`] rotates/moves
//! bones over time. The built-in **`humanoid`** skeleton (base module) comes
//! with the standard movement set: `idle walk hunker shoot overwatch throw
//! reload heal cast hit death`.
//!
//! Custom content goes **on top**:
//!
//! * **Skeletons extend skeletons.** `extends: "humanoid"` inherits every bone
//!   (and every animation). Bones with the same name override the parent's
//!   (different proportions, shape, colour); new names add bones (antennae,
//!   tails, backpacks...).
//! * **Animations are looked up along the chain.** An animation for
//!   `(skeleton, slot)` overrides the parent's; anything not redefined is
//!   inherited, so a new skeleton only needs the animations that differ.
//! * **Layers.** One-shot clips play over the looping base (idle/walk/hunker).
//!   `mask` limits a clip to some bones and their children (e.g. `["spine"]`
//!   = upper body: shoot while walking), and `additive` adds the clip on top
//!   of whatever plays (hit flinches).
//!
//! Conventions: metres, Y up, the model faces **-Z**. Rotations are Euler
//! angles in **degrees** applied X, then Y, then Z, in the bone's parent space.
//! For a limb hanging down, +X swings it forward.
//!
//! This module is engine-agnostic: [`Ruleset::resolve_skeleton`] flattens
//! inheritance and [`AnimationDef::sample`] evaluates keyframes; the frontend
//! turns the result into transforms.

use std::collections::{BTreeMap, BTreeSet};

use super::Ruleset;

/// Colour role of a bone's shape (the frontend picks the actual colours).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum BoneColor {
    /// The unit's team colour.
    #[default]
    Team,
    /// Dark (weapons, boots, gear).
    Dark,
    /// Light (skin, visors).
    Light,
    /// Bright accent (eyes, lights).
    Accent,
}

/// What is drawn for a bone.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum BoneShape {
    /// Nothing (a pure joint).
    #[default]
    None,
    Box { size: [f32; 3] },
    /// Vertical capsule (along the bone's Y).
    Capsule { radius: f32, length: f32 },
    Sphere { radius: f32 },
}

/// One bone.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct BoneDef {
    pub name: String,
    /// Parent bone name; `None` = attached to the model root.
    pub parent: Option<String>,
    /// Rest position relative to the parent bone (metres).
    pub offset: [f32; 3],
    /// Rest rotation (degrees, XYZ), e.g. to splay antennae.
    pub rotation: [f32; 3],
    pub shape: BoneShape,
    /// Where the shape sits relative to the bone (e.g. halfway down a limb).
    pub shape_offset: [f32; 3],
    pub color: BoneColor,
}

/// A named attachment point on a bone: where weapons, helmets, backpacks
/// and other gear are mounted. `rotation` turns the attached thing (degrees,
/// XYZ); by convention attachments point their front along -Z.
///
/// Standard humanoid sockets: `weapon` (main hand grip, gun pointing along
/// the barrel), `hand_l`, `head` (top of the head), `back`, `chest`,
/// `hip_l`, `hip_r`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SocketDef {
    pub name: String,
    pub bone: String,
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
}

/// A bone hierarchy, optionally extending another.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SkeletonDef {
    pub id: String,
    /// Inherit bones and animations from this skeleton.
    pub extends: Option<String>,
    /// New bones, or overrides of inherited bones with the same name.
    pub bones: Vec<BoneDef>,
    /// Attachment points (inherited; same name overrides).
    pub sockets: Vec<SocketDef>,
}

/// One keyframe: `(time in seconds, value)`.
pub type Key = (f32, [f32; 3]);

/// Interpolation between keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Easing {
    Linear,
    /// Ease in and out (smoother motion).
    #[default]
    Smooth,
}

/// A keyframe animation for one skeleton, played in an animation slot.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AnimationDef {
    /// Skeleton it belongs to (inherited by skeletons extending it).
    pub skeleton: String,
    /// Slot name (`idle`, `walk`, `shoot`, or any custom name an ability uses).
    pub slot: String,
    /// Length in seconds.
    pub duration: f32,
    pub looping: bool,
    /// Bone rotations: bone → keys of Euler degrees (XYZ).
    pub rotations: BTreeMap<String, Vec<Key>>,
    /// Bone offsets added to the rest position: bone → keys of metres.
    pub translations: BTreeMap<String, Vec<Key>>,
    /// Only these bones and their descendants are affected (`None` = all).
    pub mask: Option<Vec<String>>,
    /// Add on top of the current pose instead of replacing it.
    pub additive: bool,
    pub easing: Easing,
}

impl Default for AnimationDef {
    fn default() -> Self {
        Self {
            skeleton: String::new(),
            slot: String::new(),
            duration: 1.0,
            looping: false,
            rotations: BTreeMap::new(),
            translations: BTreeMap::new(),
            mask: None,
            additive: false,
            easing: Easing::Smooth,
        }
    }
}

fn sample_keys(keys: &[Key], t: f32, easing: Easing) -> [f32; 3] {
    match keys {
        [] => [0.0; 3],
        [only] => only.1,
        _ => {
            if t <= keys[0].0 {
                return keys[0].1;
            }
            for w in keys.windows(2) {
                let ((t0, a), (t1, b)) = (w[0], w[1]);
                if t <= t1 {
                    let mut k = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
                    if easing == Easing::Smooth {
                        k = k * k * (3.0 - 2.0 * k);
                    }
                    return [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
                }
            }
            keys[keys.len() - 1].1
        }
    }
}

/// The pose of one bone produced by sampling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BonePose {
    /// Euler degrees (XYZ).
    pub rotation: [f32; 3],
    /// Metres added to the rest offset.
    pub translation: [f32; 3],
}

impl AnimationDef {
    /// Local time for `elapsed` seconds of playback (wraps when looping,
    /// holds the last frame otherwise).
    pub fn local_time(&self, elapsed: f32) -> f32 {
        let d = self.duration.max(1e-3);
        if self.looping { elapsed.rem_euclid(d) } else { elapsed.clamp(0.0, d) }
    }

    /// Samples every animated bone at `elapsed` seconds.
    pub fn sample(&self, elapsed: f32) -> BTreeMap<&str, BonePose> {
        let t = self.local_time(elapsed);
        let mut out: BTreeMap<&str, BonePose> = BTreeMap::new();
        for (bone, keys) in &self.rotations {
            out.entry(bone.as_str()).or_default().rotation = sample_keys(keys, t, self.easing);
        }
        for (bone, keys) in &self.translations {
            out.entry(bone.as_str()).or_default().translation = sample_keys(keys, t, self.easing);
        }
        out
    }
}

/// A skeleton with inheritance flattened: bones ordered parents-first.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSkeleton {
    pub id: String,
    pub bones: Vec<BoneDef>,
    /// Index of each bone's parent in `bones`.
    pub parents: Vec<Option<usize>>,
    /// The chain from this skeleton up to its root ancestor (for animation lookup).
    pub lineage: Vec<String>,
    /// Attachment points whose bone exists.
    pub sockets: Vec<SocketDef>,
}

impl ResolvedSkeleton {
    pub fn index_of(&self, bone: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == bone)
    }

    pub fn socket(&self, name: &str) -> Option<&SocketDef> {
        self.sockets.iter().find(|s| s.name == name)
    }

    /// Indices of `roots` and all their descendants (for masks).
    pub fn subtree(&self, roots: &[String]) -> BTreeSet<usize> {
        let mut set: BTreeSet<usize> = roots.iter().filter_map(|r| self.index_of(r)).collect();
        // Parents come first, so one forward pass collects descendants.
        for i in 0..self.bones.len() {
            if self.parents[i].is_some_and(|p| set.contains(&p)) {
                set.insert(i);
            }
        }
        set
    }
}

impl Ruleset {
    /// Flattens `extends` and orders bones parents-first.
    pub fn resolve_skeleton(&self, id: &str) -> Result<ResolvedSkeleton, String> {
        // Walk up the chain (guarding against cycles).
        let mut lineage = Vec::new();
        let mut current = Some(id.to_string());
        while let Some(c) = current {
            if lineage.contains(&c) {
                return Err(format!("skeleton '{id}': 'extends' loops through '{c}'"));
            }
            let def = self.skeleton(&c).ok_or_else(|| format!("unknown skeleton '{c}'"))?;
            lineage.push(c);
            current = def.extends.clone();
        }
        // Apply from the root ancestor down: children override by name.
        let mut bones: Vec<BoneDef> = Vec::new();
        for sk in lineage.iter().rev() {
            for b in &self.skeleton(sk).expect("checked").bones {
                match bones.iter_mut().find(|x| x.name == b.name) {
                    Some(existing) => *existing = b.clone(),
                    None => bones.push(b.clone()),
                }
            }
        }
        // Order parents-first.
        let mut ordered: Vec<BoneDef> = Vec::with_capacity(bones.len());
        let mut remaining = bones;
        while !remaining.is_empty() {
            let before = remaining.len();
            remaining.retain(|b| {
                let ready = b.parent.as_ref().is_none_or(|p| ordered.iter().any(|o| &o.name == p));
                if ready {
                    ordered.push(b.clone());
                }
                !ready
            });
            if remaining.len() == before {
                let names: Vec<_> = remaining.iter().map(|b| format!("{} (parent {:?})", b.name, b.parent)).collect();
                return Err(format!("skeleton '{id}': bones with missing or cyclic parents: {}", names.join(", ")));
            }
        }
        let parents = ordered
            .iter()
            .map(|b| b.parent.as_ref().and_then(|p| ordered.iter().position(|o| &o.name == p)))
            .collect();
        let mut sockets: Vec<SocketDef> = Vec::new();
        for sk in lineage.iter().rev() {
            for s in &self.skeleton(sk).expect("checked").sockets {
                match sockets.iter_mut().find(|x| x.name == s.name) {
                    Some(existing) => *existing = s.clone(),
                    None => sockets.push(s.clone()),
                }
            }
        }
        if let Some(bad) = sockets.iter().find(|s| !ordered.iter().any(|b| b.name == s.bone)) {
            return Err(format!("skeleton '{id}': socket '{}' is on unknown bone '{}'", bad.name, bad.bone));
        }
        Ok(ResolvedSkeleton { id: id.to_string(), bones: ordered, parents, lineage, sockets })
    }

    /// Every animation available to a skeleton (its own, then inherited ones
    /// it doesn't override), by slot.
    pub fn skeleton_animations(&self, id: &str) -> BTreeMap<String, AnimationDef> {
        let mut out = BTreeMap::new();
        if let Ok(resolved) = self.resolve_skeleton(id) {
            for sk in &resolved.lineage {
                for anim in self.animations_of(sk) {
                    out.entry(anim.slot.clone()).or_insert_with(|| anim.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bone(name: &str, parent: Option<&str>, y: f32) -> BoneDef {
        BoneDef { name: name.into(), parent: parent.map(Into::into), offset: [0.0, y, 0.0], ..Default::default() }
    }

    fn rules() -> Ruleset {
        let mut r = Ruleset::new();
        r.add_skeleton(SkeletonDef {
            id: "base".into(),
            extends: None,
            // Deliberately out of order: resolution sorts parents first.
            bones: vec![bone("head", Some("chest"), 0.3), bone("hips", None, 0.5), bone("chest", Some("hips"), 0.4)],
            sockets: vec![SocketDef { name: "hat".into(), bone: "head".into(), ..Default::default() }],
        });
        r.add_skeleton(SkeletonDef {
            id: "big".into(),
            extends: Some("base".into()),
            bones: vec![bone("head", Some("chest"), 0.6), bone("antenna", Some("head"), 0.2)],
            sockets: vec![SocketDef { name: "hat".into(), bone: "antenna".into(), ..Default::default() }],
        });
        let anim = |sk: &str, slot: &str, deg: f32| AnimationDef {
            skeleton: sk.into(),
            slot: slot.into(),
            duration: 1.0,
            rotations: [("chest".to_string(), vec![(0.0, [0.0; 3]), (1.0, [deg, 0.0, 0.0])])].into(),
            easing: Easing::Linear,
            ..Default::default()
        };
        r.add_animation(anim("base", "idle", 10.0));
        r.add_animation(anim("base", "shoot", 20.0));
        r.add_animation(anim("big", "shoot", 90.0));
        r
    }

    #[test]
    fn inheritance_overrides_and_adds_bones() {
        let r = rules();
        let s = r.resolve_skeleton("big").unwrap();
        let names: Vec<_> = s.bones.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["hips", "chest", "head", "antenna"], "parents first");
        assert_eq!(s.bones[2].offset[1], 0.6, "child overrides the head");
        assert_eq!(s.socket("hat").unwrap().bone, "antenna", "sockets are inherited and overridden by name");
        assert_eq!(s.parents, vec![None, Some(0), Some(1), Some(2)]);
        assert_eq!(s.subtree(&["chest".into()]), [1, 2, 3].into());
    }

    #[test]
    fn animations_inherit_and_override() {
        let r = rules();
        let anims = r.skeleton_animations("big");
        assert_eq!(anims["idle"].skeleton, "base", "inherited");
        assert_eq!(anims["shoot"].skeleton, "big", "overridden");
        assert_eq!(r.skeleton_animations("base")["shoot"].skeleton, "base");
    }

    #[test]
    fn sampling_loops_and_holds() {
        let r = rules();
        let mut a = r.skeleton_animations("base")["idle"].clone();
        assert_eq!(a.sample(0.5)["chest"].rotation[0], 5.0);
        assert_eq!(a.sample(1.5)["chest"].rotation[0], 10.0, "one-shot holds the last frame");
        a.looping = true;
        assert!((a.sample(1.5)["chest"].rotation[0] - 5.0).abs() < 1e-4, "looping wraps");
        a.easing = Easing::Smooth;
        assert!(a.sample(1.25)["chest"].rotation[0] < 2.5, "smooth easing starts slower");
    }

    #[test]
    fn broken_hierarchies_are_reported() {
        let mut r = Ruleset::new();
        r.add_skeleton(SkeletonDef { id: "a".into(), extends: Some("b".into()), ..Default::default() });
        r.add_skeleton(SkeletonDef { id: "b".into(), extends: Some("a".into()), ..Default::default() });
        assert!(r.resolve_skeleton("a").unwrap_err().contains("loops"));
        r.add_skeleton(SkeletonDef { id: "c".into(), bones: vec![bone("arm", Some("nope"), 0.0)], ..Default::default() });
        assert!(r.resolve_skeleton("c").unwrap_err().contains("missing"));
    }
}

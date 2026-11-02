//! The standard **humanoid** skeleton and its basic movement animations.
//!
//! ```text
//!                head
//!                 |
//!                neck
//!                 |
//!  hand_l-forearm_l-upper_arm_l--chest--upper_arm_r-forearm_r-hand_r-weapon
//!                 |
//!                spine
//!                 |
//!                hips (root, 0.55 m above the ground)
//!              /      \
//!          thigh_l    thigh_r
//!             |          |
//!          shin_l     shin_r
//!             |          |
//!          foot_l     foot_r
//! ```
//!
//! Animations (slots the game plays automatically):
//!
//! | slot | kind | notes |
//! |---|---|---|
//! | `idle` | loop | breathing, head sway, rifle carried |
//! | `walk` | loop | leg cycle, hip bob, torso twist |
//! | `hunker` | loop | crouched (while defending) |
//! | `shoot` | one-shot, upper body | raise, recoil, lower: works while walking |
//! | `overwatch` | one-shot, upper body | raise to aim |
//! | `throw` | one-shot, upper body | wind-up and release |
//! | `reload` | one-shot, upper body | support hand to the magazine |
//! | `heal` | one-shot | kneel and treat |
//! | `cast` | one-shot, upper body | arms up (psionics) |
//! | `hit` | one-shot, **additive** | flinch on top of anything |
//! | `death` | one-shot, holds | collapse |
//!
//! Custom skeletons `extends: "humanoid"` inherit all of this and only
//! redefine what differs (see `assets/modules/creatures.ron`).

use crate::content::*;

pub const HUMANOID: &str = "humanoid";

fn bone(name: &str, parent: Option<&str>, offset: [f32; 3], shape: BoneShape, shape_offset: [f32; 3], color: BoneColor) -> BoneDef {
    BoneDef {
        name: name.into(),
        parent: parent.map(Into::into),
        offset,
        rotation: [0.0; 3],
        shape,
        shape_offset,
        color,
    }
}

/// The bone hierarchy.
pub fn skeleton() -> SkeletonDef {
    use BoneColor::*;
    use BoneShape::{Box as Cube, Capsule, Sphere};
    let mut bones = vec![
        bone("hips", None, [0.0, 0.55, 0.0], Cube { size: [0.3, 0.14, 0.18] }, [0.0; 3], Dark),
        bone("spine", Some("hips"), [0.0, 0.07, 0.0], Cube { size: [0.3, 0.22, 0.18] }, [0.0, 0.11, 0.0], Team),
        bone("chest", Some("spine"), [0.0, 0.22, 0.0], Cube { size: [0.38, 0.2, 0.22] }, [0.0, 0.08, 0.0], Team),
        bone("neck", Some("chest"), [0.0, 0.19, 0.0], BoneShape::None, [0.0; 3], Light),
        bone("head", Some("neck"), [0.0, 0.03, 0.0], Sphere { radius: 0.12 }, [0.0, 0.12, 0.0], Team),
    ];
    for (side, x) in [("l", -1.0f32), ("r", 1.0)] {
        let n = |b: &str| format!("{b}_{side}");
        let arm = |b: &str, parent: String, offset, shape, shape_offset, color| BoneDef {
            name: n(b),
            parent: Some(parent),
            offset,
            rotation: [0.0; 3],
            shape,
            shape_offset,
            color,
        };
        bones.push(arm("upper_arm", "chest".into(), [0.23 * x, 0.15, 0.0], Capsule { radius: 0.055, length: 0.16 }, [0.0, -0.13, 0.0], Team));
        bones.push(arm("forearm", n("upper_arm"), [0.0, -0.26, 0.0], Capsule { radius: 0.045, length: 0.15 }, [0.0, -0.12, 0.0], Team));
        bones.push(arm("hand", n("forearm"), [0.0, -0.24, 0.0], Cube { size: [0.07, 0.08, 0.07] }, [0.0, -0.03, 0.0], Light));
        bones.push(arm("thigh", "hips".into(), [0.1 * x, -0.05, 0.0], Capsule { radius: 0.07, length: 0.18 }, [0.0, -0.14, 0.0], Dark));
        bones.push(arm("shin", n("thigh"), [0.0, -0.27, 0.0], Capsule { radius: 0.06, length: 0.16 }, [0.0, -0.12, 0.0], Dark));
        bones.push(arm("foot", n("shin"), [0.0, -0.23, 0.0], Cube { size: [0.1, 0.05, 0.22] }, [0.0, 0.0, -0.04], Dark));
    }
    // The weapon rests pointing down so that raising the arm 90° aims it forward.
    bones.push(BoneDef {
        name: "weapon".into(),
        parent: Some("hand_r".into()),
        offset: [0.0, -0.05, 0.0],
        rotation: [-90.0, 0.0, 0.0],
        shape: Cube { size: [0.07, 0.1, 0.55] },
        shape_offset: [0.0, 0.0, -0.17],
        color: Dark,
    });
    SkeletonDef { id: HUMANOID.into(), extends: None, bones }
}

/// Keyframe helper: `(time, [x, y, z])`.
fn k(t: f32, x: f32, y: f32, z: f32) -> Key {
    (t, [x, y, z])
}

/// Builder for an animation.
struct Anim(AnimationDef);

impl Anim {
    fn new(slot: &str, duration: f32, looping: bool) -> Self {
        Anim(AnimationDef { skeleton: HUMANOID.into(), slot: slot.into(), duration, looping, ..Default::default() })
    }
    fn rot(mut self, bone: &str, keys: Vec<Key>) -> Self {
        self.0.rotations.insert(bone.into(), keys);
        self
    }
    fn mov(mut self, bone: &str, keys: Vec<Key>) -> Self {
        self.0.translations.insert(bone.into(), keys);
        self
    }
    fn upper_body(mut self) -> Self {
        self.0.mask = Some(vec!["spine".into()]);
        self
    }
    fn additive(mut self) -> Self {
        self.0.additive = true;
        self
    }
    fn linear(mut self) -> Self {
        self.0.easing = Easing::Linear;
        self
    }
    /// Arms holding the rifle across the body (held constant).
    fn carry(self) -> Self {
        self.rot("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0)])
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0)])
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0)])
    }
}

/// Builds arm keys: carry → aim at `up`, held until `hold`, back to carry at `end`.
fn raise(up: f32, hold: f32, end: f32) -> [(&'static str, Vec<Key>); 4] {
    [
        ("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0), k(up, 85.0, 0.0, 0.0), k(hold, 85.0, 0.0, 0.0), k(end, 35.0, 0.0, 0.0)]),
        ("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0), k(up, 8.0, 0.0, 0.0), k(hold, 8.0, 0.0, 0.0), k(end, 55.0, 0.0, 0.0)]),
        ("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(up, 75.0, 0.0, 28.0), k(hold, 75.0, 0.0, 28.0), k(end, 45.0, 0.0, 20.0)]),
        ("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0), k(up, 22.0, 0.0, 0.0), k(hold, 22.0, 0.0, 0.0), k(end, 50.0, 0.0, 0.0)]),
    ]
}

/// The standard movement set.
pub fn animations() -> Vec<AnimationDef> {
    let mut out = Vec::new();

    // Idle: weight shift onto one hip, breathing, a glance left then right.
    out.push(
        Anim::new("idle", 4.0, true)
            .carry()
            .mov(
                "hips",
                vec![k(0.0, 0.0, 0.0, 0.0), k(1.0, 0.02, -0.01, 0.0), k(2.0, 0.02, -0.01, 0.0), k(3.0, 0.0, 0.0, 0.0), k(4.0, 0.0, 0.0, 0.0)],
            )
            .rot("hips", vec![k(0.0, 0.0, 0.0, 0.0), k(1.0, 0.0, 0.0, -2.0), k(2.0, 0.0, 0.0, -2.0), k(3.0, 0.0, 0.0, 0.0), k(4.0, 0.0, 0.0, 0.0)])
            .rot("spine", vec![k(0.0, 0.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 2.5), k(2.0, 0.0, 0.0, 2.5), k(3.0, 0.0, 0.0, 0.0), k(4.0, 0.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(2.0, 3.5, 0.0, 0.0), k(4.0, 0.0, 0.0, 0.0)])
            .rot(
                "head",
                vec![
                    k(0.0, 0.0, 0.0, 0.0),
                    k(1.2, 0.0, 12.0, 0.0),
                    k(2.0, 0.0, 12.0, 0.0),
                    k(2.6, 4.0, -8.0, 0.0),
                    k(3.4, 4.0, -8.0, 0.0),
                    k(4.0, 0.0, 0.0, 0.0),
                ],
            )
            .rot("thigh_l", vec![k(0.0, 4.0, 0.0, -2.0)])
            .rot("thigh_r", vec![k(0.0, 2.0, 0.0, 3.0)])
            .rot("shin_l", vec![k(0.0, -8.0, 0.0, 0.0)])
            .rot("shin_r", vec![k(0.0, -4.0, 0.0, 0.0)])
            .rot("foot_l", vec![k(0.0, 4.0, 0.0, 0.0)])
            .0,
    );

    // Walk: a jog cycle (0.6 s = two steps), matched to the movement speed.
    // Keys: 0 left contact, 0.15 left mid-stance, 0.3 right contact,
    // 0.45 right mid-stance (left leg passing), 0.6 left contact again.
    let c = |a: f32, b: f32, c2: f32, d: f32| vec![k(0.0, a, 0.0, 0.0), k(0.15, b, 0.0, 0.0), k(0.3, c2, 0.0, 0.0), k(0.45, d, 0.0, 0.0), k(0.6, a, 0.0, 0.0)];
    out.push(
        Anim::new("walk", 0.6, true)
            .linear()
            .rot("thigh_l", c(35.0, 0.0, -35.0, 10.0))
            .rot("shin_l", c(-10.0, -25.0, -20.0, -95.0))
            .rot("foot_l", c(15.0, 0.0, -30.0, 10.0))
            .rot("thigh_r", c(-35.0, 10.0, 35.0, 0.0))
            .rot("shin_r", c(-20.0, -95.0, -10.0, -25.0))
            .rot("foot_r", c(-30.0, 10.0, 15.0, 0.0))
            .mov(
                "hips",
                vec![k(0.0, 0.0, 0.0, 0.0), k(0.15, 0.0, -0.04, 0.0), k(0.3, 0.0, 0.0, 0.0), k(0.45, 0.0, -0.04, 0.0), k(0.6, 0.0, 0.0, 0.0)],
            )
            .rot(
                "hips",
                vec![k(0.0, 0.0, -8.0, 0.0), k(0.15, 0.0, 0.0, 3.0), k(0.3, 0.0, 8.0, 0.0), k(0.45, 0.0, 0.0, -3.0), k(0.6, 0.0, -8.0, 0.0)],
            )
            // Shoulders counter-rotate against the hips.
            .rot("spine", vec![k(0.0, 4.0, 7.0, 0.0), k(0.3, 4.0, -7.0, 0.0), k(0.6, 4.0, 7.0, 0.0)])
            .rot("chest", vec![k(0.0, 8.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, -6.0, 0.0, 0.0)])
            // Rifle carried, bobbing slightly with each step.
            .rot("upper_arm_r", c(38.0, 33.0, 38.0, 33.0))
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0)])
            .rot("upper_arm_l", vec![k(0.0, 47.0, 0.0, 20.0), k(0.15, 42.0, 0.0, 20.0), k(0.3, 47.0, 0.0, 20.0), k(0.45, 42.0, 0.0, 20.0), k(0.6, 47.0, 0.0, 20.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0)])
            .0,
    );

    // Hunker: crouched behind cover, breathing, peeking.
    out.push(
        Anim::new("hunker", 2.0, true)
            .carry()
            .mov("hips", vec![k(0.0, 0.0, -0.22, 0.0)])
            .rot("thigh_l", vec![k(0.0, 70.0, 0.0, -5.0)])
            .rot("thigh_r", vec![k(0.0, 70.0, 0.0, 5.0)])
            .rot("shin_l", vec![k(0.0, -115.0, 0.0, 0.0)])
            .rot("shin_r", vec![k(0.0, -115.0, 0.0, 0.0)])
            .rot("foot_l", vec![k(0.0, 45.0, 0.0, 0.0)])
            .rot("foot_r", vec![k(0.0, 45.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 22.0, 0.0, 0.0), k(1.0, 25.0, 0.0, 0.0), k(2.0, 22.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, -15.0, 0.0, 0.0), k(1.0, -12.0, 8.0, 0.0), k(2.0, -15.0, 0.0, 0.0)])
            .0,
    );

    // Shoot: settle into the aim, fire with a kick, recover, hold, lower.
    let mut shoot = Anim::new("shoot", 0.75, false).upper_body();
    for (b, keys) in raise(0.15, 0.55, 0.75) {
        shoot = shoot.rot(b, keys);
    }
    out.push(
        shoot
            .rot(
                "upper_arm_r",
                vec![
                    k(0.0, 35.0, 0.0, 0.0),
                    k(0.15, 85.0, 0.0, 0.0),
                    k(0.28, 85.0, 0.0, 0.0),
                    k(0.32, 72.0, 0.0, 0.0),
                    k(0.42, 85.0, 0.0, 0.0),
                    k(0.55, 85.0, 0.0, 0.0),
                    k(0.75, 35.0, 0.0, 0.0),
                ],
            )
            .rot(
                "chest",
                vec![
                    k(0.0, 0.0, 0.0, 0.0),
                    k(0.15, 6.0, 0.0, 0.0),
                    k(0.28, 6.0, 0.0, 0.0),
                    k(0.32, -7.0, 0.0, 0.0),
                    k(0.45, 6.0, 0.0, 0.0),
                    k(0.55, 6.0, 0.0, 0.0),
                    k(0.75, 0.0, 0.0, 0.0),
                ],
            )
            .rot("spine", vec![k(0.0, 0.0, 0.0, 0.0), k(0.15, 0.0, -10.0, 0.0), k(0.55, 0.0, -10.0, 0.0), k(0.75, 0.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, 0.0, 0.0, 0.0), k(0.15, 8.0, 8.0, 0.0), k(0.32, 4.0, 8.0, 0.0), k(0.55, 8.0, 8.0, 0.0), k(0.75, 0.0, 0.0, 0.0)])
            .0,
    );

    let mut overwatch = Anim::new("overwatch", 0.7, false).upper_body();
    for (b, keys) in raise(0.2, 0.55, 0.7) {
        overwatch = overwatch.rot(b, keys);
    }
    out.push(overwatch.rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.2, 8.0, 0.0, 0.0), k(0.7, 0.0, 0.0, 0.0)]).0);

    // Throw: wind-up, release, follow-through.
    out.push(
        Anim::new("throw", 0.9, false)
            .upper_body()
            .rot(
                "upper_arm_r",
                vec![k(0.0, 35.0, 0.0, 0.0), k(0.35, -150.0, 0.0, -10.0), k(0.47, 130.0, 0.0, 0.0), k(0.6, 90.0, 0.0, 0.0), k(0.9, 35.0, 0.0, 0.0)],
            )
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0), k(0.35, 70.0, 0.0, 0.0), k(0.47, 5.0, 0.0, 0.0), k(0.6, 20.0, 0.0, 0.0), k(0.9, 55.0, 0.0, 0.0)])
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(0.35, 90.0, 0.0, -20.0), k(0.47, 40.0, 0.0, 30.0), k(0.9, 45.0, 0.0, 20.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -18.0, -25.0, 0.0), k(0.47, 22.0, 25.0, 0.0), k(0.6, 15.0, 15.0, 0.0), k(0.9, 0.0, 0.0, 0.0)])
            .rot("spine", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, 0.0, -15.0, 0.0), k(0.47, 0.0, 15.0, 0.0), k(0.9, 0.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, 5.0, 20.0, 0.0), k(0.47, 5.0, -10.0, 0.0), k(0.9, 0.0, 0.0, 0.0)])
            .0,
    );

    out.push(
        Anim::new("reload", 1.0, false)
            .upper_body()
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(0.3, 20.0, 0.0, 30.0), k(0.6, 60.0, 0.0, 10.0), k(1.0, 45.0, 0.0, 20.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0), k(0.3, 110.0, 0.0, 0.0), k(0.6, 60.0, 0.0, 0.0), k(1.0, 50.0, 0.0, 0.0)])
            .rot("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0), k(0.3, 25.0, 0.0, -10.0), k(0.8, 25.0, 0.0, -10.0), k(1.0, 35.0, 0.0, 0.0)])
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, 20.0, 0.0, 0.0), k(0.8, 20.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .0,
    );

    out.push(
        Anim::new("heal", 1.0, false)
            .mov("hips", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, 0.0, -0.22, 0.0), k(0.75, 0.0, -0.22, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("thigh_l", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, 70.0, 0.0, 0.0), k(0.75, 70.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("thigh_r", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, 70.0, 0.0, 0.0), k(0.75, 70.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("shin_l", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, -115.0, 0.0, 0.0), k(0.75, -115.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("shin_r", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, -115.0, 0.0, 0.0), k(0.75, -115.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.3, 25.0, 0.0, 0.0), k(0.75, 25.0, 0.0, 0.0), k(1.0, 0.0, 0.0, 0.0)])
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(0.4, 70.0, 0.0, 10.0), k(0.6, 60.0, 0.0, 15.0), k(1.0, 45.0, 0.0, 20.0)])
            .rot("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0), k(0.4, 70.0, 0.0, -10.0), k(0.6, 60.0, 0.0, -15.0), k(1.0, 35.0, 0.0, 0.0)])
            .0,
    );

    out.push(
        Anim::new("cast", 0.9, false)
            .upper_body()
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(0.35, 165.0, 0.0, -20.0), k(0.6, 165.0, 0.0, -20.0), k(0.9, 45.0, 0.0, 20.0)])
            .rot("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0), k(0.35, 165.0, 0.0, 20.0), k(0.6, 165.0, 0.0, 20.0), k(0.9, 35.0, 0.0, 0.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0), k(0.35, 0.0, 0.0, 0.0), k(0.9, 50.0, 0.0, 0.0)])
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0), k(0.35, 0.0, 0.0, 0.0), k(0.9, 55.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -12.0, 0.0, 0.0), k(0.9, 0.0, 0.0, 0.0)])
            .0,
    );

    // Hit: knocked back and flinching, added on top of anything.
    out.push(
        Anim::new("hit", 0.45, false)
            .additive()
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.06, -22.0, 0.0, 8.0), k(0.2, -8.0, 0.0, 3.0), k(0.45, 0.0, 0.0, 0.0)])
            .rot("head", vec![k(0.0, 0.0, 0.0, 0.0), k(0.06, -20.0, 8.0, 0.0), k(0.45, 0.0, 0.0, 0.0)])
            .rot("spine", vec![k(0.0, 0.0, 0.0, 0.0), k(0.06, -6.0, 0.0, -10.0), k(0.45, 0.0, 0.0, 0.0)])
            .mov("hips", vec![k(0.0, 0.0, 0.0, 0.0), k(0.06, 0.0, 0.0, 0.06), k(0.45, 0.0, 0.0, 0.0)])
            .0,
    );

    // Death: stagger back, knees buckle, fall to the side, settle. Held.
    out.push(
        Anim::new("death", 1.6, false)
            .mov(
                "hips",
                vec![
                    k(0.0, 0.0, 0.0, 0.0),
                    k(0.35, 0.0, -0.02, 0.12),
                    k(0.8, 0.0, -0.3, 0.12),
                    k(1.3, 0.15, -0.43, 0.15),
                    k(1.45, 0.15, -0.4, 0.15),
                    k(1.6, 0.15, -0.43, 0.15),
                ],
            )
            .rot("hips", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -8.0, 0.0, 0.0), k(0.8, 10.0, 0.0, 0.0), k(1.3, 10.0, 0.0, 85.0), k(1.6, 10.0, 0.0, 85.0)])
            .rot("thigh_l", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, 10.0, 0.0, 0.0), k(0.8, 65.0, 0.0, -5.0), k(1.3, 30.0, 0.0, -10.0), k(1.6, 30.0, 0.0, -10.0)])
            .rot("thigh_r", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -5.0, 0.0, 0.0), k(0.8, 60.0, 0.0, 5.0), k(1.3, 45.0, 0.0, 5.0), k(1.6, 45.0, 0.0, 5.0)])
            .rot("shin_l", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -5.0, 0.0, 0.0), k(0.8, -110.0, 0.0, 0.0), k(1.3, -40.0, 0.0, 0.0), k(1.6, -40.0, 0.0, 0.0)])
            .rot("shin_r", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -15.0, 0.0, 0.0), k(0.8, -105.0, 0.0, 0.0), k(1.3, -70.0, 0.0, 0.0), k(1.6, -70.0, 0.0, 0.0)])
            .rot("chest", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -22.0, 0.0, 0.0), k(0.8, 30.0, 0.0, 0.0), k(1.3, 5.0, 0.0, -10.0), k(1.6, 5.0, 0.0, -10.0)])
            .rot("head", vec![k(0.0, 0.0, 0.0, 0.0), k(0.35, -25.0, 0.0, 0.0), k(0.8, 25.0, 0.0, 0.0), k(1.3, 0.0, 20.0, -15.0), k(1.6, 0.0, 20.0, -15.0)])
            .rot("upper_arm_l", vec![k(0.0, 45.0, 0.0, 20.0), k(0.35, 60.0, 0.0, -40.0), k(0.8, 20.0, 0.0, -10.0), k(1.3, 10.0, 0.0, -60.0), k(1.6, 10.0, 0.0, -60.0)])
            .rot("upper_arm_r", vec![k(0.0, 35.0, 0.0, 0.0), k(0.35, 50.0, 0.0, 40.0), k(0.8, 15.0, 0.0, 10.0), k(1.3, -20.0, 0.0, 70.0), k(1.6, -20.0, 0.0, 70.0)])
            .rot("forearm_l", vec![k(0.0, 50.0, 0.0, 0.0), k(0.35, 20.0, 0.0, 0.0), k(1.3, 10.0, 0.0, 0.0), k(1.6, 10.0, 0.0, 0.0)])
            .rot("forearm_r", vec![k(0.0, 55.0, 0.0, 0.0), k(0.35, 20.0, 0.0, 0.0), k(1.3, 15.0, 0.0, 0.0), k(1.6, 15.0, 0.0, 0.0)])
            .0,
    );

    out
}

/// Registers the skeleton and its animations.
pub fn register(rules: &mut Ruleset) {
    rules.add_skeleton(skeleton());
    for a in animations() {
        rules.add_animation(a);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn humanoid_is_valid_and_complete() {
        let mut rules = Ruleset::new();
        register(&mut rules);
        let s = rules.resolve_skeleton(HUMANOID).unwrap();
        assert_eq!(s.bones.len(), 18);
        assert_eq!(s.bones[0].name, "hips");
        let anims = rules.skeleton_animations(HUMANOID);
        for slot in ["idle", "walk", "hunker", "shoot", "overwatch", "throw", "reload", "heal", "cast", "hit", "death"] {
            assert!(anims.contains_key(slot), "missing {slot}");
        }
        assert!(rules.validate().is_empty(), "{:?}", rules.validate());
        // The upper-body mask covers the arms and weapon but not the legs.
        let upper = s.subtree(&["spine".into()]);
        assert!(upper.contains(&s.index_of("weapon").unwrap()));
        assert!(!upper.contains(&s.index_of("thigh_l").unwrap()));
        // Feet rest on the ground: hips 0.55 + thigh -0.05 + shin -0.27 + foot -0.23 = 0.
        let foot: f32 = ["hips", "thigh_l", "shin_l", "foot_l"].iter().map(|b| s.bones[s.index_of(b).unwrap()].offset[1]).sum();
        assert!(foot.abs() < 1e-4);
    }

    #[test]
    fn looping_animations_close_their_loop() {
        // A loop's first and last keys must match, or it pops each cycle.
        for a in animations().into_iter().filter(|a| a.looping) {
            for (bone, keys) in a.rotations.iter().chain(a.translations.iter()) {
                if keys.len() > 1 {
                    assert_eq!(keys.first().unwrap().1, keys.last().unwrap().1, "{}/{bone}", a.slot);
                    assert!((keys.last().unwrap().0 - a.duration).abs() < 1e-4, "{}/{bone} ends at the duration", a.slot);
                }
            }
        }
    }

    #[test]
    fn rest_pose_of_bones_without_keys_is_unchanged() {
        let a: BTreeMap<_, _> = animations().into_iter().map(|a| (a.slot.clone(), a)).collect();
        assert!(!a["shoot"].sample(0.3).contains_key("thigh_l"), "shoot leaves the legs to the base layer");
    }
}

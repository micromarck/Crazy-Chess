# Skeletons and animation

Units are animated by a **standard skeleton** with a full set of basic
movements. You can put **custom skeletons and animations on top** of it in
data, without touching Rust.

## The standard skeleton: `humanoid`

Defined in `crates/tactics_core/src/modules/humanoid.rs` (registered by the
base module). 18 bones:

```
                       head
                        |
                       neck
                        |
 hand_l─forearm_l─upper_arm_l──chest──upper_arm_r─forearm_r─hand_r─weapon
                        |
                       spine
                        |
                       hips   (root, 0.55 m up)
                     /      \
                 thigh_l   thigh_r
                    |         |
                 shin_l    shin_r
                    |         |
                 foot_l    foot_r
```

Its animations cover every slot the game plays automatically:

| slot | kind | when |
|---|---|---|
| `idle` | loop | standing |
| `walk` | loop | moving |
| `hunker` | loop | while defending |
| `shoot` | upper body | Shoot, Aimed Shot, reaction fire |
| `overwatch` | upper body | Overwatch |
| `throw` | upper body | grenades / tile-targeted attacks |
| `reload` | upper body | Reload |
| `heal` | full body | Medkit |
| `cast` | upper body | psionic abilities (Psi Bolt) |
| `hit` | additive | taking damage |
| `death` | full body, held | dying |

Soldiers (`soldier` model) and aliens (`alien` model) use it out of the box,
drawn as simple shapes on the bones: no art needed.

## How it plays

```
base layer     idle / walk / hunker (loops), cross-faded when it changes
   +
overlay layer  one-shots: shoot, throw, hit, death, custom slots...
                 replace  → only bones the clip animates, within its mask
                 additive → added on top of the current pose
   =
bone pose      rest offset + translation, rest rotation × rotation
```

* **Masks:** `mask: ["spine"]` limits a clip to that bone and everything
  below it. All the standard upper-body clips use it, so a unit fires back
  at overwatch **while still walking**.
* **Additive:** `additive: true` adds the clip to whatever is playing (the
  standard `hit` flinch works during idle, walk or a shot).
* Overlays fade in and out quickly; `death` is held.
* A slot the skeleton doesn't define falls back to a procedural motion on the
  whole model (see `procedural_pose` in `tactics_game/src/animation.rs`),
  so no ability ever freezes a unit.

## Custom skeletons on top

Example (complete): `crates/tactics_game/assets/modules/creatures.ron`, a
**sectoid** with a big head, long thin arms, antennae and glowing eyes.

```ron
skeletons: [
    (
        id: "sectoid",
        extends: "humanoid",          // inherit all bones AND all animations
        bones: [
            // same name = override (proportions, shape, colour)
            (name: "head", parent: "neck", offset: (0.0, 0.07, 0.0),
             shape: Sphere(radius: 0.2), shape_offset: (0.0, 0.16, 0.0), color: Light),
            // new name = additional bone
            (name: "antenna_l", parent: "head", offset: (-0.08, 0.3, 0.0), rotation: (0.0, 0.0, 25.0),
             shape: Capsule(radius: 0.015, length: 0.16), shape_offset: (0.0, 0.1, 0.0), color: Accent),
        ],
    ),
],
models: [(id: "alien", skeleton: "sectoid", fallback: Alien)],
```

* Chains work: a `sectoid_elite` can `extends: "sectoid"`.
* Bone fields: `name`, `parent`, `offset` (from the parent, metres),
  `rotation` (rest pose, degrees), `shape` (`None`, `Box(size: (x, y, z))`,
  `Capsule(radius, length)`, `Sphere(radius)`), `shape_offset`, `color`
  (`Team`, `Dark`, `Light`, `Accent`).
* A skeleton doesn't have to be a humanoid. Leave out `extends` and define
  any hierarchy (quadruped, turret, tentacle...). It then needs its own
  animations for the slots it should play.

Conventions: metres, Y up, the model faces **-Z**. Rotations are Euler
degrees applied X, then Y, then Z, relative to the parent. For a limb hanging
down, **+X swings it forward** and +Z swings it towards +X (the unit's right).

## Custom animations on top

```ron
animations: [
    (
        skeleton: "sectoid",          // replaces sectoid/cast only; everything else inherited
        slot: "cast",
        duration: 1.0,
        looping: false,
        mask: ["spine"],              // upper body
        additive: false,
        easing: Smooth,               // or Linear
        rotations: {
            // bone: [(time, (x, y, z) degrees), ...]
            "upper_arm_l": [(0.0, (10.0, 0.0, -8.0)), (0.3, (60.0, 0.0, -85.0)), (1.0, (10.0, 0.0, -8.0))],
            "antenna_l":   [(0.0, (0.0, 0.0, 0.0)),   (0.3, (0.0, 0.0, 40.0)),   (1.0, (0.0, 0.0, 0.0))],
        },
        translations: {
            // bone: [(time, (x, y, z) metres added to the rest offset)]
        },
    ),
],
```

* **Override** a standard animation for one skeleton: same `slot`, your
  skeleton's id. Children of that skeleton inherit your version.
* **New slots**: any name works (`creatures.ron` adds `taunt`). Use it from
  an ability with `presentations: [(ability: "my_move", animation: "taunt")]`.
* Bones without keys in a clip keep their pose from the layer below.
* For loops, make the first and last keys equal and end the last key at
  `duration` (the base module's tests check this for the standard set).

## glTF models

Models with a glTF `scene` play their own clips per slot (see MODULES.md).
If such a model also has a `skeleton`, the rig is shown while the glTF
loads, or permanently if the file is missing. Slots with neither a clip nor
a skeleton animation use the procedural fallback.

## Checks

`Ruleset::validate()` (shown in the combat log at startup) reports unknown
parents, `extends` loops, animations for unknown skeletons and keyframes
naming bones the skeleton doesn't have. `cargo test` covers inheritance,
sampling, masks, additive layering, cross-fades and the standard set.

## Where the code is

| file | what |
|---|---|
| `tactics_core/src/content/skeleton.rs` | data types, inheritance (`resolve_skeleton`, `skeleton_animations`), keyframe sampling |
| `tactics_core/src/modules/humanoid.rs` | the standard skeleton and its movement set |
| `tactics_game/src/rig.rs` | spawning bone entities, layered pose evaluation |
| `tactics_game/src/animation.rs` | picks base/overlay slots per unit, glTF and procedural fallbacks |

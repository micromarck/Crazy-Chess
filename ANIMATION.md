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

## Drop-in characters (rig in Blender, drop in, test)

Put a rigged, animated `.glb` / `.gltf` into **`crates/tactics_game/assets/characters/`**
and start the game. No code and no config needed:

1. The file becomes a model named after it (`robot.gltf` → `robot`).
2. Its animation clips are mapped to slots **by name**:

   | slot | clip names that map to it (case/punctuation ignored) |
   |---|---|
   | `idle` | Idle, Stand, Breathe, Rest |
   | `walk` | Walk, Walking, Run, Running, Jog, Move, Locomotion |
   | `shoot` | Shoot, Fire, Attack, Shot |
   | `overwatch` | Aim, Aiming |
   | `throw` | Throw, Toss, Grenade |
   | `reload` | Reload |
   | `hit` | Hit, Hurt, Damage, Get Hit, Flinch, Impact |
   | `death` | Death, Die, Dying, Dead |
   | `hunker` | Crouch, Cover, Hunker, Crouch Idle |
   | `heal` | Heal, Interact, Use, Kneel |
   | `cast` | Cast, Spell, Magic, Psi |

   Exporter prefixes are ignored (`Armature|Run`, `rig:Walk`, `Take 001/Fire`).
   Any other clip becomes a custom slot with its own name (`Wave` → `wave`)
   that abilities can use. Slots the character lacks fall back to procedural
   motion, so a model with only an idle still works.
3. It is **auto-fitted**: scaled to soldier height (1.3 m) with its lowest
   point on the ground, so centimetre-scale Mixamo exports and oversized
   models just work.
4. It is turned 180°, because glTF characters face +Z and the game faces -Z.
5. Press **F7** (or Missions → *Character Lab*) to test it.

### The Character Lab

A quiet stage with one test subject, a crate and a wall for scale. The left
panel lists **every model** (built-in rigs, module glTFs, dropped-in
characters) with where it comes from. Click one to put it on the subject.
Below it, every **animation slot** the model has is listed with its source
(`clip Fire`, `skeleton`); click to play. *walk loop* toggles walking in
place so you can judge the cycle. Right-drag orbits around the subject and
the wheel zooms. The combat log shows how each character's clips were mapped
(`Fire->shoot, Armature|Die->death, ...`).

### Optional settings: `<name>.character.ron`

```ron
(
    name: "Test Robot",          // display name
    height: 1.35,                // auto-fit target (metres)
    // scale: 0.01,              // fixed scale instead of auto-fit
    yaw_degrees: 180.0,          // facing correction (glTF default is +Z)
    offset: (0.0, 0.0, 0.0),
    slots: { "mixamo.com": "idle" },   // clip -> slot overrides
    looping: ["wave"],           // extra looping slots
    replace: ["soldier"],        // use this character for these models in battle
)
```

`replace` is how a character goes into battle: `["soldier"]` turns every
soldier into it, `["alien"]` every sectoid. Any model id works (see the Lab
list). To give it its own class instead, set a class's `model` to the
character's id in a module.

### Exporting from Blender

* File → Export → glTF 2.0 (`.glb`), with **Animation** enabled and each
  action exported as its own clip. Name actions after the slots (Idle, Walk,
  Shoot...).
* Keep Blender's default orientation (+Y up on export). Model the character
  facing Blender's **-Y** (front view); it then faces +Z in glTF, which the
  default `yaw_degrees: 180` handles.
* Any skeleton works; the game plays the clips the file contains. Bone
  names don't matter.
* Mixamo: download each animation as FBX "with skin", combine in Blender,
  export one `.glb`. Mixamo names every clip `mixamo.com`; rename the
  actions or map them with `slots`.

### Clips that only animate some bones

Each frame the character's joints are first reset to their rest pose, then
the playing clips are applied. A clip with no hip track ("Walk") therefore
never inherits the fallen hips of the clip before it ("Die"). Author clips
for the bones they move; everything else stays at rest.

### Web and Android

These platforms can't list folders, so add the file name to
`assets/characters/index.txt` (one per line). Desktop scans the folder
automatically.

### Reference character

`assets/characters/robot.gltf` is generated by `tools/gen_robot_gltf.py`: a
real skinned mesh (12 joints, inverse bind matrices) exported the "wrong"
way on purpose (2.2 units tall, facing +Z) with clips named `Idle`, `Walk`,
`Fire`, `Armature|Die` and `Wave`. Use it to check the pipeline, or as a
template for generated art.

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
| `tactics_game/src/characters.rs` | drop-in characters: discovery, clip-name mapping, sidecar settings |
| `tactics_game/src/models.rs` | model library, glTF spawning, auto-fit |
| `tactics_game/src/lab.rs` | the Character Lab |

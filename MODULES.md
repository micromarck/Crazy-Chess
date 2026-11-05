# Modules: adding and editing game content

Everything you can tune lives in a **`Ruleset`** that is built from
**modules**: abilities ("moves"), weapons, unit classes, squads, maps, AI
behaviour, the global combat rules, plus the presentation layer (3D models,
animation clips, camera scripts and per-ability visuals). The rules engine (`tactics_core::battle`) only reads
the ruleset, so new content never requires touching the rules code.

```
 BaseModule (Rust) ─┐
 GrenadierModule ───┤  later modules        ┌─────────┐      ┌────────┐
 marksman.ron ──────┼─ override earlier ──► │ Ruleset │ ───► │ Battle │
 modules/*.ron ─────┤  content by id        └─────────┘      └────────┘
 editor save ───────┘                            ▲ live edits (F1 editor)
```

There are three ways to add content:

| Way | Good for | Where |
|---|---|---|
| **Data module** (`.ron` file) | new moves/weapons/classes and rebalancing, no Rust needed | `crates/tactics_game/assets/modules/` (bundled) or `modules/` next to the game (desktop) |
| **Rust module** | everything a data module does, plus logic (generated content, conditional changes) | any file implementing `GameModule` |
| **In-game editor** (F1) | tuning numbers while playing | saves `modules/zz_editor_tuning.ron` |

Load order is set in `crates/tactics_game/src/content.rs`:
1. Rust modules from `rust_modules()` (base game first),
2. bundled RON modules from `BUNDLED_DATA_MODULES`,
3. desktop only: every `modules/*.ron`, sorted by file name (the editor's
   `zz_editor_tuning.ron` therefore loads last).

**Registering something with an id that already exists replaces it.** That is
how a module rebalances existing content.

---

## Quick start: a new move as data

Create `crates/tactics_game/assets/modules/my_moves.ron`:

```ron
(
    info: (id: "my_moves", name: "My Moves"),
    abilities: [
        (
            id: "snap_shot",
            name: "Snap Shot",
            description: "Quick 1 AP shot: -10 aim, cover hurts more.",
            cost: Points(1),          // or EndsTurn
            ammo_cost: 1,
            targeting: Enemy,          // SelfOnly | Enemy | Ally | Tile
            range: Weapon,             // or Fixed(8.0)
            requires_los: true,
            effect: Attack(
                accuracy_bonus: -10,
                half_cover_penalty: 25,
                full_cover_penalty: 45,
            ),
        ),
    ],
    grants: [(class: "rifleman", ability: "snap_shot")],
)
```

Then add it to the bundle list in `src/content.rs`:

```rust
pub const BUNDLED_DATA_MODULES: &[(&str, &str)] = &[
    ("marksman.ron", include_str!("../assets/modules/marksman.ron")),
    ("my_moves.ron", include_str!("../assets/modules/my_moves.ron")),
];
```

On desktop you can skip that step: drop the file into a `modules/` folder in
the directory you run the game from. The rifleman now has a **Snap Shot**
button and hotkey, the AI uses it too, and the hover panel shows its odds
with a full modifier breakdown.

Every field is optional. Anything you leave out keeps its default (tables below).

## Quick start: a Rust module

Copy `crates/tactics_core/src/modules/grenadier.rs`. In short:

```rust
use tactics_core::content::*;

pub struct MyModule;

impl GameModule for MyModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo::new("my_module", "My Module")
    }

    fn register(&self, rules: &mut Ruleset) {
        rules.add_ability(
            AbilityDef::new("snap_shot", "Snap Shot")
                .with_cost(ApCost::Points(1))
                .with_ammo_cost(1)
                .with_targeting(Targeting::Enemy, AbilityRange::Weapon)
                .with_effect(AbilityEffect::Attack(AttackProfile {
                    accuracy_bonus: -10,
                    half_cover_penalty: 25,
                    full_cover_penalty: 45,
                    ..Default::default()
                })),
        );
        rules.grant_ability("rifleman", "snap_shot");

        // Tweak existing content too:
        if let Some(shoot) = rules.ability_mut(&"shoot".into()) {
            shoot.description.push_str(" (modded)");
        }
    }
}
```

Register it in `rust_modules()` in `crates/tactics_game/src/content.rs`.
`Ruleset::validate()` reports broken references (unknown weapon/ability/class)
and shows them in the combat log at startup.

---

## Models, animations and camera scripts

Presentation lives in the same modules but is **never read by the rules
engine**, so battles stay deterministic and headless tests never load assets.
The full working example is `crates/tactics_game/assets/modules/drones.ron`
(an animated glTF drone with its own laser camera).

### Custom models (`models`)

```ron
models: [
    (
        id: "drone",                              // referenced by a class's `model`
        scene: "models/drone.gltf#Scene0",        // glTF/GLB path under assets/, with label
        skeleton: None,                           // or a skeleton id: drawn as an animated rig (ANIMATION.md)
        fallback: Drone,                          // Soldier | Alien | Drone: shown while loading / if missing
        scale: 1.0,
        offset: (0.0, 0.0, 0.0),                  // from the tile centre, y up
        yaw_degrees: 0.0,                         // turn the model so it faces -Z
        animations: {
            "idle":  (clip: "models/drone.gltf#Animation0", looping: true),
            "walk":  (clip: "models/drone.gltf#Animation1", looping: true, speed: 1.2),
            "shoot": (clip: "models/drone.gltf#Animation2"),
            "hit":   (clip: "models/drone.gltf#Animation3"),
            "death": (clip: "models/drone.gltf#Animation4"),
        },
    ),
],
classes: [(id: "drone", model: "drone", /* ... */)],
```

Model conventions: origin on the ground at the tile centre, 1 unit = 1 tile,
facing **-Z** (fix with `yaw_degrees`). Put files in
`crates/tactics_game/assets/models/` (shipped next to the executable on
desktop, copied by Trunk for the web, packaged by `cargo apk` for Android).
`tools/gen_drone_gltf.py` shows how the bundled drone was made, if you want to
generate placeholder art.

### Animation slots

| slot | played when | procedural fallback |
|---|---|---|
| `idle` | default (loop) | breathing |
| `walk` | moving (loop) | bob + lean |
| `hunker` | while defending (loop) | crouch |
| `shoot` / `throw` / `overwatch` / `reload` / `heal` | default for that ability effect | recoil / wind-up / brace / tilt / kneel |
| `cast` | psionic-style abilities (by name) | rise + spin |
| `hit` | when damaged | flinch |
| `death` | when killed (held) | topple + sink |

A model only needs the clips it has: every missing slot falls back to the
procedural animation, so primitive models and half-animated models both work.
Abilities can name **any** slot (`animation: "slash"`). Add a procedural
version in `procedural_pose` (`crates/tactics_game/src/animation.rs`) or
provide a clip on the model.

### Ability visuals (`presentations`)

```ron
presentations: [
    (
        ability: "laser",
        animation: "shoot",       // slot on the user (default depends on the effect)
        camera: "laser_cam",      // camera script to run
        camera_chance: 60,        // % of uses that get the cinematic
        projectile: Beam,         // Tracer | Beam | Arc | Rocket | None
        windup: 0.25,             // seconds from animation start to the effect
    ),
],
presentation_settings: (kill_camera: "kill_cam", kill_camera_chance: 100),
```

### Camera scripts (`camera_scripts`)

A script is a list of steps run in order; `Together([...])` runs steps at the
same time. Anchors: `Actor` (who acts), `Target` (unit or tile it's aimed at;
for the kill cam, the victim), `Midpoint`, `Current`.

| Step | Effect |
|---|---|
| `Focus(on: Target, duration: 0.4)` | move the look-at point |
| `FaceAlong(from: Actor, to: Target, offset_degrees: 25.0, duration: 0.4)` | swing behind `from` looking at `to` (over the shoulder) |
| `Orbit(degrees: 40.0, duration: 1.0)` | rotate around the focus point |
| `Zoom(distance: 8.0, duration: 0.4)` | distance from the focus point |
| `Pitch(degrees: 25.0, duration: 0.4)` | angle above the horizon (90 = top-down) |
| `Shake(strength: 0.3, duration: 0.5)` | camera shake, fading out |
| `TimeScale(scale: 0.35)` | slow motion (reset by `Restore` / end of script) |
| `Wait(duration: 1.0)` | hold |
| `Restore(duration: 0.5)` | glide back to the player's camera |

```ron
camera_scripts: [
    (
        id: "laser_cam",
        steps: [
            Together([Focus(on: Actor, duration: 0.3), Zoom(distance: 7.0, duration: 0.3)]),
            Together([
                Focus(on: Midpoint, duration: 0.5),
                FaceAlong(from: Actor, to: Target, offset_degrees: -30.0, duration: 0.5),
                Pitch(degrees: 20.0, duration: 0.5),
            ]),
            Wait(duration: 0.8),
            Restore(duration: 0.5),
        ],
    ),
],
```

Built-in scripts (base module): `over_shoulder`, `kill_cam`, `explosion_cam`,
`focus_actor`. While a script runs, the player's camera controls and the game
pause. **Space** skips the script and **C** turns cinematic cameras on/off.
Durations are in real time, so slow motion doesn't stretch them.

From Rust you can run any script directly (cutscenes, mission intros):

```rust
fn intro(mut director: ResMut<CameraDirector>, state: Res<BattleState>) {
    if let Some(script) = state.battle.rules().camera_script("over_shoulder") {
        director.play(script, CameraContext { actor: Vec3::ZERO, target: Vec3::new(10.0, 0.0, -10.0) });
    }
}
```

Known limitation: the camera has no collision, so a scripted angle can end up
behind a wall. Adding a ray-cast from the focus point that shortens the
distance would fix it (`camera.rs`, `apply_rig`).

To add a new **step type**: add a variant to `CameraStep`
(`tactics_core/src/content/presentation.rs`) and handle it in `Running::start`
and `Running::apply` (`tactics_game/src/camera_director.rs`).

---

## Reference

### Ability (`AbilityDef`)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — | Unique id. Same id = replace. |
| `name`, `description` | string | `""` | Button label and tooltip. |
| `cost` | `Points(n)` \| `EndsTurn` | `EndsTurn` | AP cost. `EndsTurn` needs 1 AP and spends all. |
| `ammo_cost` | int | `0` | Rounds used from the weapon magazine. |
| `cooldown` | int | `0` | Own turns before the move can be used again. |
| `charges` | int or none | none (unlimited) | Uses per battle. |
| `targeting` | `SelfOnly` \| `Enemy` \| `Ally` \| `Tile` | `SelfOnly` | What it is aimed at. |
| `range` | `Weapon` \| `Fixed(tiles)` | `Weapon` | `Weapon` = weapon max range capped by sight. |
| `requires_los` | bool | `true` | Must see the target (tile). |
| `effect` | see below | `Attack(())` | What happens. |
| `heat` | int | `0` | Heat it adds to a user that runs on heat (on top of the weapon's `heat` for weapon attacks). See *Heat* below. |

### Effects (`AbilityEffect`)

| Effect | Fields | Notes |
|---|---|---|
| `Attack(profile)` | `AttackProfile` | Single target, or area if `area_radius > 0`. |
| `Overwatch(profile)` | `AttackProfile` | Arms a reaction shot using this profile. |
| `Defend(defense_bonus: n)` | int | Hunker: +defence until your next turn. |
| `Suppress(profile)` | `AttackProfile` | Fires a long burst (`rounds`) at the target and pins it: −`combat.suppression_aim` aim and +`combat.suppression_fear` fear, and the gunner fires again if it moves. Lasts until the gunner's next turn. |
| `Reload` | — | Refills the magazine from a spare one (fails with no spares left). |
| `Heal(amount: n)` | int | Restores HP, never above max. Not on machines (units that run on heat). |
| `Shutdown(turns: n)` | int | Machines only: offline for the user's next `n` turns, then reboots with **all heat vented**. Any damage while shut down destroys it. |
| `Cool(amount: n)` | int | Machines only: sheds `n` heat (coolant). |
| `Overclock` | — | For the rest of the user's turn: its next turn-ending attack costs only 1 AP (an extra shot) and all heat it makes is doubled. |
| `Idle` | — | Does nothing (with `cost: EndsTurn`, ends the turn): robots idling to cool off without making heat. |
| `Leap(reach: r, over_walls: b)` | float, bool | Target a tile: move there in a straight line, up to `r` times the unit's normal move. `over_walls: true` flies over anything (jump pack); `false` is stopped by walls, holes, units and steep steps (boost thrusters). Overwatch can fire at the landing. |

### Attack stats (`AttackProfile`)

| Field | Default | Meaning |
|---|---|---|
| `base_accuracy` | none (use unit `aim`) | Fixed starting hit chance, e.g. `75`. |
| `accuracy_bonus` | `0` | Added to the base (negative for snap/reaction shots). |
| `half_cover_penalty` | `20` | Subtracted when the target is in half cover. |
| `full_cover_penalty` | `40` | Subtracted when the target is in full cover. |
| `range_falloff` | `true` | Apply weapon optimal-range falloff / close bonus. |
| `can_miss` | `true` | `false` = always 100% (grenades). |
| `crit_bonus` | `0` | Added to the weapon's crit chance. |
| `flank_crit_bonus` | `40` | Extra crit vs. flanked (uncovered) targets. |
| `damage` | `Weapon` | Or `Fixed(min: 2, max: 4)`. |
| `damage_bonus` | `0` | Flat damage added on hit. |
| `area_radius` | `0.0` | > 0 = hits every unit within that many tiles (friendly fire!), and damages destructible props and **walls** there too. Cover is checked from the blast centre. |
| `rounds` | weapon `burst` | Rounds fired per use (e.g. `Some(12)` for suppression). |
| `melee` | `false` | A strike in reach (claws, blades): rolled, not traced as a bullet. |
| `shrapnel` | `0` | Area attacks: fragments thrown out of the blast in every direction, traced like bullets up to `area_radius + 5` tiles; 1 damage each to the first unit hit (friend or foe, outside the blast); walls stop them. The Frag Grenade throws 16. |
| `cone` | `0.0` | > 0 = a **cone** this many degrees wide from the user towards the target tile, reaching that far (flamethrowers). Hits everyone in it the user can see, friend or foe; cover counts from the user. |
| `ignite` | `0` | Sets the blast or cone **on fire** for this many rounds: a unit starting its turn in fire takes `combat.fire_damage` (2), each burning tile walked into `combat.fire_step_damage` (1); props there take 1 a round; walls don't burn. |
| `direct` | `false` | Area attacks: the projectile flies **straight** (a rocket) and goes off at the first wall or unit in the way. Pair with `projectile: Rocket` in the presentation. |
| `square` | `false` | Blasts: a **square** reaching `area_radius` tiles out on every side (radius 2 = 5x5) instead of a circle. |
| `fuse` | `0` | Blasts: goes off this many of the thrower's turns later (at the start of its turn) where it landed, not at once: the Particle Grenade (2). The area shows on the board while it ticks; the AI keeps out of it. |

Hit chance:

```
hit  = (base_accuracy or unit aim) + accuracy_bonus
       - target defense - target defend bonus
       - half/full cover penalty (if the target has cover facing the attack)
       ± range modifier (if range_falloff)
       × combat.accuracy_multiplier (1.3 by default; the cover penalty comes after)
       clamped to [combat.min_hit, combat.max_hit] unless can_miss = false
crit = weapon crit (weapon damage only) + crit_bonus + flank_crit_bonus (if flanked)
```

**Shots are bullets.** Single-target attacks that can miss are *ballistic*:
the hit chance above (before cover) sets how wide the shooter's aim error
is, and every round (`burst`) and pellet (`pellets`) is a ray fired with
that error plus the weapon's `spread` (degrees). Rays are traced through the
map: walls, props and **any unit in the way — friend or foe —** stop them;
the target's cover stops a share of the rounds that would hit
(`stops n%` in the tooltip). Damage is per round, so a 4-round burst of
1–2 damage does 0–8. The tooltip shows "4 rounds x 38%" (chance per round).
The result is the `Volley` event, one `Bullet` per round. Area attacks and
`can_miss: false` attacks still resolve as before (`AttackResolved`).

### Other content

| Section | Type | Key fields |
|---|---|---|
| `weapons` | `Weapon` | `id, name, damage_min, damage_max, crit_chance, crit_bonus, optimal_range, max_range, clip_size, burst` (rounds per shot, 1), `pellets` (rays per round, 1; shotguns 8), `spread` (extra degrees of scatter, 0.6), `magazines` (spare magazines, 3), `ignores_cover` (shots can't be blocked or penalised by the target's cover; the Shotgun), `min_range` + `close_penalty` (closer than `min_range` tiles: no close bonus and `close_penalty` aim lost per tile; the Sniper Rifle: 5 and 10), `classes` (who may carry it; empty = anyone), `look` (`Rifle`, `Shotgun`, `Sniper`, `Smg`, `Lmg`, `Pistol`, `Plasma`, `Laser`, `Minigun`, `Cannon`, `Flamer`, `None`; `Laser` and `Cannon` fire beams), `model` (custom held model id), `heat` (heat per attack for units that run on heat; 0 = the class's `per_shot`), `cone` + `ignite` (a **flame weapon**: its Shoot/overwatch sweep a cone this many degrees wide out to the target instead of firing bullets, never miss, ignore cover, burn everyone in it and leave fire for `ignite` rounds; can't suppress; look `Flamer`) |
| `classes` | `UnitClass` | `id, name, stats: (max_hp, mobility, aim, defense, sight, climb, bravery), weapon, abilities: [...], model, fearless, loadout`, `hooks` (class scripts, see below), `horde` (mindless swarm AI: close in and strike, cheap enough for hundreds; their deaths play in the background, all at once, without a kill camera), `hearing` (tiles: gunfire this close wakes it while dormant), `body: (hover, height, muzzle)` (hitbox in metres: `hover` lifts flyers like drones off the ground — their shots pass over heads, enemies aim high, half cover beneath them does not count; `muzzle: Some(m)` sets the gun height), `heat: (cooling, per_move, per_shot)` (runs on heat instead of health, see *Heat*), `restricted_gear` (only weapons/items whose `classes` name it: a separate arsenal), `no_half_cover` (too big for low cover: half cover gives it nothing, full cover still does; robot frames), `size` (tiles it takes up: `size` x `size`, default 1; `pos` is the footprint's corner with the lowest x and y. Big units need room for the whole footprint to move or spawn, can be targeted from any of their tiles and take blasts on any; dinosaurs) |
| `squads` | `SquadDef` | `id` (`"player"` / `"enemy"` are deployed), `members: [(class, name, loadout, gear, appearance)]` (the Barracks roster replaces `"player"` in game) |
| `items` | `ItemDef` | `id, name, description, slot (Armor / Utility), stats: (max_hp, mobility, aim, defense, sight, bravery), abilities, charges, classes, model, socket` |
| `loadouts` | `LoadoutDef` | `id, name, description, classes, gear: (weapon, armor, items: [...])` |
| `maps` | `MapDef` | `id` (`"demo"` is used for the first battle), `name`, `ascii` (`.` floor, `h` half, `H` full, `#` void, `A`/`E` spawns) |
| `ai_profiles` | `AiProfile` | `id` (`"default"` drives the enemy), `config: (shoot_threshold, area_min_score, heal_below, ...)` |
| `combat` | `CombatRules` | `min_hit, max_hit, falloff_per_tile, close_bonus_per_tile, max_close_bonus, action_points, height_advantage_bonus, morale: (...)` (see "Morale and fear") |
| `grants` | `AbilityGrant` | `(class, ability)`: add a move to an existing class |
| `models` | `ModelDef` | see "Models, animations and camera scripts" |
| `presentations` | `AbilityPresentation` | `ability, animation, camera, camera_chance, projectile, windup` |
| `camera_scripts` | `CameraScriptDef` | `id, steps` |
| `presentation_settings` | `PresentationSettings` | `kill_camera, kill_camera_chance` |
| `skeletons` | `SkeletonDef` | `id, extends, bones, sockets`; see [ANIMATION.md](ANIMATION.md) |
| `animations` | `AnimationDef` | `skeleton, slot, duration, looping, mask, additive, rotations, translations`; see ANIMATION.md |
| `ui_layouts` | `UiLayoutDef` | `id` (`"hud"` replaces the HUD), `html`; see [UI.md](UI.md) |

A class's `model` is the id of a `ModelDef` (base game: `"soldier"`, `"alien"`).
A `ModelDef` with a glTF `scene` may list `sockets` whose `bone` is a node
name in the file (where weapons and gear attach).

An `appearance` (squad members) looks like
`(build: Heavy, height: 1.05, skin: 2, hair: Short, hair_color: 1, headgear: Helmet, armor: 1, accent: 4, backpack: true)`
(colours are palette indices; see ANIMATION.md, Looks).

### Loadouts (`items`, `loadouts`)

A unit carries a **weapon**, one **armor** item and up to two **utility**
items. Items add stats and grant abilities; an ability with limited
`charges` gets uses per item carried (two frag grenades = two throws).

```ron
items: [
    (id: "smoke", name: "Smoke Grenade", slot: Utility,
     abilities: ["smoke_grenade"], charges: 1),
    (id: "exo", name: "Exo Suit", slot: Armor,
     stats: (max_hp: 5, mobility: -4, defense: 10), classes: ["grenadier"]),
],
loadouts: [
    (id: "tank", name: "Tank", gear: (weapon: "lmg", armor: "exo", items: ["frag", "frag"])),
],
squads: [
    (id: "player", members: [
        (class: "grenadier", name: "Pvt. Kowalski", loadout: "tank"),
        (class: "assault", name: "Cpl. Okoye", loadout: "breacher", gear: (items: ["smoke", "stims"])),
    ]),
],
```

Gear is resolved as: class weapon → class `loadout` preset → the member's
`loadout` preset → the member's `gear` overrides. `armor: ""` / `items: [""]`
mean "explicitly none". Illegal gear (unknown ids, class restrictions, wrong
slot, more than two utility items) is reported by `validate()` and refused
at spawn. In game, the **Loadout** button edits the player squad; mission
units take `loadout`, `weapon`, `armor` and `items` attributes.

Bundled: weapons rifle, shotgun, sniper rifle (snipers only), SMG, LMG;
armor Kevlar Vest, Plated Armor, Ghost Suit; utility Scope, Combat Stims,
Lucky Dog Tags, Frag Grenade, Medkit; presets Standard, Breacher, Recon,
Field Medic, Heavy, Marksman.

### Morale and fear (`combat.morale`)

Units have **fear** (0-100). Taking hits, being shot at, allies dying nearby,
being flanked or outnumbered raise it; kills, hunkering down and quiet turns
in cover lower it. Gains are scaled by **bravery** (stat, default 50; 100 =
immune to gains, 0 = double). Classes with `fearless: true` (the Drone Pack's
drones; any robot you add) never feel fear.

| level | at | effect |
|---|---|---|
| Steady | below `shaken_at` (40) | none |
| Shaken | `shaken_at` | -10 aim; the AI gets cautious |
| Panicked | `panicked_at` (75) | -25 aim, loses 1 AP per turn; the AI flees or hunkers |

This applies to **your soldiers too**. Every number is in `MoraleRules`;
override them in a module's `combat: (morale: (...))`, or set
`enabled: false` to turn morale off.

```ron
combat: (morale: (shaken_at: 30, panicked_at: 60, ally_killed_near: 35)),
```

---

### Heat (robot frames)

A class with `heat` runs on heat instead of health (the Robot Frames
module's Strider and Bastion). Its `max_hp` is the **heat limit** and its
HP is the headroom left (`heat = max_hp - hp`), so:

* **getting hit** adds heat (the damage);
* **its own attacks** add the weapon's `heat` (or the class's `per_shot`),
  plus the ability's `heat` (a missile salvo); overwatch heats it when it fires;
* **moving** adds `per_move` per action point spent;
* at the start of each of its turns it sheds `cooling`;
* at the limit it is **destroyed** (the UI warns before an action would do
  that, and the AI never does it to itself);
* a `Shutdown` ability vents everything after a couple of offline turns,
  but any hit while shut down destroys it; `Cool` abilities (coolant)
  shed some heat at once. Medkits don't work on machines.

```ron
classes: [(
    id: "frame_heavy", name: "Bastion Frame",
    stats: (max_hp: 20, mobility: 8, aim: 62, defense: 20, sight: 13.0),
    weapon: "gatling_laser",
    abilities: ["shoot", "suppress", "overwatch", "shutdown", "reload"],
    heat: (cooling: 3, per_move: 3, per_shot: 4),
    restricted_gear: true,   // its own arsenal only
)],
weapons: [(id: "gatling_laser", name: "Gatling Laser", classes: ["frame_heavy"],
           damage_min: 1, damage_max: 2, burst: 8, heat: 6, look: Minigun)],
```

---

### Class scripts (`hooks`) and shared functions (`script_library`)

A class can carry **scripts** that every unit of it runs, wherever it
appears (hand-made missions, generated maps, `spawn_unit`). They use the
mission scripting language and API (see MISSIONS.md, *Scripts*), with
`unit` = the unit itself:

| `on` | runs when |
|---|---|
| `turn_start` | its team's turn starts |
| `damaged` | it takes damage (`amount`) |
| `death` | it dies |
| `spotted` | the player's squad first sees it |
| `enter` | it steps onto a tile (`x`, `y`) |

```ron
classes: [
    (
        id: "rusty_bot",
        // ...
        hooks: [
            (on: "turn_start", code: "decay(unit);"),
            (on: "death", once: true, code: "explode(x, y, 1.5, 2);"),
        ],
    ),
],
// Functions every script can call (class scripts and missions).
script_library: r#"
    fn decay(u) {
        add_stat(u, "defense", -5);
        if chance(20) { say(u, "MALFUNCTION"); give_ap(u, -2); }
    }
"#,
```

Handy for living enemies: `unit_stat` / `set_stat` / `add_stat` change
aim, defence, mobility, sight and max HP; `chance` / `rand` roll from the
battle's seed (replays stay identical); `units(team)` and `distance(a, b)`
reach the units around; `say(unit, text)` shows a callout over it; mission
flags (`set_flag("wear_" + unit, ...)`) keep per-unit state. Unknown events
are reported as module problems; script errors show in the combat log, and
`Battle::dry_run_scripts()` checks every hook in tests.

Examples: `assets/modules/corrupted_robots.ron` (machines that shed armour
and malfunction more as they wear down) and `assets/modules/mercenaries.ron`
(an officer who steadies nearby troops and whose death breaks their nerve).

## In-game editor

Press **F1** (or tap **Edit**). Use `<` / `>` to pick an ability, then `-` / `+`
to change its numbers: AP cost, ammo, cooldown, charges, range, base accuracy,
accuracy bonus, half/full cover penalties, crit, damage, area radius, can-miss,
range falloff, defence bonus, heal amount. Changes apply **immediately** to
the running battle. Hover an enemy to see the new odds.

**Save module** writes all abilities and the combat rules to
`modules/zz_editor_tuning.ron`. It loads automatically next time; delete it to
reset. On web/Android there is no file system, so the RON is printed to the log
for you to copy into a bundled module.

To make another field editable, add a variant to `EditField` in
`crates/tactics_game/src/editor.rs` and handle it in `fields_for`, `label`,
`value` and `adjust`.

---

## Adding a new *kind* of effect (needs Rust)

Data can combine existing effect kinds freely. A new kind (teleport, smoke,
mind control...) needs code in three places:

1. **`content/ability.rs`**: add a variant to `AbilityEffect`, e.g.
   `Teleport { max_distance: f32 }`.
2. **`battle.rs`**: handle it in `Battle::run_effect` (and add any extra
   requirements in `validate`). Emit an existing or new `GameEvent`.
3. **`tactics_game/src/playback.rs`**: animate the new event in `start_event`.
   Optionally teach the AI (`ai.rs`) when to use it.

Because effects are an enum, the compiler points at every place you need to
update.

---

## Future module types (outline)

Each of the following would follow the same pattern: a `…Def` struct with
`#[serde(default)]` in `content/`, a `BTreeMap` in `Ruleset` with
`add_…`/`…()` accessors, a section in `DataModule`, and a hook in the engine.
The `Ruleset` struct has a comment marking where they go.

### 1. Status effects
Burning, poisoned, stunned, suppressed, inspired...
```rust
pub struct StatusEffectDef {
    pub id: String, pub name: String,
    pub duration: u32,                 // turns
    pub damage_per_turn: i32,
    pub aim_modifier: i32, pub defense_modifier: i32, pub mobility_modifier: i32,
    pub prevents_actions: bool,        // stun
    pub stacks: bool,
}
// new effect: AbilityEffect::ApplyStatus { id: String, chance: i32 }
```
Hooks: replace `UnitStatus` with `Vec<ActiveStatus>` in `unit.rs`; tick them in
`Battle::end_turn`; read modifiers in `combat::preview_attack` and
`movement_range`. Overwatch and Defend become ordinary statuses.

### 2. Terrain types
Doors, destructible cover, water, high ground, hazards.
```rust
pub struct TerrainDef {
    pub id: String, pub glyph: char,   // character in ASCII maps
    pub walkable: bool, pub blocks_sight: bool,
    pub cover: CoverLevel, pub move_cost: u32,
    pub hit_points: Option<i32>,       // destructible
    pub becomes: Option<String>,       // terrain left after destruction
    pub model: String,
}
```
Hooks: `grid::Terrain` becomes an id into the ruleset; `Map::from_ascii`
looks glyphs up; `pathfinding` uses `move_cost`; area attacks damage
destructible tiles (new `GameEvent::TerrainChanged`); `board.rs` rebuilds
the affected tile.

### 3. Items and equipment
✅ Done: see "Loadouts" above. Next steps: an inventory screen between
missions, items found on the map (props that grant items), weapon mods.

### 4. Mission objectives and win conditions
```rust
pub enum ObjectiveDef {
    Eliminate,                                   // current behaviour
    Extract { zone: Vec<GridPos>, min_units: u32 },
    Survive { turns: u32 },
    Hack { tile: GridPos, turns: u32 },
    Protect { unit_name: String },
}
pub struct MissionDef { pub id: String, pub map: String, pub squads: Vec<String>,
                        pub objectives: Vec<ObjectiveDef>, pub turn_limit: Option<u32> }
```
Hooks: `Battle::check_victory` evaluates the mission's objectives;
`scenario.rs` builds battles from a `MissionDef`; a HUD panel lists objectives.

### 5. Enemy pods, spawn tables and reinforcements
```rust
pub struct SpawnTableDef { pub id: String, pub entries: Vec<(String /*class*/, u32 /*weight*/)> }
pub struct ReinforcementDef { pub turn: u32, pub table: String, pub count: u32, pub zone: String }
```
Hooks: `scenario.rs` rolls squads from tables with the battle RNG;
`Battle::end_turn` triggers reinforcements (new `GameEvent::UnitSpawned`).

### 6. Progression: ranks, perks and skill trees
```rust
pub struct ClassTreeDef { pub class: String, pub ranks: Vec<RankDef> }
pub struct RankDef { pub name: String, pub xp: u32, pub stat_bonus: UnitStats,
                     pub choices: Vec<AbilityId> }   // pick one perk per rank
```
Hooks: lives in a campaign layer above battles; a soldier's chosen perks
become their `abilities` list when spawned.

### 7. Campaign / strategy layer
Base facilities, research, economy, world map. This sits *outside* the battle:
a new crate (`tactics_campaign`) with its own module types (`ResearchDef`,
`FacilityDef`, `EventDef`). It produces a `Ruleset` + `MissionDef` for each
battle and reads the battle result back. In Bevy, add a `GameState` with
`Campaign` / `Battle` states around the existing plugins.

### 8. Sound and particle effects
Models, animations and camera scripts are implemented (see above). Sound and
particles would extend `AbilityPresentation` the same way:
```rust
pub struct AbilityPresentation { /* ... */
    pub sfx: Option<String>,          // "sounds/laser.ogg", played on AbilityUsed
    pub impact_sfx: Option<String>,
    pub vfx: Option<String>,          // particle preset id, spawned at the target
}
```
Hooks: enable Bevy's `audio` feature, play the sounds in `playback.rs`'s
`start_event`, and add a particle crate or a small gizmo-based emitter.

### 9. AI behaviours
Today an AI profile is a set of weights (`AiConfig`) plus fear-driven moods
(see `ai.rs`: Confident / Steady / Shaken / Panicked). Next steps:
per-class profiles (`UnitClass.ai_profile`), then behaviour *types*
(`Aggressive`, `Support`, `Sniper`, `Berserker`) implemented as strategies in
`ai.rs` and selected by data.

### 10. Localisation
`StringTableDef { language: String, entries: BTreeMap<String, String> }`, with
`name`/`description` fields holding keys (`"ability.snap_shot.name"`) and the
HUD resolving them.

### 11. Scripted effects
For effects too specific for data, embed a scripting language (e.g. Rhai or
Lua) behind an `AbilityEffect::Script { source: String }` variant. The script
gets a restricted API (deal damage, apply status, move unit) that emits normal
`GameEvent`s, so determinism and replays keep working.

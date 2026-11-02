# TBSium — a turn-based squad tactics framework in Rust

A well-commented base for an XCOM-style game: 3D rendering via
[Bevy 0.20](https://bevyengine.org), with all game rules in a separate,
engine-agnostic crate. It targets **Windows, Linux, macOS, Android and the Web**
from one codebase.

All content (moves with their accuracy and cover penalties, weapons, classes,
squads, maps, AI tuning, **3D models with animation clips, cinematic camera
scripts** and per-ability visuals) comes from **modules** written in Rust or as
RON data files, and can be tuned live with the **in-game editor (F1)**.
See **[MODULES.md](MODULES.md)** for the how-to, the full field reference and
the roadmap of future module types.

Units are animated on a **standard humanoid skeleton** with a full set of
basic movements (idle, walk, hunker, shoot, throw, reload, heal, cast, hit,
death), with upper-body and additive layering. Custom skeletons `extends` it
and custom animations override or add slots, all in data. **Drop a rigged
`.glb` into `assets/characters/`** and it is scaled, turned and its clips
mapped by name automatically; test it in the **Character Lab (F7)**. See
**[ANIMATION.md](ANIMATION.md)**.

Units carry **loadouts** (weapon, armor, two utility items: grenades,
medkits, scopes...) defined as module data and edited in game with the
**Loadout** panel. **Morale**: fear rises with wounds, near misses, fallen
comrades, flanks and bad odds, and falls with kills and hunkering; shaken
and panicked units (yours too) aim worse and panicked ones lose AP. Robots
and drones are fearless. The **AI** reads threat (the damage it would take at
each spot), plans "move then shoot", hunts flanks, focuses fire, avoids
grenade clusters and walking into overwatch, and changes behaviour with fear
(confident, steady, shaken, panicked).

Levels are **XML mission files** in one folder (`assets/missions/`): grid
layout with cut-outs, a height map (units climb 1 level unless a ladder or
their class allows more; 3D line of sight, high-ground bonus), units, props
and zones, with **scripts** attached to any of them. Modular mission
mechanics: **dormant pods, patrols, concealment and alarms, doors, fog of
war, AI goals and objectives**. Bundled large levels: *Proving Grounds*
(sandbox), *Hold the Beacon* (defend and extract), *Clear the Complex*
(explore and clear) and *Silent Theft* (stealth heist). See
**[MISSIONS.md](MISSIONS.md)**.

The HUD is an **HTML/CSS layout** (`assets/ui/hud.html`) rendered natively on
every platform, with live data bindings and clickable commands. Restyle or
rebuild it without touching Rust, and hot-reload it with **F6**. See
**[UI.md](UI.md)**.

```
TBSium/
├── Cargo.toml                 workspace + build profiles
├── .cargo/config.toml         per-target flags (wasm)
└── crates/
    ├── tactics_core/          pure Rust rules (serde/ron only for data modules)
    │   └── src/
    │       ├── content/       Ruleset, GameModule trait, AbilityDef, RON loader
    │       ├── modules/       base game, standard humanoid skeleton, example "grenadier" module
    │       ├── grid.rs        tiles, terrain, coordinates, ASCII maps
    │       ├── pathfinding.rs Dijkstra movement range + A*
    │       ├── visibility.rs  symmetric line of sight
    │       ├── cover.rs       directional cover & flanking
    │       ├── unit.rs        units, weapons, classes
    │       ├── combat.rs      hit / crit / damage formulas with breakdown
    │       ├── action.rs      Move / UseAbility / Wait / EndTurn
    │       ├── event.rs       what happened (consumed by the renderer)
    │       ├── battle/        authoritative state, turns, overwatch (mod.rs)
    │       │                  + props, script hooks, flags (world.rs)
    │       │                  + pods, stealth, doors, fog, goals (stealth.rs)
    │       │                  + fear triggers (morale.rs)
    │       ├── mission.rs     XML mission format
    │       ├── script.rs      Rhai scripting sandbox
    │       ├── ai.rs          utility AI: threat, flanks, plans, fear moods
    │       ├── content/       ruleset, modules, gear (items/loadouts), morale
    │       ├── scenario.rs    demo map + procedural skirmish generator
    │       └── rng.rs         deterministic SplitMix64
    └── tactics_game/          Bevy 3D frontend
        ├── src/
        │   ├── lib.rs         app setup, plugin wiring, Android entry
        │   ├── main.rs        desktop / web entry
        │   ├── content.rs     module load order (Rust, bundled RON, modules/ folder)
        │   ├── missions.rs    loads assets/missions/, starts battles
        │   ├── editor.rs      F1 in-game ability editor
        │   ├── state.rs       BattleState resource (only mutation point)
        │   ├── board.rs       level meshes
        │   ├── units.rs       unit entity trees + id→entity map
        │   ├── models.rs      glTF models + primitive fallbacks, animation graphs
        │   ├── animation.rs   per-unit animator: glTF clips, skeleton rigs or procedural poses
        │   ├── rig.rs         skeleton rigs: bone entities, layered keyframe poses
        │   ├── camera_director.rs  runs camera scripts (cinematics)
        │   ├── camera.rs      smoothed orbit camera (mouse, keys, touch)
        │   ├── input.rs       hover/pick tiles, clicks, shortcuts
        │   ├── enemy_turn.rs  runs the AI with pacing
        │   ├── playback.rs    animates the GameEvent queue
        │   ├── overlay.rs     gizmo overlays (range, path, cover, HP)
        │   ├── html_ui/       HTML+CSS engine: parser, cascade, styles, bindings, commands
        │   ├── hud.rs         log store, floating combat text, code-built UI helpers
        │   ├── coords.rs      grid ↔ world conversion
        │   └── theme.rs       colours & sizes
        ├── assets/modules/      bundled data modules (marksman, drones, creatures)
        ├── assets/models/       glTF models (drone.gltf, generated by tools/)
        ├── assets/characters/   drop-in rigged characters (robot.gltf + settings)
        ├── assets/ui/hud.html   the battle HUD as HTML/CSS
        ├── assets/missions/     mission XML files + index.xml
        ├── index.html, Trunk.toml   web build
        └── android/res              Android launcher icon
```

## Core idea: simulate, then present

```
 input / AI ──Action──► Battle::apply ──Vec<GameEvent>──► Playback (animations)
                            │
 UI / overlays ◄──queries───┘  (movement_range, shot_preview, visible_enemies…)
```

* `tactics_core::Battle` is the single source of truth. **Every** change goes
  through `Battle::apply(Action)`, which validates, rolls dice with a seeded RNG,
  and returns a list of `GameEvent`s.
* The frontend plays these events back one at a time (walk → overwatch shot →
  keep walking…), pausing input and AI while anything is animating.
* Because the core is deterministic and dependency-free you get, almost for
  free: unit tests for every rule, replays (store seed + actions), lock-step
  multiplayer, a headless server, or a different frontend (2D, terminal…).

## Gameplay included

* Abilities defined as data: AP cost, ammo, cooldown, charges, targeting (self /
  enemy / ally / tile), range, and attack stats (base accuracy, accuracy bonus,
  half/full cover penalties, crit, damage, area radius).
* Included moves: Shoot, Overwatch, Hunker Down, Reload (base); Frag Grenade,
  Medkit + Grenadier class (Rust example module); Aimed Shot, Psi Bolt (RON example).
* 2 action points per unit; move (1 AP) or dash (2 AP), blue/yellow range overlay.
* Directional half/full cover, flanking (+crit), cover shields on hover.
* Hit chance = aim − defence − cover − hunker ± range falloff, shown with a full breakdown.
* Overwatch reaction fire that interrupts movement mid-path.
* Hunker down, reload, ammo, critical hits, death, victory/defeat.
* Utility AI (cover-seeking, flanking-aware, shoots good odds, advances when blind).
* Hand-made demo map + procedural skirmishes ("New Battle").

## Controls

| | Desktop | Touch |
|---|---|---|
| Select / move / shoot | Left click | Tap |
| Move by dragging | Drag from the selected soldier to a tile, release | — |
| Pan | Left- or middle-drag the ground, WASD / arrows | One-finger drag |
| Rotate / tilt | Right-drag (sideways / up-down), Q / E | Two-finger twist |
| Zoom | Wheel (towards the cursor), Z / X | Pinch |
| Abilities of the selected unit | 1-9 (Esc cancels targeting) | ability buttons |
| Next unit / End turn | Tab / Enter | HUD buttons |
| Module editor | F1 | Edit button |
| Skip a cinematic / toggle cinematics | Space / C | Cinema button |
| Reload the HTML HUD | F6 | — |
| Character Lab | F7 | Missions button |
| Restart mission | F5 | button on game-over screen |
| Pick a mission | — | Missions button |

## Building

Requires Rust **1.97.1+** (`rustup update`).

### Desktop (Linux / Windows / macOS)

```sh
cargo run -p tactics_game            # dev build (deps optimised, game code fast to compile)
cargo run -p tactics_game --release  # release
cargo test -p tactics_core           # rules test-suite
cargo run -p tactics_core --example autobattle -- 20   # headless AI-vs-AI balance check
cargo test -p tactics_core --no-default-features      # rules with zero dependencies
```

Linux needs a few system libraries:
`sudo apt install pkg-config libudev-dev libwayland-dev libxkbcommon-dev libasound2-dev`.
Windows and macOS need nothing extra.

Cross-compiling a Windows `.exe` from Linux:
`rustup target add x86_64-pc-windows-gnu && sudo apt install mingw-w64 && cargo build -p tactics_game --release --target x86_64-pc-windows-gnu`.

Assets (`crates/tactics_game/assets/`) are found automatically by `cargo run`.
When running a built executable directly, ship the `assets/` folder next to it
(or set `BEVY_ASSET_ROOT` to the folder that contains `assets/`).

### Web (WebAssembly, WebGL2)

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
cd crates/tactics_game
trunk serve --release     # http://127.0.0.1:8080
trunk build --release     # static files in crates/tactics_game/dist/
```

To use WebGPU instead of WebGL2, add the `webgpu` Bevy feature in the wasm
section of `crates/tactics_game/Cargo.toml`.

### Android

The library target (`tactics`, i.e. `libtactics.so`) is built as a `cdylib` with a
`#[bevy_main]` entry and uses
NativeActivity, so it can be packaged without a Java/Gradle project:

```sh
# Requires the Android SDK + NDK, with ANDROID_HOME and ANDROID_NDK_ROOT set.
rustup target add aarch64-linux-android
cargo install --locked cargo-apk
cargo apk run -p tactics_game --release      # builds, installs & launches on a connected device
```

Packaging settings (app id, SDK levels, orientation, icon) live under
`[package.metadata.android]` in `crates/tactics_game/Cargo.toml`. For Play
Store builds (AAB, GameActivity), use a Gradle project with `cargo-ndk`, as in
Bevy's `examples/mobile` template, and switch the feature to
`android-game-activity`.

## Extending the framework

Content (new moves, weapons, classes, squads, maps, AI tuning) → **[MODULES.md](MODULES.md)**.

| Want to… | Where |
|---|---|
| New move from existing effect kinds | a RON or Rust module — no engine changes |
| New *kind* of effect (teleport, smoke…) | `AbilityEffect` + `Battle::run_effect` + `playback.rs` (see MODULES.md) |
| Fog of war | `Battle::team_can_see`; hide enemy visuals in `units.rs` and filter the AI in `ai.rs` |
| Custom skeleton / animations | `skeletons` + `animations` in a module (`extends: "humanoid"`); see ANIMATION.md |
| Real unit art and animations | a `ModelDef` in any module (glTF path + clips per slot); see MODULES.md |
| New level / mission | an XML file in `assets/missions/` + a line in `index.xml`; see MISSIONS.md |
| Scripted events (doors, traps, reinforcements, objectives) | `<script on="...">` in the mission file; see MISSIONS.md |
| Restyle / rearrange the HUD | edit `ui/hud.html` (HTML + CSS), F6 to reload; see UI.md |
| Cinematic camera for an action | a `CameraScriptDef` + `AbilityPresentation.camera`; see MODULES.md |
| Real level art | replace the primitive meshes in `board.rs` with glTF scenes |
| Save / load, networking | serialize `Battle` (or seed + action log) — all state is plain data |
| Strategy layer (base, research) | a new Bevy `State` around the battle; see "Future module types" in MODULES.md |

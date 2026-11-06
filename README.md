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

Soldiers are **individuals**: each has a look (build, height, skin, hair,
headgear, armour colours, backpack) and visibly carries their weapon and
gear on attachment points, holding rifles two-handed (IK). The **Barracks**
saves your soldiers between sessions (`tbsium_roster.ron`): customise
their looks, recruit, retrain, pick who deploys, and watch their service
records (missions, kills, wounds; optional permadeath).

Units carry **loadouts** (weapon, armor, two utility items: grenades,
medkits, scopes...) defined as module data and edited in game with the
**Loadout** panel: pick a soldier, then step through their class, weapon,
armour, items or a quick-pick preset with the arrows (each choice is
described); soldiers retrain as other soldier classes, robots refit as
other frames. **Morale**: fear rises with wounds, near misses, fallen
comrades, flanks and bad odds, and falls with kills and hunkering; shaken
and panicked units (yours too) aim worse and panicked ones lose AP. Robots
and drones are fearless. The **AI** reads threat (the damage it would take at
each spot), plans "move then shoot", hunts flanks, focuses fire, avoids
grenade clusters and walking into overwatch, and changes behaviour with fear
(confident, steady, shaken, panicked).

Levels are **XML mission files** in one folder (`assets/missions/`): grid
layout with cut-outs, a height map (units climb 1 level unless a ladder or
their class allows more; 3D line of sight with corner peeking, high-ground bonus), units, props
and zones, with **scripts** attached to any of them. Modular mission
mechanics: **dormant pods, patrols, concealment and alarms, doors, fog of
war, AI goals and objectives**. **Generated missions**: urban streets with enterable buildings and
rooftops, industrial yards with container stacks, or wilds with hills, a
river and woods, each with a random objective (eliminate, extract, hold a
beacon, recover data). **Outbreak**: a 500x500 zombie city with
multi-storey towers and split-level car parks (stairs and ramps between
floors), supply caches everywhere and hundreds of slow zombies that come
running at the sound of gunfire. Maps are painted with surfaces (asphalt, grass,
brick, containers, trees, water...), and hand-made missions get the same
look automatically. Each mission can set its **sky** in the file: a colour, a
gradient or a panorama image (`<sky>`).

New to the controls? Play *Basic Training* from the Missions list. Bundled large levels: *Proving Grounds*
(sandbox), *Hold the Beacon* (defend and extract), *Clear the Complex*
(explore and clear) and *Silent Theft* (stealth heist); with the
faction modules: *Merc Ambush* (storm a merc compound, break their nerve by
killing the officer), *Scrapyard* (corrupted robots that fall apart: kill
them or outlast them) and *Salvage Run* (steal a robot core from a merc
crew and their cargo); with the Dinosaurs module, *Lost Valley* (recover a
lost expedition's drive from a jungle of raptors, a 2x2 Hornback and a 3x3
Tyrant), *Raptor Run* (run a canyon to the evac before the chopper leaves
while packs pour out of the side ravines), *Tyrant Hunt* (kill a Tyrant, and
its mate, round a research station whose doorways are too narrow for them)
and *Nest Raid* (steal three raptor eggs at night). See
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
    │       ├── visibility.rs  symmetric line of sight, corner peeking
    │       ├── cover.rs       directional cover & flanking
    │       ├── unit.rs        units, weapons, classes
    │       ├── combat.rs      hit / crit / damage formulas with breakdown
    │       ├── action.rs      Move / UseAbility / Wait / EndTurn
    │       ├── event.rs       what happened (consumed by the renderer)
    │       ├── battle/        authoritative state, turns, overwatch (mod.rs)
    │       │                  + props, script hooks, flags (world.rs)
    │       │                  + pods, stealth, doors, fog, goals (stealth.rs)
    │       │                  + fear triggers (morale.rs)
    │       │                  + traced bullets, suppression (ballistics.rs)
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
        │   ├── fog.rs         fog of war shading (in sight / explored / unseen)
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
  Medkit + Grenadier class (Rust example module); Aimed Shot, Psi Bolt (RON example);
  Zombies (Rust class with a melee Bite and the horde AI, shambling look in
  `assets/modules/zombies.ron`); Mercenaries and Corrupted Robots (pure data
  modules with class scripts: an officer who holds the line, machines that
  break down, malfunction and lose armour over time).
* Arsenal (data module): laser rifles, a laser lance and scatter laser (beams),
  minigun, autocannon, a rocket launcher (straight-flying rockets that go off
  at the first thing in the way), a flamethrower (as a main weapon or a
  backpack: a cone of fire) and
  incendiary grenades, a particle grenade (short throw, goes off two turns
  later: 5 damage in a 5x5 square plus shrapnel), melee weapons (combat
  knife, machete, sledgehammer), a **taser** (shorts out light and medium
  robots on the spot, 9 damage to heavy ones, stuns people), an **adrenaline
  shot** (+2 AP now for half your remaining health; free at 1 HP), **scanner
  armor** (sees 50% further), a **gas grenade** (a cloud where nobody living
  can aim and every move staggers somewhere random; robots are immune), and the Heavy
  Gunner class. Big robots (Bastion frames, wardens, loaders) can **slam** the
  ground, hitting everyone next to them.
* Fire: flamethrowers, incendiaries and exploding fuel barrels leave tiles
  burning for a few rounds; standing or walking in fire burns (machines heat
  up), props in it cook off, and the AI keeps out of it.
* Robot Frames (data module): player robots that run on **heat instead of
  health**: hits, shots and moving heat them up, they cool each turn, and
  they're destroyed at the heat limit. **Shutdown** vents all heat after two
  offline turns (but any hit while shut down destroys them); coolant tanks
  shed heat at once. Light Strider and heavy Bastion frames each have their
  own radical arsenal (pulse lasers, needle rail, arc thrower; gatling
  laser, plasma cannon, missile pod). Build them in the Barracks. Striders
  can **overclock** (3 a mission: an extra shot, double heat); frames can
  carry **boost thrusters** (a long straight-line rush that walls stop),
  soldiers a **jump pack** (a shorter leap that clears walls). **Idle** ends a
  frame's turn without making any heat.
* Dinosaurs (data module) and **multi-tile units**: a class `size` makes a
  unit take up size x size tiles (it only fits where its whole footprint is
  walkable and level, can't squeeze through narrow gaps, and is shot, blasted,
  seen and bitten from any of its tiles). Raptors (1 tile, fast packs that
  screech to wake each other), the Hornback (2x2, armoured, charges when hurt)
  and the Tyrant (3x3, 48 HP, its roar frightens soldiers).
* 2 action points per unit; move (1 AP) or dash (2 AP), blue/yellow range overlay.
* Directional half/full cover, flanking (+crit), cover shields on hover.
* Hit chance = aim − defence − cover − hunker ± range falloff, shown with a full breakdown.
* Shots are bullets: every round of a burst (and every shotgun pellet) is a ray
  fired with an aim error from the hit chance; rays stop on walls and props
  and hit **whoever is in the line of fire**, friend or foe. Cover soaks a
  share of the rounds. Machine guns can **suppress** (pin a target: −aim,
  +fear, reaction burst if it moves).
* Explosions damage props and walls: walls crack, then collapse into rubble
  (half cover); grenades throw traced shrapnel past the blast.
* Magazines: bursts use 4–6 rounds of a larger clip, reloads use up spare
  magazines (an ammo pouch adds one).
* Overwatch reaction fire that interrupts movement mid-path.
* Hunker down, reload, ammo, critical hits, death, victory/defeat.
* Utility AI (cover-seeking, flanking-aware, shoots good odds, advances when blind).
* Feel: spring-smoothed camera that follows lazily (dead zone) and frames
  shots then returns to your zoom; walls between the camera and your soldiers
  sink out of the way (cutaway); a cursor tooltip with odds / move cost;
  aim-then-confirm attacks; turn and alarm banners; enemy turns skip hidden and
  idle units instantly so big maps stay brisk; move look-ahead (who you could
  shoot or flank from a tile, overwatch on the way); damage preview on HP
  pips; glowing, labelled evac pads and objectives; a minimap (with a vague enemy heatmap once only a few hidden hostiles are left).
* Hand-made demo map + procedural skirmishes ("New Battle").

## Controls

| | Desktop | Touch |
|---|---|---|
| Select | Left click | Tap |
| Move | Left click a tile | **Move** button, tap a tile (path and cost preview), tap it again or **Confirm** |
| Inspect a tile | Hover | Tap (without Move): its info stays up |
| Attack | Click an enemy to **aim** (camera frames the shot, odds by the cursor), click again / Space / Enter to **fire** | Tap to aim, tap again or **Fire** |
| Cancel aim / targeting | Right-click or Esc | Cancel button |
| Move by dragging | Drag from the selected soldier to a tile, release | — |
| Pan | Left- or middle-drag the ground, WASD / arrows | One-finger drag (the ground stays under the finger; flick to glide) |
| Rotate / tilt | Right-drag (sideways / up-down), Q / E | the < and > buttons (45° steps) |
| Zoom | Wheel (towards the cursor), Z / X | Pinch (towards the fingers; two fingers only zoom) |
| Scroll menus and lists | Wheel | Drag the panel (flick to keep scrolling) |
| Menu / button size | Settings > UI size | Settings > UI size (auto: a bit smaller on phones) |
| Keep clear of the phone's back / home buttons | — | Settings > Screen edge margin (auto: 48 px on Android) |
| Abilities of the selected unit | 1-9 (press again to fire when aimed) | ability buttons |
| Next / previous unit | Tab / Shift+Tab | Next button |
| Centre on selected unit / reset view | F / Home | — |
| End turn | Enter (when not aiming) | End Turn button |
| Fast-forward animations | hold Shift (x3) | — |
| Minimap (click to jump) | M | — |
| Controls cheat sheet | H | Help button |
| Barracks (saved soldiers, looks, squad) | — | Barracks button |
| Module editor | F1 | Edit button |
| Skip a cinematic / toggle cinematics | Space / C | Cinema button |
| Reload the HTML HUD | F6 | — |
| Character Lab | F7 | Missions button |
| Restart mission | F5 | button on game-over screen |
| Pick a mission | — | Missions button |

Touch controls switch on automatically on Android and at the first touch
elsewhere (back to mouse rules when the mouse is used). `TBS_TOUCH=1`
forces them on, to try the touch interface with a mouse.

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

No Rust toolchain at hand? The **Web (WASM)** GitHub Actions workflow
(`.github/workflows/web.yml`) runs `trunk build --release --public-url ./`
and attaches the finished site as the `tactics-web` artifact. Serve that
folder with any static server (`python3 -m http.server -d tactics-web 8080`);
opening `index.html` straight from disk doesn't work, because browsers block
wasm and asset loading from `file://`. In the browser, the roster and
settings are saved in localStorage. User module and HUD files (`modules/`,
`ui/`) are desktop-only.

On phones and tablets the web version starts in touch mode, scales the HUD
to the screen, keeps every gesture for the game (no browser pinch-zoom or
pull-to-refresh), and asks for landscape. The **Full screen**
button in the toolbar hides the browser bars and locks landscape where the
browser allows it. It is also installable: "Add to Home Screen" (manifest
and icons in `crates/tactics_game/web/`) opens it fullscreen in landscape,
which is the way to go full screen on iPhones.

The same workflow publishes the game on **GitHub Pages** at
`https://<owner>.github.io/<repo>/`. One-time setup: Settings -> Pages ->
Source: **GitHub Actions** (and, if you build from a branch other than the
default one, allow it under Settings -> Environments -> github-pages ->
Deployment branches).

### Android

The library target (`tactics`, i.e. `libtactics.so`) is built as a `cdylib` with a
`#[bevy_main]` entry and uses
NativeActivity, so it can be packaged without a Java/Gradle project:

```sh
# Requires the Android SDK + NDK, with ANDROID_HOME and ANDROID_NDK_ROOT set.
rustup target add aarch64-linux-android
cargo install --locked cargo-apk
cargo apk run -p tactics_game --release --lib   # builds, installs & launches on a connected device
```

No NDK at hand? The **Android APK** GitHub Actions workflow
(`.github/workflows/android.yml`) builds the APK on GitHub's runners (they
have the SDK and NDK) and attaches it to the run as the `tactics-apk`
artifact; start it from the Actions tab. The build needs no C compiler for
the target (Bevy's `blake3` is built in its pure-Rust mode), only the NDK's
linker. On the device, the roster and settings are saved in the app's
private data folder.

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

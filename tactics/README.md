# Tactics — a turn-based squad tactics framework in Rust

A well-commented base for an XCOM-style game: 3D rendering via
[Bevy 0.20](https://bevyengine.org), with all game rules in a separate,
engine-agnostic crate. It targets **Windows, Linux, macOS, Android and the Web**
from one codebase.

```
tactics/
├── Cargo.toml                 workspace + build profiles
├── .cargo/config.toml         per-target flags (wasm)
└── crates/
    ├── tactics_core/          pure Rust rules — no dependencies
    │   └── src/
    │       ├── grid.rs        tiles, terrain, coordinates, ASCII maps
    │       ├── pathfinding.rs Dijkstra movement range + A*
    │       ├── visibility.rs  symmetric line of sight
    │       ├── cover.rs       directional cover & flanking
    │       ├── unit.rs        units, weapons, templates
    │       ├── combat.rs      hit / crit / damage formulas with breakdown
    │       ├── action.rs      the commands players & AI can issue
    │       ├── event.rs       what happened (consumed by the renderer)
    │       ├── battle.rs      authoritative state, turns, overwatch
    │       ├── ai.rs          utility-based enemy AI
    │       ├── scenario.rs    demo map + procedural skirmish generator
    │       └── rng.rs         deterministic SplitMix64
    └── tactics_game/          Bevy 3D frontend
        ├── src/
        │   ├── lib.rs         app setup, plugin wiring, Android entry
        │   ├── main.rs        desktop / web entry
        │   ├── state.rs       BattleState resource (only mutation point)
        │   ├── board.rs       level meshes
        │   ├── units.rs       unit meshes + id→entity map
        │   ├── camera.rs      smoothed orbit camera (mouse, keys, touch)
        │   ├── input.rs       hover/pick tiles, clicks, shortcuts
        │   ├── enemy_turn.rs  runs the AI with pacing
        │   ├── playback.rs    animates the GameEvent queue
        │   ├── overlay.rs     gizmo overlays (range, path, cover, HP)
        │   ├── hud.rs         bevy_ui HUD + touch buttons
        │   ├── coords.rs      grid ↔ world conversion
        │   └── theme.rs       colours & sizes
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
| Pan | WASD / arrows / middle drag | One-finger drag |
| Rotate | Q / E, right drag | Two-finger twist |
| Zoom | Wheel, Z / X | Pinch |
| Next unit / Overwatch / Hunker / Reload / End turn | Tab / O / H / R / Enter | HUD buttons |
| New battle | F5 | button on game-over screen |

## Building

Requires Rust **1.97.1+** (`rustup update`).

### Desktop (Linux / Windows / macOS)

```sh
cargo run -p tactics_game            # dev build (deps optimised, game code fast to compile)
cargo run -p tactics_game --release  # release
cargo test -p tactics_core           # rules test-suite
cargo run -p tactics_core --example autobattle -- 20   # headless AI-vs-AI balance check
```

Linux needs a few system libraries:
`sudo apt install pkg-config libudev-dev libwayland-dev libxkbcommon-dev libasound2-dev`.
Windows and macOS need nothing extra.

Cross-compiling a Windows `.exe` from Linux:
`rustup target add x86_64-pc-windows-gnu && sudo apt install mingw-w64 && cargo build -p tactics_game --release --target x86_64-pc-windows-gnu`.

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

| Want to… | Where |
|---|---|
| Add a new ability (grenade, medkit) | add a variant to `Action`, validate + apply it in `battle.rs`, emit an event; animate it in `playback.rs`; add a button in `hud.rs` |
| New unit class / weapon | a constructor in `unit.rs` (`UnitTemplate`, `Weapon`) |
| New terrain (doors, destructibles) | `Terrain` in `grid.rs` — walkability, sight and cover are derived from it |
| Fog of war | `Battle::team_can_see`; hide enemy visuals in `units.rs` and filter the AI in `ai.rs` |
| Real art | replace the primitive meshes in `board.rs` / `units.rs` with glTF scenes from `assets/` |
| Save / load, networking | serialize `Battle` (or seed + action log) — all state is plain data |
| Strategy layer (base, research) | a new Bevy `State` around the battle; the core crate already doesn't care |

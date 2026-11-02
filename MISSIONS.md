# Missions: maps, heights and scripts

A mission is **one XML file** in `crates/tactics_game/assets/missions/`. It
defines the grid (including where it is cut out), a height map, the units,
props and zones, and scripts attached to any of them.

```
assets/missions/
├── index.xml               lists the missions (and which one starts first)
├── depot.mission.xml
└── rooftops.mission.xml
```

Files load through Bevy's asset server, so the same folder works on desktop,
web and Android. Errors (bad XML, unknown element, position outside the map,
script syntax errors) are shown in the combat log with **line and column**.
`cargo test -p tactics_game` parses every listed mission and plays each one
AI-vs-AI to catch broken content.

## Adding a mission

1. Copy `depot.mission.xml` to `my_level.mission.xml` and change its `id`.
2. Add it to `index.xml`: `<mission file="my_level.mission.xml"/>`
   (`<missions start="my_level">` makes it the one that starts first).
3. Run the game. Pick it from the **Missions** button in the HUD.

## Coordinates

`x` = column from the **left**, `y` = row from the **top**, exactly as you
read `<layout>`. (Scripts use the same convention.)

## The file

```xml
<mission id="ambush" name="Ambush" win="eliminate" seed="42">
  <description>Shown in the mission list.</description>

  <layout>
    ..........
    ..##...h..
    ..##...H..
  </layout>

  <heights>
    2200000000
    2200000000
    0000000000
  </heights>

  <!-- rectangle helpers, applied in order after layout/heights -->
  <cutout x="9" y="0" width="1" height="3"/>
  <raise x="4" y="0" width="2" height="1" level="1"/>
  <terrain x="5" y="2" type="half"/>
  <ladder x="2" y="2"/>

  <deploy team="player" squad="player" x="5" y="2"/>
  <unit team="enemy" class="sectoid" name="Leader" x="8" y="0" tag="leader">
    <script on="death">message("Leader down!");</script>
  </unit>
  <prop tag="barrel" kind="barrel" x="6" y="1" cover="half" hp="2">
    <script on="destroyed">explode(x, y, 1.5, 3);</script>
  </prop>
  <zone tag="exit" x="0" y="2" width="2" height="1">
    <script on="enter" team="player">win();</script>
  </zone>
  <tile x="7" y="2"><script on="enter" once="true">message("Click.");</script></tile>

  <script>fn shout(s) { message(s + "!") }</script>
  <script on="start">shout("Go");</script>
</mission>
```

### `<mission>` attributes

| attribute | meaning |
|---|---|
| `id` | unique id (used by `index.xml` `start` and the `mission:<id>` UI command) |
| `name` | display name |
| `win` | `eliminate` (default): killing every enemy wins. `script`: only `win()` wins. Losing the whole squad always loses. |
| `seed` | fixed random seed (same dice every time); omit for random |
| `width`, `height` | optional; taken from `<layout>` otherwise |

### The grid

| element | meaning |
|---|---|
| `<layout>` | one character per tile, first row = top. `.` floor, `#` **cut out** (no tile: impassable, not drawn), `h` half cover, `H` full cover. Any other character is floor. |
| `<heights>` | same shape as the layout: ground level `0`-`9` per tile. |
| `<cutout x y [width height]>` | cut a rectangle out of the grid. |
| `<raise x y [width height] level>` | set the ground level of a rectangle. |
| `<terrain x y [width height] type>` | `floor`, `half`, `full` or `void`. |
| `<ladder x y [width height]>` | ladder/ramp tiles (see below). |

### Height rules

* A unit can step **up at most 1 level** per step. Its class's `climb` stat
  can raise this (the Drone Pack's drones have `climb: 9`, so they fly).
* **Ladders** allow any climb into or out of their tile.
* Stepping **down** is always allowed. Climbing costs +1 movement point per
  level.
* **Line of sight** is 3D: sight lines run between units' eyes (1.5 levels
  above their ground). A 1-level step doesn't block sight; a 2-level cliff
  does; high ground sees over walls.
* **Cover from terrain:** a neighbouring tile 1 level higher is half cover,
  2+ levels is full cover.
* **High ground:** +15 aim when firing from higher ground
  (`CombatRules::height_advantage_bonus`).

### Contents

| element | attributes |
|---|---|
| `<deploy>` | `team`, `squad` (a squad id from the modules, e.g. `player`), `x`, `y`: places the squad around that tile |
| `<unit>` | `team` (`player`/`enemy`), `class` (from the modules), `name`, `x`, `y`, `tag` (for scripts) |
| `<prop>` | `tag`, `kind` (`crate`, `barrel`, `console`... used for visuals), `x`, `y`, `cover` (`none`/`half`/`full`; any cover blocks movement), `hp` (omit = indestructible), `model` |
| `<zone>` | `tag`, `x`, `y`, `width`, `height`: an area for `enter` scripts |
| `<tile>` | `x`, `y`: a 1×1 zone |
| `<script>` | without `on`: shared functions for all scripts. With `on`: a hook (below). |

Area attacks (grenades, `explode`) damage destructible props; a destroyed
prop frees its tile.

## Scripts

Scripts are [Rhai](https://rhai.rs), a small Rust-like language, written
directly inside `<script>`. You can use `<`, `>` and `&&` freely: the loader
wraps script bodies in CDATA for you.

### Where they attach and when they run

| attached to | `on=` | runs when | `unit` is |
|---|---|---|---|
| `<mission>` | `start` | the battle begins | -1 |
| `<mission>` | `turn_start` | every turn start (`team` = whose turn) | -1 |
| `<mission>` | `unit_killed` | any unit dies | the dead unit |
| `<zone>` / `<tile>` | `enter` | a unit steps onto the area | the mover |
| `<unit>` | `death` | this unit dies | this unit |
| `<unit>` | `damaged` | this unit takes damage (`amount`) | this unit |
| `<unit>` | `spotted` | the player's squad first sees this unit | this unit |
| `<unit>` | `turn_start` | its own team's turn starts | this unit |
| `<prop>` | `damaged` / `destroyed` | the prop is hit / destroyed | -1 (`prop` = its tag) |

Options on `<script>`: `team="player|enemy"` (only for that team's units /
turns) and `once="true"` (run at most once).

### Variables

`round`, `team` (whose turn: `"player"`/`"enemy"`), `unit` (id or -1),
`prop` (tag or `""`), `x`, `y` (the event's tile), `amount` (damage taken).

### Functions

| function | effect |
|---|---|
| `message(text)` / `print(text)` | show a message |
| `spawn_unit(team, class, name, x, y)` | add a unit (nearest free tile) |
| `damage(unit, n)`, `heal(unit, n)`, `kill(unit)` | |
| `explode(x, y, radius, damage)` | blast hitting units and props |
| `damage_prop(tag, n)`, `destroy(tag)` | |
| `set_terrain(x, y, "floor"/"half"/"full"/"void")` | change a tile (e.g. open a door, collapse a floor) |
| `set_height(x, y, level)`, `set_ladder(x, y, true/false)` | reshape the level |
| `give_ap(unit, n)` | |
| `win()`, `lose()` | end the mission |
| `set_flag(name, n)`, `flag(name)` | mission variables (integers, default 0) |
| `unit_by_tag(tag)`, `unit_at(x, y)` | find units (-1 = none) |
| `unit_alive(id)`, `unit_hp(id)`, `unit_name(id)`, `unit_team(id)`, `unit_x(id)`, `unit_y(id)` | |
| `alive_count("player"/"enemy")`, `prop_alive(tag)`, `height(x, y)`, `terrain(x, y)` | |

Example: a door that opens when a switch is used.

```xml
<layout>
  ....H....
  ....H....
</layout>
<tile x="0" y="1">
  <script on="enter" team="player" once="true">
    set_terrain(4, 1, "floor");
    message("The door slides open.");
  </script>
</tile>
```

### Safety and determinism

* Scripts can't touch the game directly: they read a snapshot and return
  commands that the rules engine applies, emitting normal events that the
  game animates. Same seed, same actions, same scripts: same battle (replays
  and multiplayer stay possible).
* Each run is capped (100,000 operations, call depth, string size), so an
  infinite loop just stops with an error. Scripts triggering scripts are
  cut off after 16 levels.
* Any error (syntax, unknown function, bad team name...) is shown in the
  combat log as `Script error: ...`; the battle continues.

### Extending the scripting API

1. Add a function in `register_api` (`tactics_core/src/script.rs`). Queries
   read the `ScriptWorld` snapshot; actions push a `ScriptCommand`.
2. For a new action, add the `ScriptCommand` variant and handle it in
   `Battle::apply_script_command` (`tactics_core/src/battle/world.rs`),
   emitting `GameEvent`s.
3. New hook events: add a `HookEvent` variant, parse it in
   `HookEvent::parse`, and call `fire(...)` where it happens in the rules.

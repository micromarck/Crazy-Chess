# Modules: adding and editing game content

Everything you can tune — abilities ("moves"), weapons, unit classes, squads,
maps, AI behaviour and the global combat rules — lives in a **`Ruleset`** that
is built from **modules**. The rules engine (`tactics_core::battle`) only reads
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

### Effects (`AbilityEffect`)

| Effect | Fields | Notes |
|---|---|---|
| `Attack(profile)` | `AttackProfile` | Single target, or area if `area_radius > 0`. |
| `Overwatch(profile)` | `AttackProfile` | Arms a reaction shot using this profile. |
| `Defend(defense_bonus: n)` | int | Hunker: +defence until your next turn. |
| `Reload` | — | Refills the magazine. |
| `Heal(amount: n)` | int | Restores HP, never above max. |

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
| `area_radius` | `0.0` | > 0 = hits every unit within that many tiles (friendly fire!). Cover is checked from the blast centre. |

Hit chance:

```
hit  = (base_accuracy or unit aim) + accuracy_bonus
       - target defense - target defend bonus
       - half/full cover penalty (if the target has cover facing the attack)
       ± range modifier (if range_falloff)
       clamped to [combat.min_hit, combat.max_hit] unless can_miss = false
crit = weapon crit (weapon damage only) + crit_bonus + flank_crit_bonus (if flanked)
```

### Other content

| Section | Type | Key fields |
|---|---|---|
| `weapons` | `Weapon` | `id, name, damage_min, damage_max, crit_chance, crit_bonus, optimal_range, max_range, clip_size` |
| `classes` | `UnitClass` | `id, name, stats: (max_hp, mobility, aim, defense, sight), weapon, abilities: [...], model` |
| `squads` | `SquadDef` | `id` (`"player"` / `"enemy"` are deployed), `members: [(class, name)]` |
| `maps` | `MapDef` | `id` (`"demo"` is used for the first battle), `name`, `ascii` (`.` floor, `h` half, `H` full, `#` void, `A`/`E` spawns) |
| `ai_profiles` | `AiProfile` | `id` (`"default"` drives the enemy), `config: (shoot_threshold, area_min_score, heal_below, ...)` |
| `combat` | `CombatRules` | `min_hit, max_hit, falloff_per_tile, close_bonus_per_tile, max_close_bonus, action_points` |
| `grants` | `AbilityGrant` | `(class, ability)`: add a move to an existing class |

`model` is a renderer hint: `"alien"` gets the big-headed mesh, anything else
the soldier mesh.

---

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
Grenades, armour, scopes, medkits as inventory instead of fixed class moves.
```rust
pub struct ItemDef {
    pub id: String, pub name: String,
    pub slot: ItemSlot,                // Primary, Secondary, Armor, Utility
    pub stat_modifiers: UnitStats,     // added to the class stats
    pub grants_abilities: Vec<AbilityId>,
    pub charges: Option<u32>,
}
// UnitClass gains `loadout: Vec<String>`; SquadMember can override it.
```
Hooks: `Battle::spawn` merges item stats and abilities; ability charges can be
tied to item stacks.

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

### 8. Presentation: models, VFX, sound per content
```rust
pub struct PresentationDef {
    pub id: String,                    // ability, class or terrain id it decorates
    pub model: Option<String>,         // glTF path, e.g. "models/sectoid.glb#Scene0"
    pub animation: Option<String>,
    pub sfx: Option<String>, pub vfx: Option<String>,
    pub icon: Option<String>,          // HUD button icon
}
```
Kept separate from rules data so headless servers and tests never load
assets. Hooks: `units.rs`, `board.rs`, `playback.rs` and `hud.rs` look these
up by id.

### 9. AI behaviours
Today an AI profile is a set of weights (`AiConfig`). Next steps:
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

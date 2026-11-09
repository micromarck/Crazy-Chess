# Campaign system: design outline

**Status: not implemented yet.** This is the plan for the next piece of
work. It sticks to systems the game already has: the saved roster (Barracks,
permadeath, mission counts), loadouts, data modules, XML missions with
scripts, and `read_save` / `write_save` (a file on desktop, `localStorage` on
the web). Everything new is **data first**, like the rest of the game, so
modules can add campaigns.

---

## 1. What the player does

A campaign is a chain of operations with a strategy layer between missions:

```
 Command screen ──► pick an operation ──► Loadout (squad, gear, augments)
       ▲                                         │
       │                                         ▼
  Debrief (rewards, injuries, promotions) ◄── Mission (the existing battle)
```

* **Command screen**: the campaign map, which is a list of operations
  grouped by region or act. Each one shows its threat, rewards, deadline and
  which enemies it has (module tags). The player also sees their resources,
  the roster's state (fit, wounded, shaken, KIA) and the research queue.
* **Operations** are missions. Some are **story** (hand-made: Ghost in the
  Machine, Lost Valley, Dead Signal...) and some are **generated** (the
  random urban, industrial or wilds missions) to fill gaps and pay for things.
* **Time** passes in **days**. Each operation takes days; wounds heal and
  research progresses over days. Ignored operations escalate or expire,
  and **threat** rises.
* **Lose the campaign** if threat hits its maximum, or if the roster can't
  field a squad and you can't afford recruits.

## 2. Acts (using the missions that exist today)

| Act | Theme | Story operations | Unlocks |
|---|---|---|---|
| 1. Boots on the ground | training, mercs | Basic Training, Proving Grounds, Merc Ambush, Hold the Beacon | Grenadier and Assault gear, Arsenal (lasers) |
| 2. Outbreak | zombies | Clear the Complex, Hold the Hill, Outbreak (the city) | incendiaries, flamethrower |
| 3. The machine | robots | Scrapyard, Salvage Run, Ghost in the Machine (VESPER) | Robot Frames (salvaged VESPER cores), taser, Augments |
| 4. Lost world | dinosaurs | Lost Valley, Raptor Run, Nest Raid, Tyrant Hunt | Heat Frame augment, heavy weapons |
| 5. Dead signal | symbiotes | Silent Theft (intel), Dead Signal | the ending |

Acts unlock by **flags** (for example `act3_open` once VESPER is dead), not
by a fixed order. That lets a module insert its own act.

## 3. Persistent state (the save)

One file, `campaign.ron` (like `roster.ron` today), holding:

```ron
(
  campaign: "main",            // CampaignDef id
  day: 37,
  seed: 1234,                  // generated operations stay the same on reload
  resources: (funds: 450, salvage: 12, intel: 3),
  threat: 41,                  // 0-100
  flags: { "vesper_dead": 1, "act3_open": 1 },
  research: (done: ["lasers", "tasers"], active: Some(("augments", 4))),  // days left
  operations: [ (id: "op_17", mission: "gen:urban", expires: 40, rewards: (...)) ],
  roster: [ /* SoldierRecord as today, plus the fields in section 5 */ ],
  history: [ (day: 12, op: "facility", result: Win, dead: ["Pvt. Kowalski"]) ],
)
```

* The **roster** moves into the campaign save. The existing `Roster` stays
  the type; skirmish keeps using `roster.ron`.
* Saving happens after every debrief and on quit. On the web, everything in
  the save must fit in `localStorage` (keep it small, text).

## 4. Data: `CampaignDef` in modules

A new module section, `campaigns: [...]`, sits next to `classes`,
`loadouts` and the rest:

```ron
campaigns: [
  (
    id: "main",
    name: "Operation Nightfall",
    start: (funds: 300, squad: "player", flags: {}),
    threat: (per_day: 1, per_ignored_op: 5, max: 100),
    operations: [
      (
        id: "vesper",
        mission: "facility",              // a mission id, or "gen:urban" etc.
        name: "Ghost in the Machine",
        requires: ["act3_open"],          // flags
        days: 3,
        deadline: Some(20),               // expires N days after appearing
        rewards: (funds: 200, salvage: 6, research: ["robot_frames"], flags: ["vesper_dead"]),
        on_fail: (threat: 10, flags: ["vesper_escaped"]),
      ),
    ],
    research: [
      (id: "augments", name: "Cybernetics", days: 6, cost: (funds: 150, salvage: 4),
       requires: ["robot_frames"], unlocks: (classes: ["augmented"], items: ["aug_*"])),
    ],
    shop: [ (item: "frag", funds: 25), (item: "taser", funds: 60, requires: ["tasers"]) ],
  ),
]
```

* **Unlocks gate content.** Classes, weapons, items and loadouts that a
  campaign hasn't unlocked are hidden from the Loadout and Barracks screens.
  This is a filter over the `Ruleset`; nothing is removed from it.
* **Items cost money.** Gear comes from the shop into a stockpile. Equipping
  an item takes one from the stock, and it comes back unless the soldier
  dies. Grenades, medkits and other single-use items are consumed.

## 5. Soldiers across missions

These are new fields on `SoldierRecord`, saved:

| Field | Meaning |
|---|---|
| `xp`, `rank` | XP for missions, kills and objectives. Ranks give small stat bumps (+aim, +HP) and unlock a perk choice later. |
| `injury_days` | Badly wounded soldiers (below half health at the end) sit out N days. Saves the existing `wounds` counter. |
| `insanity` | **Carries over** between missions (the existing insanity system). It drops a little per day off duty, and an R&R facility or shop item drops it faster. Lost soldiers are gone. |
| `augments` | Already in `gear.augments`. Surgery costs salvage and days. Removing an augment takes days too. |
| `heat_scarred` | (optional) heat-frame soldiers that overheated come back damaged. |

**Mission results reach the save** through the existing `Roster::record`
(deployed, kills, fallen), extended with each survivor's HP and insanity at
the end, and with objectives from script flags.

## 6. Missions report back

Scripts can already set flags. The campaign reads a few by convention when
the battle ends:

* `win()` / `lose()` give the result.
* Flags named `reward_*` (for example `reward_salvage`), `campaign_*` (copied
  into the campaign's flags, such as `campaign_vesper_dead`) and `intel`
  (logs read: Dead Signal's `logs` count) add to the rewards.
* A new `extracted(unit)` script call (or the evac zones that exist) marks who
  got out. On missions with an evac, soldiers left behind count as **MIA**.

Generated missions get rewards from their objective type and difficulty.

## 7. Screens (HTML HUD, like Loadout and Barracks)

1. **Command**: operations list (name, region, enemy tags, days, deadline,
   rewards), resources, threat bar, day counter, and an "Advance day / Launch"
   button.
2. **Research**: available projects, the active one with days left, and what
   each unlocks.
3. **Shop and stock**: buy and sell gear. The stock count shows next to
   items in Loadout.
4. **Roster**: the existing Barracks plus rank, XP, injury days and insanity.
   Includes recruiting (cost in funds) and the memorial.
5. **Debrief**: the result, rewards, promotions, injuries, the fallen, and
   flags that changed.

Each screen is a `data-if` panel in `hud.html` with `{{ campaign.* }}`
bindings and `campaign:*` commands, following `loadout.rs` and `roster.rs`.

## 8. Implementation phases

1. **Core data** (`tactics_core::content::campaign`): `CampaignDef`,
   `OperationDef`, `ResearchDef`, `ShopEntry` with serde; `Ruleset::campaigns`;
   validation (missions, flags, unlocks exist). Tests: parse, validate.
2. **Campaign state** (`tactics_core::campaign`): `CampaignState` (the save in
   section 3) plus pure functions: `advance_day`, `available_operations`,
   `apply_result(op, BattleResult)`, `start_research`, `buy`, `unlocked(id)`.
   These have no Bevy dependency and are fully unit-tested (deterministic
   with `seed`).
3. **Battle results**: `BattleResult { winner, flags, survivors: [(name, hp,
   insanity)], fallen, kills }` built from a finished `Battle`. Tests use the
   existing missions (for example: win Basic Training and get its rewards).
4. **Game wiring** (`crates/tactics_game/src/campaign.rs`): a `CampaignPlugin`
   resource holding `CampaignState`; load and save via `read_save` /
   `write_save`; launching an operation loads its mission with the campaign
   squad; returning from the battle runs the debrief. The content filter goes
   into `loadout.rs`, `roster.rs` and `weapon_choices`.
5. **Screens**: Command, Debrief, Research, Shop (section 7).
6. **Content**: the main campaign in a new `campaign.ron` module (section 2
   acts); reward flags in the story missions' generators.
7. **Balance and polish**: threat curve, prices, injury lengths. Optional:
   Ironman (one save slot, no reloading) and difficulty presets.

## 9. Open questions

* Should robot frames be built (salvage) or recruited? Proposal: built, from
  salvage, after the VESPER research.
* Should Lost soldiers (insanity 100) appear later as enemies? It's a fun hook
  for Act 5.
* Should there be a base-building layer (facilities), or keep it to
  research and shop? Proposal: start without one, add an R&R / infirmary
  facility later if needed.

# HTML/CSS user interface

The battle HUD is an **HTML file with CSS**:
[`crates/tactics_game/assets/ui/hud.html`](crates/tactics_game/assets/ui/hud.html).
The game renders it **natively** with Bevy's UI. There's no browser or
webview involved, so it behaves identically on Windows, Linux, Android and the
web. The engine supports a practical subset of HTML and CSS, listed below.

## Customising the HUD

| Where | How | Platforms |
|---|---|---|
| `ui/hud.html` next to where you run the game | copy the default file there, edit, press **F6** in game to reload instantly | desktop |
| a module | `ui_layouts: [(id: "hud", html: r#"...your html..."#)]` in a RON module, or `rules.add_ui_layout(...)` in Rust | all |
| the built-in file | edit `assets/ui/hud.html` (compiled into the game) | all |

The first one found wins, in that order. Unsupported CSS properties are
reported in the combat log (`UI: unsupported CSS property 'x'`), and
`cargo test` checks that the built-in HUD uses only supported CSS.

## Dynamic content

### Text bindings: `{{ path }}`

```html
<span>HP {{ unit.hp }}/{{ unit.max_hp }}</span>
```
Text updates live. Unknown paths render empty.

### Conditions: `data-if`

```html
<div data-if="battle.over">...</div>
<div data-if="!editor.open">...</div>
<span data-if="turn.team == XCOM">Your move</span>
<span data-if="unit.status != ''">...</span>
```
Truthy = anything except empty, `0`, `false`, `no`, `off`.

### Repetition: `data-for`

The element's **children** are repeated for each list item:

```html
<div class="bar" data-for="a in abilities">
  <button class="{{ a.state }}" data-command="ability:{{ a.id }}">
    {{ a.name }} <span class="key">{{ a.key }}</span>
  </button>
</div>
```
Inside a repeat, item fields (`{{ a.name }}`) also work in **attributes**
(`class`, `style`, `data-command`, `data-if`...), e.g.
`style="width: {{ m.hp_percent }}%"` for health bars. Attributes outside
repeats are evaluated when the layout is built.

### Actions: `data-command`

Any `<button>` (or element with `data-command`) is clickable/tappable:

| command | effect |
|---|---|
| `end_turn`, `next_unit`, `prev_unit`, `cancel`, `restart` | same as the End Turn button / Tab / Shift+Tab / Esc / F5 |
| `confirm` | fire the aimed ability (Space) |
| `gear:<what>:<member>[:<slot>]`, `apply_loadout` | edit squad loadouts (see the Loadout panel) |
| `ability:<id>` | use the selected unit's ability (or start targeting it) |
| `select:<unit id>` | select a squad member |
| `editor` | toggle the module editor (F1) |
| `cinematics` | toggle cinematic cameras (C) |
| `reload_ui` | rebuild the HUD from its source (F6) |
| `mission:<id>` | start a mission from `assets/missions/` |
| `skirmish` | start a procedural skirmish |
| `toggle:<name>` | flip the layout flag `ui.<name>` (open/close your own panels) |
| `log:<text>` | print to the combat log |

New commands: add a match arm in `run_ui_commands` (`src/html_ui/mod.rs`).

## Data reference

Values (strings; booleans are `true`/`false`):

| path | meaning |
|---|---|
| `turn.round`, `turn.team` (`XCOM`/`ALIEN`), `turn.label`, `turn.is_player` | turn info |
| `squad.alive`, `enemies.alive`, `enemies.label` | living units per side (with fog: hostiles in sight) |
| `mission.concealed`, `mission.alarm`, `mission.objective`, `mission.has_objective` | stealth state and scripted objective |
| `battle.over`, `battle.won`, `battle.result`, `battle.seed` | outcome (set after final animations) |
| `busy` | animations or a cinematic are playing |
| `unit.selected`, `unit.name`, `unit.class`, `unit.hp`, `unit.max_hp`, `unit.hp_percent`, `unit.ap`, `unit.max_ap`, `unit.weapon`, `unit.ammo`, `unit.clip`, `unit.aim`, `unit.mobility`, `unit.defense`, `unit.status`, `unit.gear`, `unit.morale`, `unit.morale_state` (`steady`/`shaken`/`panicked`/`fearless`) | selected unit |
| `targeting`, `targeting.name`, `targeting.aimed` | an ability is waiting for a target / a target is aimed (show a Fire button) |
| `hover.info` | odds breakdown / move cost under the pointer (multi-line) |
| `log.text`, `log.last` | combat log |
| `editor.open`, `cinematics` | toggles |
| `mission.active`, `mission.name`, `mission.description`, `missions.ready` | current mission |
| `ui.<name>` | layout flags set by `toggle:<name>` |

Lists (for `data-for`):

| list | item fields |
|---|---|
| `abilities` | `id name description key cost charges cooldown state` (`state` = `ready` / `active` / `disabled`) |
| `squad` | `id name hp max_hp hp_percent ap fear state` (`state` = `selected` / `ready` / `done`; `fear` = `SHAKEN`/`PANIC`/empty) |
| `loadout` | `index name class preset weapon armor item0 item1 stats problems` (the player squad's gear) |
| `log` | `text` |
| `missions` | `id name description file state` (`state` = `current` / empty) |

New data: add `v.set("path", value)` or a list in `collect_ui_data`
(`src/html_ui/data.rs`).

## Supported HTML

* Any tag name works; each element is a flexbox box. Defaults: `div` and other
  blocks are columns; `span p b strong i em label a small h1-h4 button` are
  rows; `h1-h4` and `small` set font sizes; `button` centres its content.
* `<img src="ui/icon.png">` loads an image from `assets/`.
* `<br>` breaks a row; `<hr>` draws a thin line (style it with `background`).
* `<style>` blocks (any number), `class`, `id`, inline `style="..."`.
* Comments, doctype, entities `&amp; &lt; &gt; &quot; &#39; &nbsp;`.
* Whitespace collapses like HTML; text is trimmed. Use margins/gap for spacing.
* The default font is ASCII-only. Characters like `—` or `·` render as boxes;
  add a font asset if you need them.

## Supported CSS

Selectors: `tag`, `.class`, `#id`, `*`, compounds (`button.primary`),
descendants (`#hud .panel span`; `>` is treated as a descendant), comma lists,
`:hover` and `:active` (background colour of buttons). Normal
specificity/order rules apply; inline styles win. `@media` and other at-rules
are skipped.

| group | properties |
|---|---|
| box | `width height min-width min-height max-width max-height`, `padding margin` (1-4 values and `-top/-right/-bottom/-left`) |
| position | `position: absolute/relative/fixed`, `top right bottom left`, `z-index` |
| flex | `display: flex/none`, `flex-direction`, `flex-wrap`, `justify-content`, `align-items`, `align-self`, `gap row-gap column-gap`, `flex`/`flex-grow`, `flex-shrink`, `flex-basis` |
| look | `background`/`background-color`, `border` (`2px solid #fff`), `border-width`, `border-color`, `border-radius`, `overflow: hidden/scroll/visible` |
| text | `color`, `font-size`, `text-align` (inherited) |

Units: `px`, `%`, `vw`, `vh`, `vmin`, `vmax`, `em`/`rem`, `auto`, bare numbers
(px). Colours: `#rgb #rgba #rrggbb #rrggbbaa`, `rgb()`, `rgba()` (write
`rgba(...)` without spaces inside `border` shorthands), `transparent` and basic
names.

Elements with no background (and all text) let clicks pass through to the
battlefield; elements with a background or buttons capture them.

## Not supported (yet)

Grid layout, transitions/animations, `::before/::after`, `:nth-child`, sibling
combinators, web fonts via CSS, forms/inputs, JavaScript. The engine lives in
`src/html_ui/` (`dom.rs` parser, `css.rs` selectors/cascade, `style.rs`
properties, `build.rs` entities, `bind.rs` templates). Each file says where to
extend it.

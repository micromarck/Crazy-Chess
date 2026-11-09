//! Missions: a map with heights, units, props, zones and scripts, loaded from
//! a small XML format.
//!
//! ```xml
//! <mission id="ambush" name="Ambush" win="eliminate">
//!   <description>Hold the ridge.</description>
//!   <!-- One character per tile, first row = top (north):
//!        . floor   # cut out (no tile)   h half cover   H full cover -->
//!   <layout>
//!     ..........
//!     ..##......
//!     ..##..h...
//!   </layout>
//!   <!-- Optional, same shape: ground level 0-9 per tile. -->
//!   <heights>
//!     2222000000
//!     2200000000
//!     2200000000
//!   </heights>
//!   <ladder x="1" y="2"/>
//!   <deploy team="player" squad="player" x="5" y="2"/>
//!   <unit team="enemy" class="sectoid" name="Leader" x="8" y="0" tag="leader">
//!     <script on="death">message("The leader is down!"); win();</script>
//!   </unit>
//!   <prop tag="barrel" kind="barrel" x="6" y="1" cover="half" hp="2">
//!     <script on="destroyed">explode(x, y, 1.5, 3);</script>
//!   </prop>
//!   <zone tag="exit" x="0" y="0" width="2" height="1">
//!     <script on="enter" team="player">win();</script>
//!   </zone>
//!   <script on="turn_start">if round == 3 { message("Reinforcements!"); }</script>
//! </mission>
//! ```
//!
//! **Coordinates:** `x` counts columns from the left and `y` counts **rows
//! from the top** of the layout, exactly as you read the text. (The engine's
//! grid has y growing north; the parser converts.)
//!
//! See `MISSIONS.md` for every element, attribute, script event and script
//! function.

use crate::grid::{CoverLevel, GridPos, Map, Terrain};
use crate::unit::Team;

/// When a script runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// Mission: once, when the battle begins.
    Start,
    /// Mission: every turn start. Units: at the start of their own team's turn.
    TurnStart,
    /// Mission: whenever any unit dies.
    UnitKilled,
    /// Zones/tiles: a unit steps onto the area.
    Enter,
    /// Units: this unit dies.
    Death,
    /// Units and props: this one takes damage.
    Damaged,
    /// Units: the player's squad sees this unit for the first time.
    Spotted,
    /// Props: this prop is destroyed.
    Destroyed,
    /// Mission: the alarm was raised (stealth broken, all enemies wake).
    Alarm,
    /// Doors: this door opened.
    Opened,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "start" => Self::Start,
            "turn_start" => Self::TurnStart,
            "unit_killed" => Self::UnitKilled,
            "enter" => Self::Enter,
            "death" => Self::Death,
            "damaged" => Self::Damaged,
            "spotted" => Self::Spotted,
            "destroyed" => Self::Destroyed,
            "alarm" => Self::Alarm,
            "opened" => Self::Opened,
            _ => return None,
        })
    }
}

/// A script attached to something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptHook {
    pub on: HookEvent,
    /// Only for units of this team (`enter`, `turn_start`...).
    pub team: Option<Team>,
    /// Run at most once.
    pub once: bool,
    /// Rhai source code.
    pub code: String,
}

/// A unit placed by the mission.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitPlacement {
    pub team: Team,
    pub class: String,
    pub name: String,
    pub pos: GridPos,
    /// Name scripts can use to find the unit (`unit_by_tag("leader")`).
    pub tag: Option<String>,
    pub hooks: Vec<ScriptHook>,
    /// Starts inactive (see [`crate::unit::UnitAi`]).
    pub dormant: bool,
    /// Pod name: dormant units in the same pod wake together.
    pub pod: Option<String>,
    /// Patrol waypoints walked while dormant.
    pub patrol: Vec<GridPos>,
    /// Loadout preset id.
    pub loadout: Option<String>,
    /// Individual gear (`weapon`, `armor`, `items` attributes).
    pub gear: crate::content::Loadout,
}

/// "Deploy the ruleset's squad `squad` for `team` around `pos`."
#[derive(Debug, Clone, PartialEq)]
pub struct DeployDef {
    pub team: Team,
    pub squad: String,
    pub pos: GridPos,
}

/// An object on a tile: gives cover, can be destructible, can run scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct PropDef {
    /// Name scripts use (`destroy("barrel")`).
    pub tag: String,
    /// Free-form type (`crate`, `barrel`, `console`...), used for visuals.
    pub kind: String,
    pub pos: GridPos,
    /// Cover it gives (and whether it blocks movement: any cover blocks).
    pub cover: CoverLevel,
    /// Hit points; `None` = indestructible.
    pub hp: Option<i32>,
    /// Model id (from a `ModelDef`) to draw it with.
    pub model: Option<String>,
    pub hooks: Vec<ScriptHook>,
}

/// A door: blocks movement and sight; opens when a unit steps next to it
/// (unless locked). Each tile of the rectangle is one door panel sharing `tag`.
#[derive(Debug, Clone, PartialEq)]
pub struct DoorDef {
    pub tag: String,
    pub tiles: Vec<GridPos>,
    pub locked: bool,
    /// Hooks (`opened`) on the first panel.
    pub hooks: Vec<ScriptHook>,
}

/// A rectangle of tiles with scripts (a single `<tile>` is a 1×1 zone).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneDef {
    pub tag: String,
    /// Inclusive corners in grid coordinates.
    pub min: GridPos,
    pub max: GridPos,
    pub hooks: Vec<ScriptHook>,
    /// Shown on the board: `evac`, `objective`, `danger` (or any word, drawn
    /// like `objective`). `None` = invisible trigger area. Zones tagged
    /// `evac*`, `extract*` or `exit*` are marked `evac` automatically.
    pub marker: Option<String>,
    /// Text floating over a marked zone (default: from the marker).
    pub label: Option<String>,
    /// Marked but not shown until a script calls `show_zone(tag, true)`.
    pub hidden: bool,
}

impl ZoneDef {
    pub fn contains(&self, p: GridPos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}

/// What fills the background behind the level (`<sky>` in a mission file).
/// Nothing set = the game's default sky colour.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkyDef {
    /// A solid colour (sRGB).
    pub color: Option<[u8; 3]>,
    /// A vertical gradient: the colour straight up...
    pub top: Option<[u8; 3]>,
    /// ...and at the horizon (and below).
    pub horizon: Option<[u8; 3]>,
    /// A panorama image (equirectangular, 2:1), asset path such as
    /// `skies/dusk.png`. Wins over the colours, which still show while it loads.
    pub image: Option<String>,
    /// Turns the image around the vertical axis, degrees.
    pub rotation: f32,
}

impl SkyDef {
    /// Parses `#rgb` or `#rrggbb`.
    pub fn parse_color(text: &str) -> Option<[u8; 3]> {
        let h = text.trim().strip_prefix('#')?;
        let byte = |s: &str| u8::from_str_radix(s, 16).ok();
        match h.len() {
            3 => {
                let c: Vec<u8> = h.chars().map(|c| byte(&c.to_string()).map(|v| v * 17)).collect::<Option<_>>()?;
                Some([c[0], c[1], c[2]])
            }
            6 => Some([byte(&h[0..2])?, byte(&h[2..4])?, byte(&h[4..6])?]),
            _ => None,
        }
    }
}

/// Everything a mission file defines.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub map: Map,
    pub units: Vec<UnitPlacement>,
    pub deploys: Vec<DeployDef>,
    pub props: Vec<PropDef>,
    pub zones: Vec<ZoneDef>,
    pub doors: Vec<DoorDef>,
    /// Enemies are only shown while the squad can see them.
    pub fog: bool,
    /// The squad starts concealed: unalerted enemies only notice soldiers
    /// within `detection` tiles; being noticed or attacking raises the alarm.
    pub concealment: bool,
    /// Detection radius (tiles) for concealment.
    pub detection: f32,
    /// Mission-level hooks (`start`, `turn_start`, `unit_killed`, `alarm`).
    pub hooks: Vec<ScriptHook>,
    /// Shared script code (functions) available to every hook.
    pub library: String,
    pub seed: Option<u64>,
    /// `true`: killing every enemy wins. `false`: only scripts (`win()`) can.
    pub eliminate_to_win: bool,
    /// The background (`<sky>`).
    pub sky: SkyDef,
}

#[cfg(feature = "missions")]
pub use xml::{parse_index, parse_mission};

#[cfg(feature = "missions")]
mod xml {
    use super::*;
    use roxmltree::{Document, Node};

    /// An error with line/column.
    fn err(doc: &Document, node: Node, msg: impl std::fmt::Display) -> String {
        let p = doc.text_pos_at(node.range().start);
        format!("line {}, column {}: <{}>: {msg}", p.row, p.col, node.tag_name().name())
    }

    fn attr_i32(doc: &Document, n: Node, name: &str) -> Result<Option<i32>, String> {
        n.attribute(name)
            .map(|v| v.trim().parse::<i32>().map_err(|_| err(doc, n, format!("'{name}' must be a whole number, got '{v}'"))))
            .transpose()
    }

    fn req_i32(doc: &Document, n: Node, name: &str) -> Result<i32, String> {
        attr_i32(doc, n, name)?.ok_or_else(|| err(doc, n, format!("missing attribute '{name}'")))
    }

    fn team(doc: &Document, n: Node) -> Result<Team, String> {
        match n.attribute("team").unwrap_or("enemy").trim() {
            "player" | "xcom" => Ok(Team::Player),
            "enemy" | "alien" => Ok(Team::Enemy),
            t => Err(err(doc, n, format!("unknown team '{t}' (use player or enemy)"))),
        }
    }

    /// Text rows of a grid block: trimmed, empty lines skipped.
    fn rows(n: Node) -> Vec<String> {
        n.text()
            .unwrap_or("")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    }

    fn scripts(doc: &Document, n: Node) -> Result<Vec<ScriptHook>, String> {
        n.children()
            .filter(|c| c.has_tag_name("script") && c.attribute("on").is_some())
            .map(|c| {
                let on = c.attribute("on").unwrap_or("");
                let on = HookEvent::parse(on).ok_or_else(|| err(doc, c, format!("unknown event '{on}'")))?;
                let team = if c.attribute("team").is_some() { Some(team(doc, c)?) } else { None };
                Ok(ScriptHook {
                    on,
                    team,
                    once: c.attribute("once").is_some_and(|v| v == "true"),
                    code: c.text().unwrap_or("").to_string(),
                })
            })
            .collect()
    }

    /// Wraps the body of every `<script>` element in CDATA (unless it already
    /// is), so script code can use `<`, `>` and `&&` without XML escaping.
    /// Doesn't add newlines, so error line numbers stay correct.
    fn protect_scripts(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 64);
        let mut rest = text;
        while let Some(start) = rest.find("<script") {
            // Skip <!-- comments --> that start before this tag.
            if let Some(c) = rest[..start].rfind("<!--")
                && !rest[c..start].contains("-->") {
                    let end = rest[c..].find("-->").map_or(rest.len(), |e| c + e + 3);
                    out.push_str(&rest[..end]);
                    rest = &rest[end..];
                    continue;
                }
            let Some(open_end) = rest[start..].find('>').map(|i| start + i) else { break };
            out.push_str(&rest[..=open_end]);
            rest = &rest[open_end + 1..];
            if rest.is_empty() || text.as_bytes()[text.len() - rest.len() - 2] == b'/' {
                continue; // self-closing <script/>
            }
            let Some(close) = rest.find("</script>") else { break };
            let body = &rest[..close];
            if body.contains("<![CDATA[") {
                out.push_str(body);
            } else {
                out.push_str("<![CDATA[");
                out.push_str(body);
                out.push_str("]]>");
            }
            rest = &rest[close..];
        }
        out.push_str(rest);
        out
    }

    /// Parses a mission file. Errors name the line and column.
    pub fn parse_mission(text: &str) -> Result<MissionDef, String> {
        let text = protect_scripts(text);
        let doc = Document::parse(&text).map_err(|e| format!("XML error: {e}"))?;
        let root = doc.root_element();
        if !root.has_tag_name("mission") {
            return Err(err(&doc, root, "the root element must be <mission>"));
        }

        // --- grid -------------------------------------------------------
        let layout = root.children().find(|c| c.has_tag_name("layout"));
        let layout_rows = layout.map(rows).unwrap_or_default();
        let width = attr_i32(&doc, root, "width")?
            .unwrap_or_else(|| layout_rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as i32);
        let height = attr_i32(&doc, root, "height")?.unwrap_or(layout_rows.len() as i32);
        if width <= 0 || height <= 0 {
            return Err(err(&doc, root, "the map is empty: add a <layout> or width/height attributes"));
        }
        let mut map = Map::new(width, height);
        // XML y = rows from the top; grid y grows north.
        let at = |x: i32, y: i32| GridPos::new(x, height - 1 - y);

        for (row, line) in layout_rows.iter().enumerate() {
            for (x, ch) in line.chars().enumerate() {
                let terrain = match ch {
                    '#' | ' ' | '_' => Terrain::Void,
                    'h' => Terrain::LowCover,
                    'H' => Terrain::HighCover,
                    _ => Terrain::Floor,
                };
                map.set_terrain(at(x as i32, row as i32), terrain);
            }
        }
        if let Some(sf) = root.children().find(|c| c.has_tag_name("surfaces")) {
            for (row, line) in rows(sf).iter().enumerate() {
                for (x, ch) in line.chars().enumerate() {
                    if let Some(s) = crate::grid::Surface::from_char(ch) {
                        map.set_surface(at(x as i32, row as i32), s);
                    }
                }
            }
        }
        let auto_style = root.attribute("style") != Some("none") && !root.children().any(|c| c.has_tag_name("surfaces"));
        if let Some(h) = root.children().find(|c| c.has_tag_name("heights")) {
            for (row, line) in rows(h).iter().enumerate() {
                for (x, ch) in line.chars().enumerate() {
                    if let Some(d) = ch.to_digit(10) {
                        map.set_elevation(at(x as i32, row as i32), d as i8);
                    }
                }
            }
        }

        // Rectangle helpers, applied in document order.
        let rect = |n: Node| -> Result<Vec<GridPos>, String> {
            let (x, y) = (req_i32(&doc, n, "x")?, req_i32(&doc, n, "y")?);
            let (w, h) = (attr_i32(&doc, n, "width")?.unwrap_or(1), attr_i32(&doc, n, "height")?.unwrap_or(1));
            Ok((y..y + h).flat_map(|yy| (x..x + w).map(move |xx| (xx, yy))).map(|(xx, yy)| at(xx, yy)).collect())
        };
        for n in root.children().filter(Node::is_element) {
            match n.tag_name().name() {
                "cutout" => rect(n)?.into_iter().for_each(|p| map.set_terrain(p, Terrain::Void)),
                "raise" => {
                    let level = req_i32(&doc, n, "level")? as i8;
                    rect(n)?.into_iter().for_each(|p| map.set_elevation(p, level));
                }
                "ladder" => rect(n)?.into_iter().for_each(|p| map.set_ladder(p, true)),
                "terrain" => {
                    let t = match n.attribute("type").unwrap_or("floor") {
                        "floor" => Terrain::Floor,
                        "half" => Terrain::LowCover,
                        "full" => Terrain::HighCover,
                        "void" => Terrain::Void,
                        other => return Err(err(&doc, n, format!("unknown terrain type '{other}'"))),
                    };
                    rect(n)?.into_iter().for_each(|p| map.set_terrain(p, t));
                }
                _ => {}
            }
        }

        // --- contents ---------------------------------------------------
        if auto_style {
            map.auto_surfaces();
        }
        let mut def = MissionDef {
            id: root.attribute("id").unwrap_or("mission").to_string(),
            name: root.attribute("name").unwrap_or("Mission").to_string(),
            description: root
                .children()
                .find(|c| c.has_tag_name("description"))
                .and_then(|d| d.text())
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
            map,
            units: Vec::new(),
            deploys: Vec::new(),
            props: Vec::new(),
            zones: Vec::new(),
            doors: Vec::new(),
            fog: root.attribute("fog") == Some("true"),
            concealment: root.attribute("concealment") == Some("true"),
            detection: root.attribute("detection").and_then(|d| d.trim().parse().ok()).unwrap_or(5.0),
            hooks: scripts(&doc, root)?,
            library: String::new(),
            seed: root.attribute("seed").and_then(|s| s.trim().parse().ok()),
            eliminate_to_win: root.attribute("win").unwrap_or("eliminate") != "script",
            sky: SkyDef::default(),
        };

        let pos = |n: Node| -> Result<GridPos, String> {
            let p = at(req_i32(&doc, n, "x")?, req_i32(&doc, n, "y")?);
            if !def_in_bounds(width, height, p) {
                return Err(err(&doc, n, "position is outside the map"));
            }
            Ok(p)
        };
        for n in root.children().filter(Node::is_element) {
            match n.tag_name().name() {
                "unit" => def.units.push(UnitPlacement {
                    team: team(&doc, n)?,
                    class: n.attribute("class").ok_or_else(|| err(&doc, n, "missing attribute 'class'"))?.to_string(),
                    name: n.attribute("name").unwrap_or("Unit").to_string(),
                    pos: pos(n)?,
                    tag: n.attribute("tag").map(String::from),
                    hooks: scripts(&doc, n)?,
                    dormant: n.attribute("dormant") == Some("true"),
                    loadout: n.attribute("loadout").map(String::from),
                    gear: crate::content::Loadout {
                        weapon: n.attribute("weapon").map(String::from),
                        armor: n.attribute("armor").map(String::from),
                        items: n
                            .attribute("items")
                            .map(|t| t.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect())
                            .unwrap_or_default(),
                        augments: n
                            .attribute("augments")
                            .map(|t| t.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect())
                            .unwrap_or_default(),
                    },
                    pod: n.attribute("pod").map(String::from),
                    patrol: match n.attribute("patrol") {
                        None => Vec::new(),
                        Some(text) => text
                            .split_whitespace()
                            .map(|pair| {
                                let (x, y) = pair
                                    .split_once(',')
                                    .and_then(|(x, y)| Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?)))
                                    .ok_or_else(|| err(&doc, n, format!("bad patrol point '{pair}' (use \"x,y x,y ...\")")))?;
                                let p = at(x, y);
                                if !def_in_bounds(width, height, p) {
                                    return Err(err(&doc, n, format!("patrol point '{pair}' is outside the map")));
                                }
                                Ok(p)
                            })
                            .collect::<Result<Vec<_>, String>>()?,
                    },
                }),
                "door" => def.doors.push(DoorDef {
                    tag: n.attribute("tag").unwrap_or("door").to_string(),
                    tiles: rect(n)?,
                    locked: n.attribute("locked") == Some("true"),
                    hooks: scripts(&doc, n)?,
                }),
                "deploy" => def.deploys.push(DeployDef {
                    team: team(&doc, n)?,
                    squad: n.attribute("squad").unwrap_or("player").to_string(),
                    pos: pos(n)?,
                }),
                "prop" => def.props.push(PropDef {
                    tag: n.attribute("tag").or(n.attribute("kind")).unwrap_or("prop").to_string(),
                    kind: n.attribute("kind").unwrap_or("crate").to_string(),
                    pos: pos(n)?,
                    cover: match n.attribute("cover").unwrap_or("half") {
                        "none" => CoverLevel::None,
                        "half" => CoverLevel::Half,
                        "full" => CoverLevel::Full,
                        other => return Err(err(&doc, n, format!("unknown cover '{other}' (none, half, full)"))),
                    },
                    hp: attr_i32(&doc, n, "hp")?,
                    model: n.attribute("model").map(String::from),
                    hooks: scripts(&doc, n)?,
                }),
                "zone" | "tile" => {
                    let tiles = rect(n)?;
                    let (min, max) = tiles.iter().fold((tiles[0], tiles[0]), |(lo, hi), p| {
                        (GridPos::new(lo.x.min(p.x), lo.y.min(p.y)), GridPos::new(hi.x.max(p.x), hi.y.max(p.y)))
                    });
                    let tag = n.attribute("tag").unwrap_or("zone").to_string();
                    let lower = tag.to_lowercase();
                    let auto_evac = ["evac", "extract", "exit"].iter().any(|w| lower.starts_with(w));
                    let marker = match n.attribute("marker") {
                        Some("none") => None,
                        Some(m) => Some(m.to_string()),
                        None if auto_evac => Some("evac".to_string()),
                        None => None,
                    };
                    def.zones.push(ZoneDef {
                        tag,
                        min,
                        max,
                        hooks: scripts(&doc, n)?,
                        marker,
                        label: n.attribute("label").map(String::from),
                        hidden: n.attribute("hidden") == Some("true"),
                    });
                }
                "sky" => {
                    let color = |name: &str| -> Result<Option<[u8; 3]>, String> {
                        match n.attribute(name) {
                            None => Ok(None),
                            Some(t) => SkyDef::parse_color(t)
                                .map(Some)
                                .ok_or_else(|| err(&doc, n, format!("bad {name} colour '{t}' (use #rgb or #rrggbb)"))),
                        }
                    };
                    def.sky = SkyDef {
                        color: color("color")?,
                        top: color("top")?,
                        horizon: color("horizon")?,
                        image: n.attribute("image").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                        rotation: n.attribute("rotation").and_then(|r| r.trim().parse().ok()).unwrap_or(0.0),
                    };
                    if def.sky.top.is_some() != def.sky.horizon.is_some() {
                        return Err(err(&doc, n, "a gradient sky needs both 'top' and 'horizon'"));
                    }
                }
                "script" if n.attribute("on").is_none() => {
                    def.library.push_str(n.text().unwrap_or(""));
                    def.library.push('\n');
                }
                "layout" | "heights" | "surfaces" | "description" | "script" | "cutout" | "raise" | "ladder" | "terrain" => {}
                other => return Err(err(&doc, n, format!("unknown element <{other}>"))),
            }
        }
        Ok(def)
    }

    fn def_in_bounds(w: i32, h: i32, p: GridPos) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < w && p.y < h
    }

    /// Parses `missions/index.xml`: `<missions start="id"><mission file="a.mission.xml"/>…</missions>`.
    /// Returns the file names and the optional starting mission id.
    pub fn parse_index(text: &str) -> Result<(Vec<String>, Option<String>), String> {
        let doc = Document::parse(text).map_err(|e| format!("XML error: {e}"))?;
        let root = doc.root_element();
        let files = root
            .children()
            .filter(|c| c.has_tag_name("mission"))
            .filter_map(|c| c.attribute("file").map(String::from))
            .collect();
        Ok((files, root.attribute("start").map(String::from)))
    }
}

#[cfg(all(test, feature = "missions"))]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        <mission id="t" name="Test" win="script" seed="7">
          <description>  A   test
            mission. </description>
          <layout>
            ....#
            .h..#
            ...H.
          </layout>
          <heights>
            22100
            22000
            00000
          </heights>
          <ladder x="2" y="0"/>
          <raise x="4" y="2" level="3"/>
          <deploy team="player" squad="player" x="0" y="2"/>
          <unit team="enemy" class="sectoid" name="Boss" x="3" y="0" tag="boss">
            <script on="death">win();</script>
          </unit>
          <prop tag="b1" kind="barrel" x="2" y="2" cover="half" hp="2">
            <script on="destroyed">explode(x, y, 1.5, 3);</script>
          </prop>
          <zone tag="exit" x="0" y="0" width="2" height="1">
            <script on="enter" team="player" once="true">message("hi");</script>
          </zone>
          <script>fn helper() { 1 }</script>
          <script on="turn_start">helper();</script>
        </mission>
    "#;

    #[test]
    fn sky_colour_gradient_and_image() {
        let with = |sky: &str| parse_mission(&SAMPLE.replace("<layout>", &format!("{sky}\n<layout>")));
        assert_eq!(parse_mission(SAMPLE).unwrap().sky, SkyDef::default(), "no <sky>: default");
        assert_eq!(with(r##"<sky color="#88aacc"/>"##).unwrap().sky.color, Some([0x88, 0xaa, 0xcc]));
        assert_eq!(with(r##"<sky color="#fa0"/>"##).unwrap().sky.color, Some([0xff, 0xaa, 0x00]), "short form");
        let g = with(r##"<sky top="#000010" horizon="#ffffff"/>"##).unwrap().sky;
        assert_eq!((g.top, g.horizon), (Some([0, 0, 0x10]), Some([255, 255, 255])));
        let i = with(r##"<sky image="skies/dusk.png" rotation="90"/>"##).unwrap().sky;
        assert_eq!((i.image.as_deref(), i.rotation), (Some("skies/dusk.png"), 90.0));
        assert!(with(r##"<sky color="blue"/>"##).unwrap_err().contains("bad color colour"));
        assert!(with(r##"<sky top="#000"/>"##).unwrap_err().contains("needs both"));
    }

    #[test]
    fn parses_everything_with_top_down_coordinates() {
        let m = parse_mission(SAMPLE).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((m.map.width(), m.map.height()), (5, 3));
        assert_eq!(m.description, "A test mission.");
        assert!(!m.eliminate_to_win);
        assert_eq!(m.seed, Some(7));
        // Top-left (x=0, y=0 in XML) is grid (0, 2).
        assert_eq!(m.map.elevation(GridPos::new(0, 2)), 2);
        assert_eq!(m.map.terrain(GridPos::new(4, 2)), Terrain::Void, "# cut out");
        assert_eq!(m.map.terrain(GridPos::new(1, 1)), Terrain::LowCover);
        assert!(m.map.is_ladder(GridPos::new(2, 2)));
        assert_eq!(m.map.elevation(GridPos::new(4, 0)), 3, "raise rect");
        assert_eq!(m.units[0].pos, GridPos::new(3, 2));
        assert_eq!(m.units[0].hooks[0].on, HookEvent::Death);
        assert_eq!(m.props[0].hp, Some(2));
        assert_eq!(m.zones[0].min, GridPos::new(0, 2));
        assert_eq!(m.zones[0].max, GridPos::new(1, 2));
        assert!(m.zones[0].hooks[0].once);
        assert_eq!(m.zones[0].hooks[0].team, Some(Team::Player));
        assert!(m.library.contains("fn helper"));
        assert_eq!(m.hooks.len(), 1);
    }

    #[test]
    fn surfaces_layer_sets_tile_looks() {
        let m = parse_mission(
            r#"<mission id="s">
                 <layout>
                   ..H
                   ...
                 </layout>
                 <surfaces>
                   agB
                   wr.
                 </surfaces>
               </mission>"#,
        )
        .unwrap();
        use crate::grid::Surface;
        // Row 0 of the text is the top (grid y = 1).
        assert_eq!(m.map.surface(GridPos::new(0, 1)), Surface::Asphalt);
        assert_eq!(m.map.surface(GridPos::new(2, 1)), Surface::Brick);
        assert_eq!(m.map.surface(GridPos::new(1, 0)), Surface::Roof);
        assert_eq!(m.map.surface(GridPos::new(2, 0)), Surface::Default);
    }

    #[test]
    fn stealth_attributes_doors_and_patrols() {
        let m = parse_mission(
            r#"<mission fog="true" concealment="true" detection="4.5">
                 <layout>
                   ......
                   ......
                   ......
                 </layout>
                 <door tag="vault" x="2" y="0" width="2" locked="true"><script on="opened">message("in");</script></door>
                 <unit class="sectoid" x="0" y="0" dormant="true" pod="a" patrol="0,0 5,0 5,2"/>
               </mission>"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(m.fog && m.concealment);
        assert_eq!(m.detection, 4.5);
        assert_eq!(m.doors[0].tiles, vec![GridPos::new(2, 2), GridPos::new(3, 2)]);
        assert!(m.doors[0].locked);
        let u = &m.units[0];
        assert!(u.dormant);
        assert_eq!(u.pod.as_deref(), Some("a"));
        assert_eq!(u.patrol, vec![GridPos::new(0, 2), GridPos::new(5, 2), GridPos::new(5, 0)]);
        assert!(parse_mission(r#"<mission><layout>..</layout><unit class="a" x="0" y="0" patrol="9,9"/></mission>"#).is_err());
    }

    #[test]
    fn errors_point_at_the_problem() {
        let e = parse_mission("<mission><layout>...</layout>\n<unit team=\"enemy\" x=\"1\" y=\"0\"/></mission>").unwrap_err();
        assert!(e.contains("line 2") && e.contains("class"), "{e}");
        let e = parse_mission("<mission><layout>..</layout><unit class=\"a\" x=\"9\" y=\"0\"/></mission>").unwrap_err();
        assert!(e.contains("outside"), "{e}");
        assert!(parse_mission("<mission>").unwrap_err().contains("XML"));
        let e = parse_mission("<mission><layout>..</layout><spaceship/></mission>").unwrap_err();
        assert!(e.contains("unknown element"), "{e}");
    }

    #[test]
    fn script_code_needs_no_xml_escaping() {
        let m = parse_mission(
            "<mission><layout>..</layout>\n<!-- <script> in a comment -->\n<script on=\"start\">if 1 < 2 && 3 > 2 { win(); }</script><script on=\"turn_start\"><![CDATA[ a && b ]]></script></mission>",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.hooks[0].code, "if 1 < 2 && 3 > 2 { win(); }");
        assert_eq!(m.hooks[1].code.trim(), "a && b");
    }

    #[test]
    fn index() {
        let (files, start) = parse_index(r#"<missions start="b"><mission file="a.xml"/><mission file="b.xml"/></missions>"#).unwrap();
        assert_eq!(files, ["a.xml", "b.xml"]);
        assert_eq!(start.as_deref(), Some("b"));
    }
}

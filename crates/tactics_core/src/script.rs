//! Mission scripting with [Rhai](https://rhai.rs).
//!
//! Scripts don't touch the battle directly. Each run gets a read-only
//! [`ScriptWorld`] snapshot plus the trigger's variables and returns a list of
//! [`ScriptCommand`]s that the battle then applies in order. This keeps the
//! rules engine deterministic (same seed + actions + scripts = same battle) and
//! means a broken script can only produce an error message, never corrupt
//! state. Runs are capped (operations, call depth, string size), so an
//! infinite loop just stops with an error.
//!
//! ## Variables available in every script
//! `round`, `team` (active team: `"player"`/`"enemy"`), `unit` (the unit the
//! event is about, or -1), `prop` (prop tag or `""`), `x`, `y` (event tile),
//! `amount` (damage for `damaged`).
//!
//! ## Functions
//! See `register_api` in this file (and `MISSIONS.md`) for the full list: messages,
//! spawning, damage/heal/kill, explosions, terrain/height/ladder changes,
//! action points, win/lose, flags and queries about units, props and tiles.

use std::collections::BTreeMap;

use crate::grid::{GridPos, Map, Terrain};
use crate::unit::Team;

/// A change requested by a script, applied by the battle.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptCommand {
    Message(String),
    Spawn { team: Team, class: String, name: String, pos: GridPos },
    Damage { unit: u32, amount: i32 },
    Heal { unit: u32, amount: i32 },
    Kill { unit: u32 },
    DamageProp { tag: String, amount: i32 },
    DestroyProp { tag: String },
    Explode { pos: GridPos, radius: f32, damage: i32 },
    SetTerrain { pos: GridPos, terrain: Terrain },
    SetHeight { pos: GridPos, level: i8 },
    SetLadder { pos: GridPos, ladder: bool },
    GiveAp { unit: u32, amount: i32 },
    SetFlag { name: String, value: i64 },
    Win,
    Lose,
}

/// What a script can see.
#[derive(Debug, Clone, Default)]
pub struct ScriptWorld {
    pub round: i64,
    pub active_team: Option<Team>,
    pub units: Vec<WorldUnit>,
    pub props: Vec<WorldProp>,
    pub map: Option<Map>,
    pub flags: BTreeMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct WorldUnit {
    pub id: u32,
    pub team: Team,
    pub name: String,
    pub tag: Option<String>,
    pub pos: GridPos,
    pub hp: i32,
    pub max_hp: i32,
    pub alive: bool,
}

#[derive(Debug, Clone)]
pub struct WorldProp {
    pub tag: String,
    pub pos: GridPos,
    pub hp: Option<i32>,
    pub alive: bool,
}

/// The trigger's variables.
#[derive(Debug, Clone, Default)]
pub struct ScriptVars {
    pub unit: Option<u32>,
    pub prop: Option<String>,
    pub pos: Option<GridPos>,
    pub amount: i64,
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Player => "player",
        Team::Enemy => "enemy",
    }
}

fn parse_team(s: &str) -> Option<Team> {
    match s {
        "player" | "xcom" => Some(Team::Player),
        "enemy" | "alien" => Some(Team::Enemy),
        _ => None,
    }
}

fn parse_terrain(s: &str) -> Option<Terrain> {
    Some(match s {
        "floor" => Terrain::Floor,
        "half" => Terrain::LowCover,
        "full" => Terrain::HighCover,
        "void" => Terrain::Void,
        _ => return None,
    })
}

/// Runs `code` (with `library` functions in scope). Returns the commands it
/// issued, or an error message (syntax error, runtime error, limit hit...).
#[cfg(feature = "scripting")]
pub fn run(code: &str, library: &str, vars: &ScriptVars, world: &ScriptWorld) -> Result<Vec<ScriptCommand>, String> {
    use std::sync::{Arc, Mutex};

    use rhai::{Engine, Scope};

    let out = Arc::new(Mutex::new(Out { commands: Vec::new(), flags: world.flags.clone() }));
    let mut engine = Engine::new();
    engine.set_max_operations(100_000);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(10_000);
    engine.set_max_array_size(10_000);
    register_api(&mut engine, &out, Arc::new(world.clone()));

    let mut scope = Scope::new();
    scope.push_constant("round", world.round);
    scope.push_constant("team", world.active_team.map_or("", team_name).to_string());
    scope.push_constant("unit", vars.unit.map_or(-1, |u| u as i64));
    scope.push_constant("prop", vars.prop.clone().unwrap_or_default());
    scope.push_constant("x", vars.pos.map_or(-1, |p| p.x as i64));
    // Scripts use the same top-down y as the mission file.
    let h = world.map.as_ref().map_or(0, |m| m.height() as i64);
    scope.push_constant("y", vars.pos.map_or(-1, |p| h - 1 - p.y as i64));
    scope.push_constant("amount", vars.amount);

    let source = format!("{library}\n{code}");
    engine.run_with_scope(&mut scope, &source).map_err(|e| e.to_string())?;
    let commands = std::mem::take(&mut out.lock().unwrap().commands);
    Ok(commands)
}

#[cfg(not(feature = "scripting"))]
pub fn run(_: &str, _: &str, _: &ScriptVars, _: &ScriptWorld) -> Result<Vec<ScriptCommand>, String> {
    Err("scripting is disabled (enable the `scripting` feature of tactics_core)".into())
}

#[cfg(feature = "scripting")]
struct Out {
    commands: Vec<ScriptCommand>,
    /// Flags as seen by this script (includes its own `set_flag`s).
    flags: BTreeMap<String, i64>,
}

/// Registers every script function. **Add new script functions here** (and a
/// [`ScriptCommand`] variant handled in `Battle::apply_script_command` if the
/// function changes the battle).
#[cfg(feature = "scripting")]
fn register_api(engine: &mut rhai::Engine, out: &std::sync::Arc<std::sync::Mutex<Out>>, world: std::sync::Arc<ScriptWorld>) {
    use rhai::{EvalAltResult, FLOAT, INT};

    let h = world.map.as_ref().map_or(0, |m| m.height());
    // Scripts use top-down y like mission files.
    let grid = move |x: INT, y: INT| GridPos::new(x as i32, h - 1 - y as i32);
    type R<T> = Result<T, Box<EvalAltResult>>;
    let fail = |msg: String| -> Box<EvalAltResult> { msg.into() };

    macro_rules! cmd {
        ($name:literal, |$($arg:ident : $ty:ty),*| $body:expr) => {{
            let out = out.clone();
            engine.register_fn($name, move |$($arg: $ty),*| -> R<()> {
                let c: R<ScriptCommand> = $body;
                out.lock().unwrap().commands.push(c?);
                Ok(())
            });
        }};
    }

    // --- actions ------------------------------------------------------------
    cmd!("message", |text: &str| Ok(ScriptCommand::Message(text.to_string())));
    cmd!("spawn_unit", |team: &str, class: &str, name: &str, x: INT, y: INT| {
        let team = parse_team(team).ok_or_else(|| fail(format!("unknown team '{team}'")))?;
        Ok(ScriptCommand::Spawn { team, class: class.into(), name: name.into(), pos: grid(x, y) })
    });
    cmd!("damage", |unit: INT, amount: INT| Ok(ScriptCommand::Damage { unit: unit as u32, amount: amount as i32 }));
    cmd!("heal", |unit: INT, amount: INT| Ok(ScriptCommand::Heal { unit: unit as u32, amount: amount as i32 }));
    cmd!("kill", |unit: INT| Ok(ScriptCommand::Kill { unit: unit as u32 }));
    cmd!("damage_prop", |tag: &str, amount: INT| Ok(ScriptCommand::DamageProp { tag: tag.into(), amount: amount as i32 }));
    cmd!("destroy", |tag: &str| Ok(ScriptCommand::DestroyProp { tag: tag.into() }));
    cmd!("explode", |x: INT, y: INT, radius: FLOAT, damage: INT| {
        Ok(ScriptCommand::Explode { pos: grid(x, y), radius: radius as f32, damage: damage as i32 })
    });
    cmd!("explode", |x: INT, y: INT, radius: INT, damage: INT| {
        Ok(ScriptCommand::Explode { pos: grid(x, y), radius: radius as f32, damage: damage as i32 })
    });
    cmd!("set_terrain", |x: INT, y: INT, kind: &str| {
        let terrain = parse_terrain(kind).ok_or_else(|| fail(format!("unknown terrain '{kind}' (floor, half, full, void)")))?;
        Ok(ScriptCommand::SetTerrain { pos: grid(x, y), terrain })
    });
    cmd!("set_height", |x: INT, y: INT, level: INT| Ok(ScriptCommand::SetHeight { pos: grid(x, y), level: level.clamp(0, 9) as i8 }));
    cmd!("set_ladder", |x: INT, y: INT, on: bool| Ok(ScriptCommand::SetLadder { pos: grid(x, y), ladder: on }));
    cmd!("give_ap", |unit: INT, amount: INT| Ok(ScriptCommand::GiveAp { unit: unit as u32, amount: amount as i32 }));
    cmd!("win", | | Ok(ScriptCommand::Win));
    cmd!("lose", | | Ok(ScriptCommand::Lose));
    {
        let out = out.clone();
        engine.register_fn("set_flag", move |name: &str, value: INT| {
            let mut o = out.lock().unwrap();
            o.flags.insert(name.to_string(), value);
            o.commands.push(ScriptCommand::SetFlag { name: name.to_string(), value });
        });
    }
    {
        let out = out.clone();
        engine.register_fn("flag", move |name: &str| -> INT { out.lock().unwrap().flags.get(name).copied().unwrap_or(0) });
    }
    {
        let out = out.clone();
        engine.on_print(move |text| out.lock().unwrap().commands.push(ScriptCommand::Message(text.to_string())));
    }

    // --- queries ------------------------------------------------------------
    let w = world.clone();
    engine.register_fn("unit_hp", move |id: INT| -> INT { w.units.iter().find(|u| u.id as INT == id).map_or(0, |u| u.hp as INT) });
    let w = world.clone();
    engine.register_fn("unit_alive", move |id: INT| w.units.iter().any(|u| u.id as INT == id && u.alive));
    let w = world.clone();
    engine.register_fn("unit_name", move |id: INT| -> String {
        w.units.iter().find(|u| u.id as INT == id).map_or(String::new(), |u| u.name.clone())
    });
    let w = world.clone();
    engine.register_fn("unit_team", move |id: INT| -> String {
        w.units.iter().find(|u| u.id as INT == id).map_or(String::new(), |u| team_name(u.team).to_string())
    });
    let w = world.clone();
    engine.register_fn("unit_x", move |id: INT| -> INT { w.units.iter().find(|u| u.id as INT == id).map_or(-1, |u| u.pos.x as INT) });
    let w = world.clone();
    engine.register_fn("unit_y", move |id: INT| -> INT {
        w.units.iter().find(|u| u.id as INT == id).map_or(-1, |u| (h - 1 - u.pos.y) as INT)
    });
    let w = world.clone();
    engine.register_fn("unit_by_tag", move |tag: &str| -> INT {
        w.units.iter().find(|u| u.tag.as_deref() == Some(tag)).map_or(-1, |u| u.id as INT)
    });
    let w = world.clone();
    engine.register_fn("unit_at", move |x: INT, y: INT| -> INT {
        let p = grid(x, y);
        w.units.iter().find(|u| u.alive && u.pos == p).map_or(-1, |u| u.id as INT)
    });
    let w = world.clone();
    engine.register_fn("alive_count", move |team: &str| -> INT {
        let t = parse_team(team);
        w.units.iter().filter(|u| u.alive && Some(u.team) == t).count() as INT
    });
    let w = world.clone();
    engine.register_fn("prop_alive", move |tag: &str| w.props.iter().any(|p| p.tag == tag && p.alive));
    let w = world.clone();
    engine.register_fn("height", move |x: INT, y: INT| -> INT { w.map.as_ref().map_or(0, |m| m.elevation(grid(x, y)) as INT) });
    let w = world.clone();
    engine.register_fn("terrain", move |x: INT, y: INT| -> String {
        let t = w.map.as_ref().map_or(Terrain::Void, |m| m.terrain(grid(x, y)));
        match t {
            Terrain::Floor => "floor",
            Terrain::LowCover => "half",
            Terrain::HighCover => "full",
            Terrain::Void => "void",
        }
        .to_string()
    });
}

#[cfg(all(test, feature = "scripting"))]
mod tests {
    use super::*;

    fn world() -> ScriptWorld {
        ScriptWorld {
            round: 3,
            active_team: Some(Team::Player),
            units: vec![WorldUnit {
                id: 4,
                team: Team::Enemy,
                name: "Boss".into(),
                tag: Some("boss".into()),
                pos: GridPos::new(1, 0),
                hp: 6,
                max_hp: 6,
                alive: true,
            }],
            props: vec![],
            map: Some(Map::new(5, 5)),
            flags: [("seen".to_string(), 1)].into(),
        }
    }

    #[test]
    fn commands_and_queries() {
        let code = r#"
            fn shout(s) { message("!" + s) }
            if round == 3 && team == "player" { shout("reinforcements") }
            let boss = unit_by_tag("boss");
            damage(boss, unit_hp(boss) - 1);
            set_flag("seen", flag("seen") + 1);
            if flag("seen") == 2 { win(); }
            explode(x, y, 1.5, 2);
            // y counts rows from the top: grid (1, 0) is row 4 of 5.
            if unit_y(boss) == 4 { print("top-down y"); }
        "#;
        let vars = ScriptVars { pos: Some(GridPos::new(2, 4)), ..Default::default() };
        let cmds = run(code, "", &vars, &world()).unwrap();
        assert_eq!(cmds[0], ScriptCommand::Message("!reinforcements".into()));
        assert_eq!(cmds[1], ScriptCommand::Damage { unit: 4, amount: 5 });
        assert_eq!(cmds[2], ScriptCommand::SetFlag { name: "seen".into(), value: 2 });
        assert_eq!(cmds[3], ScriptCommand::Win);
        assert_eq!(cmds[4], ScriptCommand::Explode { pos: GridPos::new(2, 4), radius: 1.5, damage: 2 });
        assert_eq!(cmds[5], ScriptCommand::Message("top-down y".into()));
    }

    #[test]
    fn errors_and_limits_are_reported_not_fatal() {
        assert!(run("this is not rhai", "", &ScriptVars::default(), &world()).is_err());
        let e = run("spawn_unit(\"martians\", \"x\", \"y\", 0, 0)", "", &ScriptVars::default(), &world());
        assert!(format!("{e:?}").contains("unknown team"), "{e:?}");
        let e = run("loop { }", "", &ScriptVars::default(), &world()).unwrap_err();
        assert!(e.to_lowercase().contains("operations"), "{e}");
    }
}

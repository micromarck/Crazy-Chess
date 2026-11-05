//! Builds the [`Ruleset`] the game plays with, from modules.
//!
//! Load order (later modules override earlier content with the same id):
//! 1. **Rust modules** listed in `rust_modules()`.
//! 2. **Bundled data modules** in [`BUNDLED_DATA_MODULES`] — RON files
//!    compiled into the binary so they work on every platform.
//! 3. **User data modules** (desktop only): every `*.ron` in [`USER_MODULE_DIR`],
//!    in file-name order. The in-game editor (F1) saves here too.
//!
//! ### Adding a module
//! * Rust: implement `GameModule` (copy `tactics_core/src/modules/grenadier.rs`)
//!   and add it to `rust_modules()`.
//! * Data: write a `.ron` file (copy `assets/modules/marksman.ron`) and either
//!   add it to [`BUNDLED_DATA_MODULES`] or drop it into `modules/` on desktop.

use tactics_core::content::{GameModule, Ruleset};
use tactics_core::modules;

/// Rust modules, in load order. Add yours here.
fn rust_modules() -> Vec<Box<dyn GameModule>> {
    vec![Box::new(modules::BaseModule), Box::new(modules::GrenadierModule), Box::new(modules::ZombieModule)]
}

/// RON modules embedded at compile time: `(file name, contents)`.
pub const BUNDLED_DATA_MODULES: &[(&str, &str)] = &[
    ("marksman.ron", include_str!("../assets/modules/marksman.ron")),
    ("drones.ron", include_str!("../assets/modules/drones.ron")),
    ("creatures.ron", include_str!("../assets/modules/creatures.ron")),
    ("zombies.ron", include_str!("../assets/modules/zombies.ron")),
    ("mercenaries.ron", include_str!("../assets/modules/mercenaries.ron")),
    ("corrupted_robots.ron", include_str!("../assets/modules/corrupted_robots.ron")),
    ("facility_ai.ron", include_str!("../assets/modules/facility_ai.ron")),
    ("arsenal.ron", include_str!("../assets/modules/arsenal.ron")),
    ("robot_frames.ron", include_str!("../assets/modules/robot_frames.ron")),
];

/// Folder (relative to the working directory) scanned for user modules on desktop.
pub const USER_MODULE_DIR: &str = "modules";

/// File the in-game editor writes. `zz_` makes it load after other user modules.
pub const EDITOR_SAVE_FILE: &str = "zz_editor_tuning.ron";

/// Builds the ruleset. Returns it together with human-readable load messages
/// (shown in the combat log and printed to the console).
pub fn build_ruleset() -> (Ruleset, Vec<String>) {
    let mut rules = Ruleset::new();
    let mut log = Vec::new();

    for module in rust_modules() {
        rules.add_module(module.as_ref());
    }

    for (name, text) in BUNDLED_DATA_MODULES {
        load_data(&mut rules, name, text, &mut log);
    }

    for (name, text) in user_module_files() {
        load_data(&mut rules, &name, &text, &mut log);
    }

    for problem in rules.validate() {
        log.push(format!("Module problem: {problem}"));
    }
    let names: Vec<_> = rules.modules().iter().map(|m| m.name.as_str()).collect();
    log.insert(0, format!("Modules: {}", names.join(", ")));
    (rules, log)
}

fn load_data(rules: &mut Ruleset, name: &str, text: &str, log: &mut Vec<String>) {
    match tactics_core::content::data::DataModule::from_ron(text) {
        Ok(module) => rules.add_module(&module),
        Err(e) => log.push(format!("Failed to load {name}: {e}")),
    }
}

/// Reads `modules/*.ron` (desktop only; the web and Android have no such folder).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn user_module_files() -> Vec<(String, String)> {
    let Ok(dir) = std::fs::read_dir(USER_MODULE_DIR) else {
        return Vec::new();
    };
    let mut files: Vec<_> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ron"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            Some((p.display().to_string(), text))
        })
        .collect()
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn user_module_files() -> Vec<(String, String)> {
    Vec::new()
}

/// Writes RON text to the editor save file. Returns the path or an error.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn save_user_module(text: &str) -> Result<String, String> {
    let dir = std::path::Path::new(USER_MODULE_DIR);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(EDITOR_SAVE_FILE);
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// No file system on web/Android: the caller logs the RON so it can be copied.
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub fn save_user_module(_text: &str) -> Result<String, String> {
    Err("saving files isn't available on this platform; the module was printed to the log".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tactics_core::prelude::*;

    fn script_errors(events: &[GameEvent]) -> Vec<String> {
        events.iter().filter_map(|e| if let GameEvent::ScriptError { message } = e { Some(message.clone()) } else { None }).collect()
    }

    /// The module enemies' class scripts compile and run (dry run of every
    /// hook), and the robots really do break down over time.
    #[test]
    fn corrupted_robots_break_down_over_time() {
        let (rules, _) = build_ruleset();
        let mut b = Battle::new(Map::from_ascii(&"....................\n".repeat(8)), rules, 7);
        let soldier = b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 0)).unwrap();
        let sentry = b.spawn("corrupted_sentry", "Sentry", Team::Enemy, GridPos::new(19, 7)).unwrap();
        let loader = b.spawn("corrupted_loader", "Loader", Team::Enemy, GridPos::new(18, 7)).unwrap();
        let drone = b.spawn("corrupted_drone", "Drone", Team::Enemy, GridPos::new(19, 5)).unwrap();
        for m in ["merc_officer", "merc_rifleman", "merc_gunner", "merc_breacher", "merc_marksman", "merc_medic"] {
            b.spawn(m, m, Team::Enemy, GridPos::new(10 + b.units().len() as i32, 2)).unwrap();
        }
        assert!(b.dry_run_scripts().is_empty(), "{:?}", b.dry_run_scripts());
        let start = |id| b.unit(id).unwrap().stats.defense;
        let (s0, d0) = (start(sentry), start(drone));
        let mut events = Vec::new();
        for _ in 0..6 {
            events.extend(b.apply(Action::EndTurn).unwrap()); // enemy turn starts: decay
            events.extend(b.apply(Action::EndTurn).unwrap());
            if b.is_over() {
                break;
            }
        }
        assert!(script_errors(&events).is_empty(), "{:?}", script_errors(&events));
        let alive_def = |id| b.unit(id).filter(|u| u.is_alive()).map(|u| u.stats.defense);
        if let Some(d) = alive_def(sentry) {
            assert!(d < s0, "sentry shed armour: {s0} -> {d}");
        }
        if let Some(d) = alive_def(drone) {
            assert!(d < d0, "drone shed armour: {d0} -> {d}");
        }
        assert!(b.flags().get(&format!("wear_{}", sentry.0)).copied().unwrap_or(0) >= 1, "wear builds up");
        let callouts: Vec<&str> =
            events.iter().filter_map(|e| if let GameEvent::Callout { text, .. } = e { Some(text.as_str()) } else { None }).collect();
        assert!(callouts.contains(&"PLATING FAILING"), "{callouts:?}");
        assert!(
            callouts.iter().any(|c| ["SYSTEM FREEZE", "SHORT CIRCUIT", "TARGETING GLITCH", "SERVO FAILURE"].contains(c)),
            "worn machines malfunction: {callouts:?}"
        );
        let _ = (soldier, loader);
    }

    #[test]
    fn loader_overdrives_and_officer_death_shakes_the_mercs() {
        let (rules, _) = build_ruleset();
        let mut b = Battle::new(Map::from_ascii(&"............\n".repeat(4)), rules, 3);
        b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 0)).unwrap();
        let loader = b.spawn("corrupted_loader", "Loader", Team::Enemy, GridPos::new(11, 3)).unwrap();
        let officer = b.spawn("merc_officer", "Officer", Team::Enemy, GridPos::new(6, 1)).unwrap();
        let grunt = b.spawn("merc_rifleman", "Grunt", Team::Enemy, GridPos::new(7, 1)).unwrap();
        let aim = b.unit(loader).unwrap().stats.aim;
        let mut events = Vec::new();
        b.apply_script_command(tactics_core::script::ScriptCommand::Damage { unit: loader.0, amount: 8 }, &mut events);
        let l = b.unit(loader).unwrap();
        assert!(l.is_alive() && l.stats.aim > aim && l.stats.defense == 0, "overdrive: aim {} def {}", l.stats.aim, l.stats.defense);
        let fear = b.unit(grunt).unwrap().fear;
        b.apply_script_command(tactics_core::script::ScriptCommand::Kill { unit: officer.0 }, &mut events);
        assert!(b.unit(grunt).unwrap().fear > fear, "the line wavers");
        assert!(script_errors(&events).is_empty(), "{:?}", script_errors(&events));
    }

    /// Robot frames: their own arsenal only (and soldiers can't take it),
    /// default loadouts spawn, heat rules come from the class, and the
    /// missile pod heats the frame. The Arsenal's lasers and heavy weapons
    /// go to the right soldiers.
    #[test]
    fn robot_frames_and_arsenal() {
        use tactics_core::content::Loadout;
        let (rules, _) = build_ruleset();
        let gear = |w: &str| Loadout { weapon: Some(w.into()), ..Default::default() };
        // Frames: own weapons only.
        assert!(rules.check_gear("frame_light", &gear("pulse_lasers")).is_empty());
        assert!(rules.check_gear("frame_light", &gear("needle_rail")).is_empty());
        assert!(!rules.check_gear("frame_light", &gear("gatling_laser")).is_empty(), "heavy gun on a light frame");
        assert!(!rules.check_gear("frame_light", &gear("rifle")).is_empty(), "soldiers' guns are off limits");
        assert!(!rules.check_gear("frame_heavy", &gear("laser_rifle")).is_empty());
        assert!(rules.check_gear("frame_heavy", &gear("plasma_cannon")).is_empty());
        assert!(!rules.check_gear("rifleman", &gear("pulse_lasers")).is_empty(), "and soldiers can't take theirs");
        // Soldiers: lasers and heavy weapons.
        assert!(rules.check_gear("rifleman", &gear("laser_rifle")).is_empty());
        assert!(rules.check_gear("sniper", &gear("laser_lance")).is_empty());
        assert!(!rules.check_gear("rifleman", &gear("laser_lance")).is_empty());
        assert!(rules.check_gear("heavy", &gear("minigun")).is_empty());
        assert!(rules.check_gear("grenadier", &gear("autocannon")).is_empty());
        assert!(!rules.check_gear("sniper", &gear("minigun")).is_empty());

        let mut b = Battle::new(Map::from_ascii(&"....................\n".repeat(8)), rules, 5);
        let light = b.spawn("frame_light", "S-07", Team::Player, GridPos::new(0, 0)).unwrap();
        let heavy = b.spawn("frame_heavy", "B-12", Team::Player, GridPos::new(1, 0)).unwrap();
        let gunner = b.spawn("heavy", "Gunner", Team::Player, GridPos::new(2, 0)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(10, 4)).unwrap();
        b.spawn("sectoid", "E2", Team::Enemy, GridPos::new(19, 7)).unwrap();
        let u = b.unit(heavy).unwrap();
        assert!(u.heat.is_some() && u.heat() == 0);
        assert_eq!(u.weapon.id, "gatling_laser");
        assert!(u.has_ability(&"missile_salvo".into()) && u.has_ability(&"coolant_flush".into()) && u.has_ability(&"shutdown".into()));
        assert!(b.unit(light).unwrap().has_ability(&"coolant_flush".into()));
        assert_eq!(b.unit(gunner).unwrap().weapon.id, "minigun");
        assert!(b.unit(gunner).unwrap().has_ability(&"fire_rocket".into()));
        assert!(b.unit(gunner).unwrap().heat.is_none());
        // Missile salvo: +8 heat; the gatling laser: +6.
        let target = b.unit(e).unwrap().pos;
        assert_eq!(b.heat_cost(&Action::ability(heavy, "missile_salvo", Target::Tile(target))), 8);
        assert_eq!(b.heat_cost(&Action::ability(heavy, "shoot", Target::Unit(e))), 6);
        b.apply(Action::ability(heavy, "missile_salvo", Target::Tile(target))).unwrap();
        assert_eq!(b.unit(heavy).unwrap().heat(), 8);
    }

    /// Frames are too big for low cover: a crate in front of a frame gives
    /// it nothing, the same crate in front of a soldier does; a wall still
    /// protects both.
    #[test]
    fn frames_get_no_half_cover() {
        let (rules, _) = build_ruleset();
        let art = "
            E.....
            ......
            ......
            .h.H..
            .F.S..
        ";
        let mut b = Battle::new(Map::from_ascii(art), rules, 1);
        let f = b.spawn("frame_light", "S-07", Team::Player, Map::find_markers(art, 'F')[0]).unwrap();
        let s = b.spawn("rifleman", "Vance", Team::Player, Map::find_markers(art, 'S')[0]).unwrap();
        let e = b.spawn("merc_rifleman", "Merc", Team::Enemy, Map::find_markers(art, 'E')[0]).unwrap();
        let (fu, su, eu) = (b.unit(f).unwrap(), b.unit(s).unwrap(), b.unit(e).unwrap());
        let profile = b.ability(&"shoot".into()).unwrap().effect.attack_profile().unwrap().clone();
        let at_frame = tactics_core::combat::preview_attack(&b.rules().combat, b.map(), eu.pos, eu, fu, &profile);
        assert!(!at_frame.breakdown.iter().any(|(l, _)| l.contains("cover")), "no half cover for frames: {:?}", at_frame.breakdown);
        assert_eq!(at_frame.cover_block, 0);
        // The crate still covers a soldier.
        let mut soldier = su.clone();
        soldier.pos = fu.pos;
        let at_soldier = tactics_core::combat::preview_attack(&b.rules().combat, b.map(), eu.pos, eu, &soldier, &profile);
        assert!(at_soldier.hit_chance < at_frame.hit_chance, "{} vs {}", at_soldier.hit_chance, at_frame.hit_chance);
        // Full cover still protects a frame.
        let mut frame = fu.clone();
        frame.pos = su.pos;
        let behind_wall = tactics_core::combat::preview_attack(&b.rules().combat, b.map(), eu.pos, eu, &frame, &profile);
        assert!(behind_wall.hit_chance < at_frame.hit_chance);
    }

    /// AI vs AI with robot frames in the squad: only legal actions, the AI
    /// never overheats its own frames, and the frames get hot and cool off.
    #[test]
    fn robot_frames_fight_without_cooking_themselves() {
        use tactics_core::ai::{self, AiConfig};
        let (rules, _) = build_ruleset();
        let art = "
            ....................
            ....H.....hh....H...
            ....H...........H...
            ..hh......H.....hh..
            ..........H.........
            ....................
        ";
        let mut seen_hot = 0;
        let mut overheated = 0;
        for seed in 0..6 {
            let mut b = Battle::new(Map::from_ascii(art), rules.clone(), seed);
            b.spawn("frame_light", "S-07", Team::Player, GridPos::new(0, 0)).unwrap();
            b.spawn("frame_heavy", "B-12", Team::Player, GridPos::new(1, 0)).unwrap();
            b.spawn("frame_heavy", "B-30", Team::Player, GridPos::new(0, 1)).unwrap();
            b.spawn("heavy", "Gunner", Team::Player, GridPos::new(2, 0)).unwrap();
            let mercs = ["merc_gunner", "merc_rifleman", "merc_marksman", "merc_breacher", "merc_rifleman", "merc_officer"];
            for (i, class) in mercs.iter().enumerate() {
                b.spawn(class, format!("E{i}"), Team::Enemy, GridPos::new(19 - (i as i32 % 3), 3 + (i as i32 / 3) * 2)).unwrap();
            }
            let config = AiConfig::default();
            for _ in 0..800 {
                if b.is_over() || b.round() > 25 {
                    break;
                }
                let action = ai::next_team_action(&b, &config);
                let ev = b.apply(action.clone()).unwrap_or_else(|e| panic!("illegal {action:?}: {e}"));
                overheated += ev.iter().filter(|e| matches!(e, GameEvent::Callout { text, .. } if text == "OVERHEATED")).count();
                seen_hot += ev.iter().filter(|e| matches!(e, GameEvent::HeatChanged { delta, .. } if *delta > 0)).count();
            }
        }
        assert_eq!(overheated, 0, "the AI cooked its own frame");
        assert!(seen_hot > 10, "frames heat up as they fight ({seen_hot})");
    }

    #[test]
    fn bundled_modules_load_cleanly() {
        let (rules, log) = build_ruleset();
        assert!(log.iter().all(|l| !l.starts_with("Failed") && !l.starts_with("Module problem")), "{log:?}");
        assert!(rules.ability(&"aimed_shot".into()).is_some());
        assert!(rules.class("sectoid").unwrap().abilities.contains(&"psi_bolt".into()));
        assert_eq!(rules.class("drone").unwrap().model, "drone");
        assert!(rules.model("drone").unwrap().animations.contains_key("death"));
        // The sectoid skeleton sits on top of the humanoid.
        let sk = rules.resolve_skeleton("sectoid").unwrap();
        assert!(sk.index_of("antenna_l").is_some() && sk.index_of("thigh_l").is_some());
        // Zombies: class from Rust, look and moves from data.
        assert_eq!(rules.class("zombie").unwrap().model, "zombie");
        let zombie = rules.resolve_skeleton("zombie").unwrap();
        assert!(zombie.index_of("jaw").is_some());
        let za = rules.skeleton_animations("zombie");
        assert_eq!(za["walk"].skeleton, "zombie");
        assert_eq!(za["bite"].skeleton, "zombie");
        assert_eq!(za["death"].skeleton, "humanoid", "inherited");
        let anims = rules.skeleton_animations("sectoid");
        assert_eq!(anims["cast"].skeleton, "sectoid");
        assert_eq!(anims["walk"].skeleton, "humanoid");
        assert_eq!(rules.model("alien").unwrap().skeleton.as_deref(), Some("sectoid"));
    }
}

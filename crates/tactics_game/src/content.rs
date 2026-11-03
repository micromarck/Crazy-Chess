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

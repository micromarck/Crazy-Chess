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

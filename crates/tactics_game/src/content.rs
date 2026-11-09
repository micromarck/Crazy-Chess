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
    ("dinosaurs.ron", include_str!("../assets/modules/dinosaurs.ron")),
    ("symbiotes.ron", include_str!("../assets/modules/symbiotes.ron")),
    ("augments.ron", include_str!("../assets/modules/augments.ron")),
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
    fn augmented_soldiers_trade_strengths_and_the_wild_augments_work() {
        use tactics_core::content::Loadout;
        let (rules, _) = build_ruleset();
        let aug = |list: &[&str]| Loadout { augments: list.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        // Gear rules: augments only for the Augmented, one of each, three at most.
        assert!(!rules.check_gear("rifleman", &aug(&["aug_targeting_eye"])).is_empty(), "riflemen can't be augmented");
        assert!(!rules.check_gear("augmented", &aug(&["aug_targeting_eye", "aug_targeting_eye"])).is_empty());
        assert!(!rules.check_gear("augmented", &aug(&["aug_targeting_eye", "aug_bulk_grafts", "aug_pyre_core", "aug_regenerator"])).is_empty());
        let lasers = Loadout { weapon: Some("pulse_lasers".into()), ..Default::default() };
        assert!(!rules.check_gear("augmented", &lasers).is_empty(), "no Strider weapons without the heat frame");
        assert!(rules.check_gear("augmented", &Loadout { augments: vec!["aug_heat_frame".into()], ..lasers }).is_empty());

        let mut b = Battle::new(Map::from_ascii(&"............\n".repeat(5)), rules, 3);
        let spawn = |b: &mut Battle, preset: &str, x: i32| {
            b.spawn_equipped("augmented", preset, Team::Player, GridPos::new(x, 0), Some(preset), &Loadout::default()).unwrap()
        };
        let plain = b.spawn_equipped("augmented", "Plain", Team::Player, GridPos::new(0, 4), None, &aug(&[""])).unwrap();
        let turret = spawn(&mut b, "aug_turret", 0);
        let brute = spawn(&mut b, "aug_berserker", 3);
        let frame = spawn(&mut b, "aug_heatframe", 6);
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(11, 4)).unwrap();
        let (p, t, br, f) = (b.unit(plain).unwrap(), b.unit(turret).unwrap(), b.unit(brute).unwrap(), b.unit(frame).unwrap());
        assert!(t.stats.aim >= p.stats.aim + 20 && t.stats.mobility < p.stats.mobility, "aims, can't move");
        assert!(t.fearless, "the neural dampener");
        assert!(br.stats.max_hp > p.stats.max_hp && br.stats.defense < p.stats.defense, "tough, easy to hit");
        assert!(br.rage_rules.is_some(), "the berserker gland");
        assert!(f.heat.is_some() && f.weapon.id == "pulse_lasers" && f.has_ability(&"shutdown".into()), "a heat-run frame with Strider weapons");
        // The pyre core: dying sets the ground alight. (An enemy grenadier's
        // frag finishes the berserker off: the blast never misses.)
        let at = b.unit(brute).unwrap().pos;
        b.debug_set_hp(brute, 1);
        let g = b.spawn("grenadier", "G", Team::Enemy, GridPos::new(11, 0)).unwrap();
        while b.active_team() == Team::Player {
            b.apply(Action::EndTurn).unwrap();
        }
        let events = b.apply(Action::ability(g, "frag_grenade", Target::Tile(at))).unwrap();
        assert!(!b.unit(brute).unwrap().is_alive());
        assert!(b.on_fire(at), "the pyre core went up: {events:?}");
    }

    #[test]
    fn boost_costs_six_heat_and_needs_a_turn_to_recharge() {
        let (rules, _) = build_ruleset();
        let mut b = Battle::new(Map::from_ascii(&"....................\n".repeat(5)), rules, 3);
        let gear = tactics_core::content::Loadout { items: vec!["boost_thrusters".into()], ..Default::default() };
        let f = b.spawn_equipped("frame_heavy", "Bastion", Team::Player, GridPos::new(0, 2), None, &gear).unwrap();
        b.spawn("sectoid", "E", Team::Enemy, GridPos::new(19, 4)).unwrap();
        let boost = |to: i32| Action::ability(f, "boost", Target::Tile(GridPos::new(to, 2)));
        let heat = b.unit(f).unwrap().heat();
        b.apply(boost(6)).unwrap();
        assert_eq!(b.unit(f).unwrap().heat() - heat, 6, "6 heat, nothing else");
        assert_eq!(b.unit(f).unwrap().pos, GridPos::new(6, 2), "the same reach as before (3 moves)");
        while b.active_team() == Team::Player {
            b.apply(Action::EndTurn).unwrap();
        }
        b.apply(Action::EndTurn).unwrap();
        assert!(b.validate(&boost(10)).is_err(), "still recharging the next turn");
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
        assert!(b.validate(&boost(10)).is_ok(), "ready again the turn after");
    }

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
        // Jump packs for soldiers, boost thrusters for frames, not the other way round.
        let item = |i: &str| Loadout { items: vec![i.into()], ..Default::default() };
        assert!(rules.check_gear("rifleman", &item("jump_pack")).is_empty());
        assert!(!rules.check_gear("frame_light", &item("jump_pack")).is_empty());
        assert!(rules.check_gear("frame_heavy", &item("boost_thrusters")).is_empty());
        assert!(!rules.check_gear("assault", &item("boost_thrusters")).is_empty());

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
        assert!(b.unit(light).unwrap().has_ability(&"overclock".into()) && b.unit(light).unwrap().has_ability(&"boost".into()));
        assert!(!b.unit(heavy).unwrap().has_ability(&"overclock".into()), "only the small frames overclock");
        // Particle grenades: short throw, delayed, square.
        let pg = b.rules().ability(&"particle_grenade".into()).unwrap();
        assert_eq!(pg.range, tactics_core::content::AbilityRange::Fixed(5.0));
        let p = pg.effect.attack_profile().unwrap();
        assert!(p.square && p.fuse == 2 && p.area_radius == 2.0);
        assert_eq!(b.unit(gunner).unwrap().weapon.id, "minigun");
        assert!(b.unit(gunner).unwrap().has_ability(&"fire_rocket".into()));
        assert!(b.unit(gunner).unwrap().heat.is_none());
        // Idle: no heat, and the frame's turn is over.
        assert_eq!(b.heat_cost(&Action::ability(light, "idle", Target::None)), 0);
        b.apply(Action::ability(light, "idle", Target::None)).unwrap();
        assert_eq!(b.unit(light).unwrap().action_points, 0);
        assert_eq!(b.unit(light).unwrap().heat(), 0);
        // Missile salvo: +8 heat; the gatling laser: +6.
        let target = b.unit(e).unwrap().pos;
        assert_eq!(b.heat_cost(&Action::ability(heavy, "missile_salvo", Target::Tile(target))), 8);
        assert_eq!(b.heat_cost(&Action::ability(heavy, "shoot", Target::Unit(e))), 6);
        b.apply(Action::ability(heavy, "missile_salvo", Target::Tile(target))).unwrap();
        assert_eq!(b.unit(heavy).unwrap().heat(), 8);
    }

    /// Fire and rockets: the flamethrower burns a cone and leaves it
    /// ablaze, the rocket launcher fires straight, and the AI uses them in
    /// a fight without errors.
    #[test]
    fn flamethrowers_and_rockets() {
        use tactics_core::ai::{self, AiConfig};
        use tactics_core::content::Loadout;
        let (rules, _) = build_ruleset();
        let pyro = Loadout { items: vec!["flamethrower_pack".into(), "incendiary".into()], ..Default::default() };
        assert!(rules.check_gear("heavy", &pyro).is_empty());
        // The flamethrower as a main weapon.
        let flamer = Loadout { weapon: Some("flamer".into()), ..Default::default() };
        assert!(rules.check_gear("assault", &flamer).is_empty() && rules.check_gear("heavy", &flamer).is_empty());
        assert!(!rules.check_gear("sniper", &flamer).is_empty());
        assert!(rules.weapon("flamer").unwrap().cone > 0.0);
        assert!(rules.check_gear("heavy", &rules.gear_for("heavy", Some("heavy_inferno"), &Loadout::default())).is_empty());
        assert!(rules.ability(&"fire_rocket".into()).unwrap().effect.attack_profile().unwrap().direct);
        let art = "
            ..............
            ..h......h....
            ..............
            ....H.....H...
            ..............
        ";
        let mut fires = 0;
        for seed in 0..4 {
            let mut b = Battle::new(Map::from_ascii(art), rules.clone(), seed);
            b.spawn_equipped("heavy", "Pyro", Team::Player, GridPos::new(0, 2), Some("heavy_flame"), &Loadout::default()).unwrap();
            b.spawn_equipped("heavy", "Rocketeer", Team::Player, GridPos::new(0, 1), Some("heavy_std"), &Loadout::default()).unwrap();
            for (i, x) in [7, 8, 9].iter().enumerate() {
                b.spawn("merc_rifleman", format!("M{i}"), Team::Enemy, GridPos::new(*x, 2)).unwrap();
            }
            let config = AiConfig::default();
            for _ in 0..400 {
                if b.is_over() || b.round() > 12 {
                    break;
                }
                let action = ai::next_team_action(&b, &config);
                let ev = b.apply(action.clone()).unwrap_or_else(|e| panic!("illegal {action:?}: {e}"));
                assert!(script_errors(&ev).is_empty(), "{:?}", script_errors(&ev));
                fires += ev.iter().filter(|e| matches!(e, GameEvent::FireStarted { .. })).count();
            }
        }
        let _ = fires;

        // Direct use: the flamethrower sets the cone ablaze; the rocket
        // flies straight into the first merc in its way.
        let mut b = Battle::new(Map::from_ascii(art), rules.clone(), 1);
        let pyro = b.spawn_equipped("heavy", "Pyro", Team::Player, GridPos::new(0, 2), Some("heavy_flame"), &Loadout::default()).unwrap();
        let rocketeer = b.spawn_equipped("heavy", "Rocketeer", Team::Player, GridPos::new(0, 0), Some("heavy_std"), &Loadout::default()).unwrap();
        let near = b.spawn("merc_rifleman", "Near", Team::Enemy, GridPos::new(4, 2)).unwrap();
        let blocker = b.spawn("merc_rifleman", "Blocker", Team::Enemy, GridPos::new(6, 0)).unwrap();
        b.spawn("merc_rifleman", "Far", Team::Enemy, GridPos::new(12, 0)).unwrap();
        let hp = b.unit(near).unwrap().hp;
        let ev = b.apply(Action::ability(pyro, "flamethrower", Target::Tile(GridPos::new(4, 2)))).unwrap();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::FireStarted { .. })));
        assert!(b.unit(near).unwrap().hp < hp, "burned");
        assert!(b.on_fire(GridPos::new(3, 2)));
        let ev = b.apply(Action::ability(rocketeer, "fire_rocket", Target::Tile(GridPos::new(12, 0)))).unwrap();
        let blocker_pos = GridPos::new(6, 0);
        assert!(ev.iter().any(|e| matches!(e, GameEvent::Explosion { center, .. } if *center == blocker_pos)), "the rocket hits the first merc");
        let _ = blocker;
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

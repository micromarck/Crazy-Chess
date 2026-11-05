//! Mission files from the assets folder.
//!
//! Everything a level needs lives in **one folder**: `assets/missions/`.
//!
//! ```text
//! assets/missions/
//! ├── index.xml               which missions exist (and which starts first)
//! ├── depot.mission.xml       one XML file per mission: grid, cut-outs,
//! └── rooftops.mission.xml    height map, units, props, zones, scripts
//! ```
//!
//! Files are read through Bevy's asset server (so this works on desktop, web
//! and Android), parsed by [`tactics_core::mission::parse_mission`] and listed
//! in the [`MissionCatalog`]. The `start` mission of the index (or the first
//! one) begins automatically; the HUD's mission list starts any other.
//! Parse errors are shown in the combat log with line/column.
//!
//! All battle (re)starts go through [`StartBattle`], handled by
//! [`start_battle`]: missions, procedural skirmishes and restarts alike.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use bevy::prelude::*;
use tactics_core::mission::{MissionDef, parse_index, parse_mission};
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::{grid_to_world, tile_top};
use crate::hud::{FloatingText, LogMessages};
use crate::playback::Playback;
use crate::state::{BattleReset, BattleState, PlayerCommand, Selection};

pub struct MissionsPlugin;

impl Plugin for MissionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<XmlText>()
            .register_asset_loader(XmlTextLoader)
            .add_message::<StartBattle>()
            .add_systems(Startup, load_index)
            .add_systems(Update, (track_loading, route_player_commands, start_battle).chain().in_set(FrameSet::Logic));
    }
}

/// Folder (under `assets/`) holding the missions.
pub const MISSION_DIR: &str = "missions";

/// A text file loaded through the asset server (`.xml`, `.txt`, `.character.ron`).
#[derive(Asset, TypePath, Debug)]
pub struct XmlText(pub String);

#[derive(Default, TypePath)]
struct XmlTextLoader;

impl AssetLoader for XmlTextLoader {
    type Asset = XmlText;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(&self, reader: &mut dyn Reader, _: &(), _: &mut LoadContext<'_>) -> Result<XmlText, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(XmlText(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn extensions(&self) -> &[&str] {
        // Mission XML, the characters index and character sidecars.
        &["xml", "txt", "character.ron"]
    }
}

/// One loaded mission.
#[derive(Debug, Clone)]
pub struct MissionEntry {
    pub file: String,
    pub def: MissionDef,
}

/// Loading state and the parsed missions.
#[derive(Resource, Default)]
pub struct MissionCatalog {
    index: Option<Handle<XmlText>>,
    files: Vec<(String, Handle<XmlText>)>,
    pub missions: Vec<MissionEntry>,
    pub start: Option<String>,
    /// All files processed (loaded or failed).
    pub ready: bool,
    started: bool,
}

impl MissionCatalog {
    /// The first mission has been started.
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn get(&self, id: &str) -> Option<&MissionEntry> {
        self.missions.iter().find(|m| m.def.id == id)
    }
}

/// Mission id used for generated missions.
pub const GENERATED: &str = "generated";

/// What to start.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum StartBattle {
    Mission(String),
    Skirmish,
    /// A generated mission; `None` = random theme.
    Generate(Option<tactics_core::levelgen::Theme>),
    /// The current mission again, or a new skirmish if none.
    Restart,
}

fn load_index(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(MissionCatalog {
        index: Some(assets.load(format!("{MISSION_DIR}/index.xml"))),
        ..default()
    });
}

/// Index → mission files → parsed definitions; auto-starts the first mission.
fn track_loading(
    assets: Res<AssetServer>,
    texts: Res<Assets<XmlText>>,
    mut catalog: ResMut<MissionCatalog>,
    mut log: ResMut<LogMessages>,
    mut start: MessageWriter<StartBattle>,
) {
    if catalog.ready {
        if !catalog.started {
            catalog.started = true;
            // `TBS_MISSION=<id>` (desktop) overrides the index's start mission,
            // handy for testing a level.
            #[cfg(not(target_arch = "wasm32"))]
            let wanted = std::env::var("TBS_MISSION").ok().or_else(|| catalog.start.clone());
            #[cfg(target_arch = "wasm32")]
            let wanted = catalog.start.clone();
            // `gen:<theme>` (or `gen:any`) starts a generated mission.
            if let Some(theme) = wanted.as_deref().and_then(|w| w.strip_prefix("gen:")) {
                start.write(StartBattle::Generate(tactics_core::levelgen::Theme::parse(theme)));
                return;
            }
            let first = wanted.filter(|s| catalog.get(s).is_some()).or_else(|| catalog.missions.first().map(|m| m.def.id.clone()));
            if let Some(id) = first {
                start.write(StartBattle::Mission(id));
            }
        }
        return;
    }

    // 1. The index.
    if let Some(index) = catalog.index.clone() {
        if let Some(text) = texts.get(&index) {
            match parse_index(&text.0) {
                Ok((files, start_id)) => {
                    catalog.files = files.into_iter().map(|f| (f.clone(), assets.load(format!("{MISSION_DIR}/{f}")))).collect();
                    catalog.start = start_id;
                }
                Err(e) => log.push(format!("missions/index.xml: {e}")),
            }
            catalog.index = None;
        } else if matches!(assets.load_state(&index), LoadState::Failed(_)) {
            log.push("No missions/index.xml found: playing the demo map.".into());
            catalog.index = None;
            catalog.ready = true;
        }
        return;
    }

    // 2. The mission files.
    let mut pending = false;
    let files = std::mem::take(&mut catalog.files);
    let mut keep = Vec::new();
    for (file, handle) in files {
        if let Some(text) = texts.get(&handle) {
            match parse_mission(&text.0) {
                Ok(def) => catalog.missions.push(MissionEntry { file, def }),
                Err(e) => log.push(format!("{file}: {e}")),
            }
        } else if matches!(assets.load_state(&handle), LoadState::Failed(_)) {
            log.push(format!("{file}: could not be loaded"));
        } else {
            pending = true;
            keep.push((file, handle));
        }
    }
    catalog.files = keep;
    if !pending {
        catalog.ready = true;
    }
}

/// Mission-related player commands → [`StartBattle`].
fn route_player_commands(mut commands: MessageReader<PlayerCommand>, mut start: MessageWriter<StartBattle>) {
    for c in commands.read() {
        match c {
            PlayerCommand::Restart => {
                start.write(StartBattle::Restart);
            }
            PlayerCommand::StartMission(id) => {
                start.write(StartBattle::Mission(id.clone()));
            }
            PlayerCommand::Skirmish => {
                start.write(StartBattle::Generate(None));
            }
            PlayerCommand::Generate(theme) => {
                start.write(StartBattle::Generate(tactics_core::levelgen::Theme::parse(theme)));
            }
            _ => {}
        }
    }
}

/// Replaces the battle and resets everything that depended on the old one.
#[allow(clippy::too_many_arguments)]
pub fn start_battle(
    mut requests: MessageReader<StartBattle>,
    mut reset: MessageWriter<BattleReset>,
    catalog: Res<MissionCatalog>,
    mut state: ResMut<BattleState>,
    mut playback: ResMut<Playback>,
    mut selection: ResMut<Selection>,
    mut log: ResMut<LogMessages>,
    mut rig: Single<&mut CameraRig>,
    floating: Query<Entity, With<FloatingText>>,
    mut flags: ResMut<crate::html_ui::UiFlags>,
    mut commands: Commands,
) {
    let Some(request) = requests.read().last().cloned() else { return };
    let replay = matches!(request, StartBattle::Restart) && state.mission.as_deref() == Some(GENERATED) && state.generated.is_some();
    let request = match request {
        StartBattle::Restart if replay => StartBattle::Restart,
        StartBattle::Restart => state.mission.clone().map_or(StartBattle::Generate(None), StartBattle::Mission),
        StartBattle::Skirmish => StartBattle::Generate(None),
        other => other,
    };
    let seed = state.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let rules = state.battle.rules().clone();

    let (mut battle, mission, mut lines) = match &request {
        StartBattle::Mission(id) => match catalog.get(id) {
            Some(entry) => {
                let (battle, warnings) = Battle::from_mission(rules, &entry.def, seed);
                let mut lines = vec![format!("Mission: {}", entry.def.name)];
                if !entry.def.description.is_empty() {
                    lines.push(entry.def.description.clone());
                }
                lines.extend(warnings.into_iter().map(|w| format!("{}: {w}", entry.file)));
                (battle, Some(id.clone()), lines)
            }
            None => {
                log.push(format!("Unknown mission '{id}'"));
                return;
            }
        },
        StartBattle::Generate(theme) => {
            use tactics_core::levelgen::{self, GenParams, Objective};
            let mut params = match theme {
                Some(t) => GenParams::new(*t, Objective::ALL[(seed % Objective::ALL.len() as u64) as usize], seed),
                None => GenParams::random(seed),
            };
            if params.theme != levelgen::Theme::Outbreak {
                params.enemy_classes = levelgen::enemy_classes(&rules);
            }
            let def = levelgen::generate(params);
            let (battle, warnings) = Battle::from_mission(rules, &def, seed);
            let mut lines = vec![format!("Generated mission: {}", def.name), def.description.clone()];
            lines.extend(warnings);
            state.generated = Some(def);
            (battle, Some(GENERATED.to_string()), lines)
        }
        StartBattle::Restart => {
            let def = state.generated.clone().expect("checked above");
            let (battle, _) = Battle::from_mission(rules, &def, seed);
            (battle, Some(GENERATED.to_string()), vec![format!("Generated mission: {} (again)", def.name), def.description.clone()])
        }
        StartBattle::Skirmish => (scenario::skirmish(&rules, seed), None, vec![format!("New skirmish (seed {seed:#x})")]),
    };
    let begin_events = battle.begin();

    state.battle = battle;
    state.stats = Default::default();
    state.seed = seed;
    state.mission = mission;
    state.revision += 1;
    playback.clear();
    playback.enqueue(begin_events);
    *selection = Selection::default();
    log.clear();
    for line in lines.drain(..) {
        log.push(line);
    }
    for e in &floating {
        commands.entity(e).despawn();
    }

    let map = state.battle.map();
    let far = grid_to_world(GridPos::new(map.width() - 1, map.height() - 1));
    rig.bounds = Rect::from_corners(Vec2::ZERO, Vec2::new(far.x, far.z));
    if let Some(u) = state.battle.living(Team::Player).next() {
        // Establishing shot: start high over the middle of the map and
        // glide down to the squad.
        let center = Vec3::new(far.x / 2.0, 0.0, far.z / 2.0);
        rig.focus = center;
        rig.focus_velocity = Vec3::ZERO;
        rig.distance = (map.width().max(map.height()) as f32 * 1.1).clamp(18.0, CameraRig::MAX_DISTANCE);
        rig.target_distance = rig.user_distance;
        rig.focus_at(tile_top(map, u.pos));
    }
    // Menus that led here (Missions, Loadout...) close with the new battle.
    for panel in ["missions", "loadout", "barracks", "settings", "help"] {
        if let Some(open) = flags.0.get_mut(panel) {
            *open = false;
        }
    }
    reset.write(BattleReset);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The faction missions (Mercenaries / Corrupted Robots modules): their
    /// scripts pass a dry run, and the objective, the evac point and every
    /// enemy can be reached on foot from the deploy point (doors opened,
    /// ladders climbed).
    #[test]
    fn faction_missions_are_reachable_and_scripted() {
        use tactics_core::grid::{Climb, Terrain};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let (rules, _) = crate::content::build_ruleset();
        for f in ["merc_ambush.mission.xml", "scrapyard.mission.xml", "salvage_run.mission.xml", "facility.mission.xml"] {
            let def = parse_mission(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap();
            let (b, warnings) = Battle::from_mission(rules.clone(), &def, 1);
            assert!(warnings.is_empty(), "{f}: {warnings:?}");
            assert!(b.dry_run_scripts().is_empty(), "{f}: {:?}", b.dry_run_scripts());
            let mut map = b.map().clone();
            for d in &def.doors {
                for t in &d.tiles {
                    map.set_terrain(*t, Terrain::Floor);
                }
            }
            let start = b.living(Team::Player).next().unwrap().pos;
            let reach = tactics_core::pathfinding::reachable(&map, start, u32::MAX / 4, Climb::default(), |_| true);
            for z in &def.zones {
                assert!(reach.contains(z.min), "{f}: zone '{}' unreachable", z.tag);
            }
            for e in b.living(Team::Enemy) {
                // Flyers can be up anywhere; everyone else must be on reachable ground.
                if e.stats.climb < 5 {
                    let near = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|(dx, dy)| reach.contains(tactics_core::grid::GridPos::new(e.pos.x + dx, e.pos.y + dy)));
                    assert!(near, "{f}: {} unreachable at {:?}", e.name, e.pos);
                }
            }
            let classes: std::collections::BTreeSet<&str> = b.living(Team::Enemy).map(|u| u.class.as_str()).collect();
            assert!(classes.iter().any(|c| ["merc_", "corrupted_", "fac_"].iter().any(|p| c.starts_with(p))), "{f}: {classes:?}");
        }
    }

    /// Ghost in the Machine: VESPER starts in its Prime chassis; destroying
    /// its host freezes every machine for 2 enemy turns, then it comes back
    /// in another machine; it seals doors near the squad, one at a time.
    #[test]
    fn facility_boss_jumps_freezes_and_locks_doors() {
        use tactics_core::script::ScriptCommand;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let text = std::fs::read_to_string(dir.join("facility.mission.xml")).unwrap();
        let def = parse_mission(&text).unwrap();
        let (rules, _) = crate::content::build_ruleset();
        let (mut b, warnings) = Battle::from_mission(rules.clone(), &def, 3);
        assert!(warnings.is_empty(), "{warnings:?}");
        let events = b.begin();
        assert!(!events.iter().any(|e| matches!(e, GameEvent::ScriptError { .. })), "{events:?}");
        let host = |b: &Battle| UnitId((b.flags().get("vesper_host").copied().unwrap_or(0) - 1) as u32);
        let prime = host(&b);
        assert_eq!(b.unit(prime).unwrap().class, "vesper_prime");
        assert_eq!(b.unit_mark(prime), Some("VESPER"));
        let enemies = b.living(Team::Enemy).count();

        // Destroy the host: everything else freezes for 2 turns.
        let mut ev = Vec::new();
        b.apply_script_command(ScriptCommand::Kill { unit: prime.0 }, &mut ev);
        assert!(!ev.iter().any(|e| matches!(e, GameEvent::ScriptError { .. })), "{ev:?}");
        assert_eq!(b.living(Team::Enemy).count(), enemies - 1);
        assert!(b.living(Team::Enemy).all(|u| u.status.stunned == 2), "all machines frozen");
        assert_eq!(b.flags().get("vesper_reboot"), Some(&3));
        for turn in 1..=3 {
            let mut ev = b.apply(Action::EndTurn).unwrap();
            assert_eq!(b.active_team(), Team::Enemy);
            if turn < 3 {
                assert!(b.living(Team::Enemy).all(|u| u.action_points == 0), "frozen on enemy turn {turn}");
                assert!(b.living(Team::Enemy).all(|u| b.unit_mark(u.id).is_none()), "no host while rebooting");
            } else {
                let h = host(&b);
                assert!(b.unit(h).is_some_and(|u| u.is_alive() && u.name.starts_with("VESPER // ")), "{:?}", b.unit(h));
                assert_eq!(b.unit_mark(h), Some("VESPER"));
                assert!(b.living(Team::Enemy).filter(|u| u.id != h).any(|u| u.action_points > 0), "machines run again");
            }
            ev.extend(b.apply(Action::EndTurn).unwrap());
            assert!(!ev.iter().any(|e| matches!(e, GameEvent::ScriptError { .. })), "{ev:?}");
        }

        // Doors: deploy the squad in the lobby; on the enemy turn VESPER
        // seals one door near them (only one at a time).
        let moved = text.replace(r#"squad="player" x="12" y="43""#, r#"squad="player" x="12" y="28""#);
        assert_ne!(moved, text);
        let (mut b, _) = Battle::from_mission(rules, &parse_mission(&moved).unwrap(), 3);
        b.begin();
        b.apply(Action::EndTurn).unwrap();
        let mut tags: Vec<&str> = b.props().iter().filter(|p| p.door.is_some_and(|d| d.locked)).map(|p| p.tag.as_str()).collect();
        tags.dedup();
        assert_eq!(tags.len(), 1, "one door sealed: {tags:?}");
        assert!(tags[0].starts_with("lobby_"), "{tags:?}");
        assert!(b.props().iter().filter(|p| p.tag == tags[0]).all(|p| p.door.is_some_and(|d| !d.open)));
    }

    /// Hold the Hill: zombies can climb to the plateau (up the ramps; the
    /// cliffs are too steep), the tide keeps 10-12 on the hill and never
    /// more than 30 alive, and the squad wins by being on the pad once the
    /// chopper lands (turn 12).
    #[test]
    fn hill_horde_tides_and_evac() {
        // The normal hill, and the extreme one (bigger, 2.5x the zombies).
        for (file, cap, builds_to, first_wave) in [("hill.mission.xml", 30, 20, 11), ("hill_extreme.mission.xml", 75, 50, 28)] {
            hill_check(file, cap, builds_to, first_wave);
        }
    }

    fn hill_check(file: &str, cap: usize, builds_to: usize, first_wave: usize) {
        use tactics_core::grid::Climb;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let def = parse_mission(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
        let (rules, _) = crate::content::build_ruleset();
        let (mut b, warnings) = Battle::from_mission(rules, &def, 5);
        assert_eq!(b.living(Team::Enemy).count(), first_wave, "{file}");
        assert!(warnings.is_empty(), "{warnings:?}");
        let ev = b.begin();
        assert!(!ev.iter().any(|e| matches!(e, GameEvent::ScriptError { .. })), "{ev:?}");
        // Zombies can get up there (climbing 1 level per step).
        let top = b.living(Team::Player).next().unwrap().pos;
        let up = tactics_core::pathfinding::reachable(b.map(), top, u32::MAX / 4, Climb { max_up: 9 }, |_| true);
        let zombie = b.living(Team::Enemy).next().unwrap();
        let climb = tactics_core::pathfinding::reachable(b.map(), zombie.pos, u32::MAX / 4, Climb { max_up: 1 }, |_| true);
        assert!(climb.contains(top), "zombies can reach the plateau");
        assert!(up.contains(zombie.pos));
        // A cliff: some slope tile next to a much lower one.
        let cliff = (0..b.map().width()).flat_map(|x| (0..b.map().height()).map(move |y| GridPos::new(x, y))).any(|p| {
            [(1, 0), (0, 1)].iter().any(|(dx, dy)| {
                let q = GridPos::new(p.x + dx, p.y + dy);
                b.map().in_bounds(q) && b.map().is_walkable(p) && b.map().is_walkable(q) && (b.map().elevation(p) - b.map().elevation(q)).abs() >= 2
            })
        });
        assert!(cliff, "the hill has cliffs");

        // Let the zombies come for a while (the squad just waits): the tide
        // tops them up and never passes 30.
        let mut most = 0;
        for _ in 0..8 {
            b.apply(Action::EndTurn).unwrap(); // zombies' turn: the tide spawns
            most = most.max(b.living(Team::Enemy).count());
            assert!(b.living(Team::Enemy).count() <= cap, "{file}");
            // Zombies don't act here (we only end turns), so nothing dies.
            b.apply(Action::EndTurn).unwrap();
        }
        assert!(most >= builds_to, "{file}: they build up: {most}");

        // The squad (still on the pad) wins when the chopper lands.
        while b.round() < 12 && !b.is_over() {
            b.apply(Action::EndTurn).unwrap();
        }
        assert_eq!(b.winner(), Some(Team::Player), "{file}: round {}", b.round());
    }

    /// Every mission in `assets/missions/index.xml` parses and builds a
    /// battle with the game's modules, without warnings or script errors.
    #[test]
    fn bundled_missions_are_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let index = std::fs::read_to_string(dir.join("index.xml")).unwrap();
        let (files, start) = parse_index(&index).unwrap();
        assert!(!files.is_empty());
        let (rules, log) = crate::content::build_ruleset();
        assert!(log.iter().all(|l| !l.starts_with("Failed") && !l.starts_with("Module problem")), "{log:?}");
        let mut ids = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(dir.join(&f)).unwrap();
            let def = parse_mission(&text).unwrap_or_else(|e| panic!("{f}: {e}"));
            let (mut battle, warnings) = Battle::from_mission(rules.clone(), &def, 1);
            assert!(warnings.is_empty(), "{f}: {warnings:?}");
            let events = battle.begin();
            for e in &events {
                assert!(!matches!(e, GameEvent::ScriptError { .. }), "{f}: {e:?}");
            }
            // The Character Lab is a test stage: one subject, no enemies.
            if def.id != crate::lab::LAB_MISSION {
                assert!(battle.living(Team::Player).count() >= 4, "{f}: squad deployed");
                assert!(battle.living(Team::Enemy).count() >= 3, "{f}: enemies placed");
            }
            ids.push(def.id);
        }
        assert!(start.is_none_or(|s| ids.contains(&s)));
    }

    /// AI vs AI on each bundled mission: legal actions only, scripts never
    /// error, and the battle makes progress.
    #[test]
    fn bundled_missions_play_out() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let (files, _) = parse_index(&std::fs::read_to_string(dir.join("index.xml")).unwrap()).unwrap();
        let (rules, _) = crate::content::build_ruleset();
        for f in files {
            let def = parse_mission(&std::fs::read_to_string(dir.join(&f)).unwrap()).unwrap();
            for seed in 0..3 {
                let (mut b, _) = Battle::from_mission(rules.clone(), &def, seed);
                let mut events = b.begin();
                let config = AiConfig::default();
                for _ in 0..1500 {
                    if b.is_over() || b.round() > 20 {
                        break;
                    }
                    let action = ai::next_team_action(&b, &config);
                    events.extend(b.apply(action.clone()).unwrap_or_else(|e| panic!("{f}: illegal {action:?}: {e}")));
                }
                let errors: Vec<_> = events.iter().filter(|e| matches!(e, GameEvent::ScriptError { .. })).collect();
                assert!(errors.is_empty(), "{f} seed {seed}: {errors:?}");
                assert!(b.round() > 1 || b.is_over(), "{f}: no progress");
            }
        }
    }

    /// The large missions really use their mechanics when played AI vs AI:
    /// doors open, pods wake, the infiltration alarm goes off.
    #[test]
    fn large_missions_exercise_their_mechanics() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MISSION_DIR);
        let (rules, _) = crate::content::build_ruleset();
        for (file, want_doors, want_alarm) in [
            ("depot.mission.xml", false, false),
            ("proving_grounds.mission.xml", true, false),
            ("holdout.mission.xml", false, false),
            ("complex.mission.xml", true, false),
            ("infiltration.mission.xml", true, true),
        ] {
            let def = parse_mission(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
            let (mut b, _) = Battle::from_mission(rules.clone(), &def, 0);
            assert_eq!(b.stealth().concealed, want_alarm, "{file}: concealment");
            let errors = b.dry_run_scripts();
            assert!(errors.is_empty(), "{file}: {errors:?}");
            let mut events = b.begin();
            for _ in 0..3000 {
                if b.is_over() || b.round() > 25 {
                    break;
                }
                let action = ai::next_team_action(&b, &AiConfig::default());
                events.extend(b.apply(action).unwrap());
            }
            let count = |f: fn(&GameEvent) -> bool| events.iter().filter(|e| f(e)).count();
            let doors = count(|e| matches!(e, GameEvent::DoorChanged { open: true, .. }));
            let alerted = count(|e| matches!(e, GameEvent::UnitAlerted { .. }));
            let alarms = count(|e| matches!(e, GameEvent::Alarm { .. }));
            eprintln!("{file}: round {} winner {:?} doors {doors} alerted {alerted} alarms {alarms} objective '{}'", b.round(), b.winner(), b.objective());
            assert!(!want_doors || doors > 0, "{file}: no door opened");
            assert!(!want_alarm || alarms == 1, "{file}: alarm count {alarms}");
            assert!(alerted > 0 || file.starts_with("holdout") || file.starts_with("depot"), "{file}: no pod woke");
            assert!(!b.objective().is_empty() || file.starts_with("depot"), "{file}: objective set");
        }
    }
}

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

/// A text file loaded through the asset server.
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
        &["xml"]
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
    pub fn get(&self, id: &str) -> Option<&MissionEntry> {
        self.missions.iter().find(|m| m.def.id == id)
    }
}

/// What to start.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum StartBattle {
    Mission(String),
    Skirmish,
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
            let first = catalog.start.clone().filter(|s| catalog.get(s).is_some()).or_else(|| catalog.missions.first().map(|m| m.def.id.clone()));
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
                start.write(StartBattle::Skirmish);
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
    mut commands: Commands,
) {
    let Some(request) = requests.read().last().cloned() else { return };
    let request = match request {
        StartBattle::Restart => state.mission.clone().map_or(StartBattle::Skirmish, StartBattle::Mission),
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
        _ => (scenario::skirmish(&rules, seed), None, vec![format!("New skirmish (seed {seed:#x})")]),
    };
    let begin_events = battle.begin();

    state.battle = battle;
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
        rig.focus_at(tile_top(map, u.pos));
    }
    reset.write(BattleReset);
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(battle.living(Team::Player).count() >= 4, "{f}: squad deployed");
            assert!(battle.living(Team::Enemy).count() >= 3, "{f}: enemies placed");
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
}

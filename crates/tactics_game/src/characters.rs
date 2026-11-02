//! Drop-in characters: put a rigged glTF in `assets/characters/` and play.
//!
//! ```text
//! assets/characters/
//! ├── robot.gltf               a rigged, animated character (.gltf or .glb)
//! ├── robot.character.ron      optional settings (size, facing, slots, replace)
//! └── index.txt                optional list of files (needed on web/Android)
//! ```
//!
//! For every character file the game:
//! 1. loads it and registers a model with the file's name (`robot`),
//! 2. maps its animation clips to slots **by name** ([`slot_for_clip`]):
//!    `Idle`, `Walk`/`Run`, `Fire`/`Shoot`, `Die`/`Death`, `Hit`... Exporter
//!    prefixes like `Armature|` are ignored; other names become custom slots,
//! 3. scales it to soldier height with its feet on the ground (unless the
//!    sidecar sets `scale`) and turns it 180° (glTF characters face +Z, the
//!    game faces -Z),
//! 4. optionally **replaces** existing models (`replace: ["soldier"]`), so
//!    it shows up in battle right away.
//!
//! The **Character Lab** (F7, or the Missions list) previews every model and
//! plays each slot; see [`crate::lab`].

use std::collections::BTreeMap;

use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use tactics_core::content::{ModelDef, PrimitiveShape};

use crate::FrameSet;
use crate::hud::LogMessages;
use crate::missions::XmlText;
use crate::models::{LoadedClip, LoadedModel, ModelLibrary};

pub struct CharactersPlugin;

impl Plugin for CharactersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CharacterRoster>()
            .add_message::<RefreshUnitVisuals>()
            .add_systems(Startup, discover_characters)
            .add_systems(Update, register_characters.in_set(FrameSet::Logic));
    }
}

/// Folder (under `assets/`) scanned for characters.
pub const CHARACTER_DIR: &str = "characters";
/// Default height of an auto-fitted character, in metres (a soldier is ~1.3).
pub const DEFAULT_HEIGHT: f32 = 1.3;

/// Rebuild all unit visuals (models changed).
#[derive(Message, Debug, Clone, Copy)]
pub struct RefreshUnitVisuals;

/// Optional `<name>.character.ron` next to the model file.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct CharacterSettings {
    /// Display name in the Character Lab.
    pub name: Option<String>,
    /// Fixed scale. `None` = auto-fit to `height`.
    pub scale: Option<f32>,
    /// Target height for auto-fit, metres.
    pub height: f32,
    /// Turn the model so it faces -Z. 180 for standard glTF (+Z forward).
    pub yaw_degrees: f32,
    /// Offset from the tile centre (x, y, z).
    pub offset: [f32; 3],
    /// Clip name → slot, overriding the automatic mapping
    /// (e.g. `{"mixamo.com": "idle"}`).
    pub slots: BTreeMap<String, String>,
    /// Slots that loop (in addition to idle / walk / hunker).
    pub looping: Vec<String>,
    /// Existing model ids this character replaces (`["soldier"]`).
    pub replace: Vec<String>,
}

impl Default for CharacterSettings {
    fn default() -> Self {
        Self {
            name: None,
            scale: None,
            height: DEFAULT_HEIGHT,
            yaw_degrees: 180.0,
            offset: [0.0; 3],
            slots: BTreeMap::new(),
            looping: Vec::new(),
            replace: Vec::new(),
        }
    }
}

/// Maps an animation clip name to a game slot.
///
/// Strips exporter prefixes (`Armature|Walk`, `rig:Walk`, `Take 001/Walk`),
/// lower-cases, and applies common aliases. Unknown names become their own
/// (custom) slot, e.g. `Wave` → `wave`.
pub fn slot_for_clip(name: &str) -> String {
    let base = name.rsplit(['|', ':', '/']).next().unwrap_or(name);
    let norm: String = base
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    let aliases: &[(&str, &[&str])] = &[
        ("idle", &["idle", "stand", "standing", "breathe", "breathing", "idle_loop", "rest"]),
        ("walk", &["walk", "walking", "run", "running", "jog", "jogging", "move", "locomotion", "walk_loop", "run_loop"]),
        ("shoot", &["shoot", "shooting", "fire", "firing", "attack", "shot", "aim_fire"]),
        ("overwatch", &["aim", "aiming", "overwatch"]),
        ("throw", &["throw", "throwing", "toss", "grenade"]),
        ("reload", &["reload", "reloading"]),
        ("hit", &["hit", "hurt", "damage", "damaged", "get_hit", "gethit", "flinch", "impact", "hit_reaction"]),
        ("death", &["death", "die", "dying", "dead", "death_loop"]),
        ("hunker", &["hunker", "crouch", "crouching", "cover", "crouch_idle", "kneel_idle"]),
        ("heal", &["heal", "healing", "interact", "use", "kneel"]),
        ("cast", &["cast", "casting", "spell", "magic", "psi"]),
    ];
    for (slot, names) in aliases {
        if names.contains(&norm.as_str()) {
            return slot.to_string();
        }
    }
    if norm.is_empty() { "clip".into() } else { norm }
}

/// One discovered character.
#[derive(Debug)]
struct CharacterEntry {
    id: String,
    file: String,
    gltf: Handle<Gltf>,
    settings: Handle<XmlText>,
    done: bool,
}

/// All characters found in `assets/characters/`.
#[derive(Resource, Default)]
pub struct CharacterRoster {
    entries: Vec<CharacterEntry>,
    index: Option<Handle<XmlText>>,
    /// Model ids of characters ready to use.
    pub ready: Vec<String>,
}

fn is_model_file(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".glb") || n.ends_with(".gltf")
}

fn add_entry(roster: &mut CharacterRoster, assets: &AssetServer, file: &str) {
    let file = file.trim();
    if !is_model_file(file) || roster.entries.iter().any(|e| e.file == file) {
        return;
    }
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    roster.entries.push(CharacterEntry {
        id: stem.to_string(),
        file: file.to_string(),
        gltf: assets.load(format!("{CHARACTER_DIR}/{file}")),
        settings: assets.load(format!("{CHARACTER_DIR}/{stem}.character.ron")),
        done: false,
    });
}

/// Desktop: scan the folder. Everywhere: also read `characters/index.txt`.
fn discover_characters(assets: Res<AssetServer>, mut roster: ResMut<CharacterRoster>) {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        let dir = bevy::asset::io::file::FileAssetReader::get_base_path().join("assets").join(CHARACTER_DIR);
        if let Ok(read) = std::fs::read_dir(&dir) {
            let mut files: Vec<String> = read.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            files.sort();
            for f in files {
                add_entry(&mut roster, &assets, &f);
            }
        }
    }
    roster.index = Some(assets.load(format!("{CHARACTER_DIR}/index.txt")));
}

/// Registers characters as their files finish loading.
#[allow(clippy::too_many_arguments)]
fn register_characters(
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    texts: Res<Assets<XmlText>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut roster: ResMut<CharacterRoster>,
    mut library: ResMut<ModelLibrary>,
    mut log: ResMut<LogMessages>,
    mut refresh: MessageWriter<RefreshUnitVisuals>,
) {
    // index.txt (optional): one file name per line.
    if let Some(index) = roster.index.clone() {
        if let Some(text) = texts.get(&index) {
            let files: Vec<String> = text.0.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
            for f in files {
                add_entry(&mut roster, &assets, &f);
            }
            roster.index = None;
        } else if matches!(assets.load_state(&index), LoadState::Failed(_)) {
            roster.index = None;
        }
    }

    let mut changed = false;
    let mut ready = Vec::new();
    for entry in roster.entries.iter_mut().filter(|e| !e.done) {
        // Wait for the model, and for the sidecar to load or be missing.
        if let LoadState::Failed(e) = assets.load_state(&entry.gltf) {
            log.push(format!("Character {}: could not load ({e})", entry.file));
            entry.done = true;
            continue;
        }
        let Some(gltf) = gltfs.get(&entry.gltf) else { continue };
        let settings = match texts.get(&entry.settings) {
            Some(text) => match ron::Options::default()
                .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
                .from_str::<CharacterSettings>(&text.0)
            {
                Ok(s) => s,
                Err(e) => {
                    log.push(format!("Character {}: settings error: {e}", entry.id));
                    CharacterSettings::default()
                }
            },
            None if matches!(assets.load_state(&entry.settings), LoadState::Failed(_)) => CharacterSettings::default(),
            None => continue,
        };
        entry.done = true;

        let Some(scene) = gltf.default_scene.clone().or_else(|| gltf.scenes.first().cloned()) else {
            log.push(format!("Character {}: the file has no scene", entry.file));
            continue;
        };

        // Clips → slots: automatic by name, overridable in the sidecar.
        let mut named: Vec<(String, Handle<AnimationClip>)> =
            gltf.named_animations.iter().map(|(n, h)| (n.to_string(), h.clone())).collect();
        named.sort_by(|a, b| a.0.cmp(&b.0));
        if named.is_empty() {
            named = gltf.animations.iter().enumerate().map(|(i, h)| (format!("anim{i}"), h.clone())).collect();
        }
        let mut mapping: BTreeMap<String, (String, Handle<AnimationClip>)> = BTreeMap::new();
        for (name, handle) in named {
            let slot = settings.slots.get(&name).cloned().unwrap_or_else(|| slot_for_clip(&name));
            // First clip wins a slot (e.g. both "Walk" and "Run" → walk).
            mapping.entry(slot).or_insert((name, handle));
        }
        let (graph, nodes) = AnimationGraph::from_clips(mapping.values().map(|(_, h)| h.clone()));
        let clips = mapping
            .iter()
            .zip(nodes)
            .map(|((slot, (name, handle)), node)| {
                let looping = matches!(slot.as_str(), "idle" | "walk" | "hunker") || settings.looping.contains(slot);
                (slot.clone(), LoadedClip { name: name.clone(), node, clip: handle.clone(), looping, speed: 1.0 })
            })
            .collect();
        let summary: Vec<String> = mapping.iter().map(|(slot, (name, _))| format!("{name}->{slot}")).collect();

        let model = LoadedModel {
            def: ModelDef {
                id: entry.id.clone(),
                fallback: PrimitiveShape::Soldier,
                skeleton: Some("humanoid".into()),
                scale: settings.scale.unwrap_or(1.0),
                offset: settings.offset,
                yaw_degrees: settings.yaw_degrees,
                ..Default::default()
            },
            scene: Some(scene),
            graph: Some(graphs.add(graph)),
            clips,
            auto_fit: settings.scale.is_none().then_some(settings.height),
            fit: None,
            source: format!("character {}", entry.file),
        };
        // The humanoid rig stands in while the scene spawns.
        let stand_in = library.rig("soldier").cloned();
        for id in std::iter::once(entry.id.clone()).chain(settings.replace.iter().cloned()) {
            library.insert(id.clone(), model.clone());
            if let Some(rig) = &stand_in {
                library.insert_rig(id, rig.clone());
            }
        }
        let name = settings.name.clone().unwrap_or_else(|| entry.id.clone());
        log.push(format!("Character '{name}' ready: {}", if summary.is_empty() { "no animations".into() } else { summary.join(", ") }));
        if !settings.replace.is_empty() {
            log.push(format!("  replaces: {}", settings.replace.join(", ")));
        }
        ready.push(entry.id.clone());
        changed = true;
    }
    roster.ready.extend(ready);
    if changed {
        refresh.write(RefreshUnitVisuals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_names_map_to_slots() {
        assert_eq!(slot_for_clip("Idle"), "idle");
        assert_eq!(slot_for_clip("Armature|Run"), "walk");
        assert_eq!(slot_for_clip("rig:Walking"), "walk");
        assert_eq!(slot_for_clip("Take 001/Fire"), "shoot");
        assert_eq!(slot_for_clip("Armature|Die"), "death");
        assert_eq!(slot_for_clip("Get Hit"), "hit");
        assert_eq!(slot_for_clip("Crouch_Idle"), "hunker");
        assert_eq!(slot_for_clip("Wave"), "wave", "unknown names become custom slots");
        assert_eq!(slot_for_clip("mixamo.com"), "mixamo_com");
    }

    #[test]
    fn settings_defaults_and_parsing() {
        let s: CharacterSettings = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(r#"(name: "Bot", height: 1.5, slots: {"mixamo.com": "idle"}, replace: ["soldier"])"#)
            .unwrap();
        assert_eq!(s.name.as_deref(), Some("Bot"));
        assert_eq!(s.yaw_degrees, 180.0, "glTF faces +Z by default");
        assert!(s.scale.is_none(), "auto-fit by default");
        assert_eq!(s.slots["mixamo.com"], "idle");
        assert_eq!(s.replace, ["soldier"]);
    }
}

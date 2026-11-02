//! HTML/CSS user interface layouts, rendered natively with `bevy_ui`.
//!
//! The battle HUD is an HTML document (`assets/ui/hud.html` by default). It is
//! parsed by a small built-in HTML + CSS engine ([`dom`], [`css`], [`style`])
//! and turned into ordinary Bevy UI entities ([`build`]), so it looks and
//! behaves the same on Windows, Linux, Android and the web — no browser or
//! webview involved.
//!
//! Layouts are dynamic through three attributes and one syntax:
//!
//! | feature | example |
//! |---|---|
//! | text binding | `<span>HP {{ unit.hp }}/{{ unit.max_hp }}</span>` |
//! | conditional | `<div data-if="battle.over">…</div>` (`!x`, `x == v`, `x != v`) |
//! | repetition | `<div data-for="a in abilities"><button data-command="ability:{{a.id}}">{{a.name}}</button></div>` |
//! | actions | `<button data-command="end_turn">End turn</button>` |
//!
//! Available data is produced by [`data::collect_ui_data`]; commands are
//! handled by [`run_ui_commands`]. See `UI.md` for the full reference.
//!
//! Where the HUD comes from (first match wins):
//! 1. desktop: `ui/hud.html` in the working directory (edit it and press
//!    **F6** to reload live),
//! 2. the `"hud"` [`UiLayoutDef`](tactics_core::content::UiLayoutDef) of the
//!    loaded modules (a RON module can ship a whole new HUD),
//! 3. the built-in `assets/ui/hud.html`.

pub mod bind;
pub mod build;
pub mod css;
pub mod data;
pub mod dom;
pub mod style;

use bevy::prelude::*;

use crate::FrameSet;
use crate::camera_director::CameraDirector;
use crate::editor::EditorCommand;
use crate::hud::LogMessages;
use crate::state::{BattleState, PlayerCommand, Selection};
use build::{BuildCtx, HtmlRoot, UiCommand};

pub struct HtmlUiPlugin;

impl Plugin for HtmlUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiData>()
            .init_resource::<UiFlags>()
            .init_resource::<HtmlLayout>()
            .add_message::<UiCommand>()
            .add_systems(Startup, request_initial_build)
            .add_systems(Update, (reload_hotkey, run_ui_commands).in_set(FrameSet::Input))
            .add_systems(
                Update,
                (
                    data::collect_ui_data,
                    rebuild_layout,
                    build::update_repeats,
                    build::update_bound_text,
                    build::update_conditions,
                    build::update_state_colors,
                )
                    .chain()
                    .in_set(FrameSet::Present),
            );
    }
}

/// Values and lists layouts bind to (see [`data`]).
#[derive(Resource, Default)]
pub struct UiData {
    pub values: bind::UiValues,
}

/// Layout-local on/off switches: `data-command="toggle:missions"` flips
/// `ui.missions`, readable in bindings (`data-if="ui.missions"`). Lets a layout
/// open and close its own panels without any Rust.
#[derive(Resource, Default)]
pub struct UiFlags(pub std::collections::BTreeMap<String, bool>);

/// The current layout's stylesheet and source, plus a rebuild flag.
#[derive(Resource, Default)]
pub struct HtmlLayout {
    pub sheet: css::Stylesheet,
    pub source: String,
    /// Where the HTML came from (for messages).
    pub origin: String,
    pub dirty: bool,
}

/// Folder (desktop) checked for user UI overrides such as `ui/hud.html`.
pub const USER_UI_DIR: &str = "ui";

/// Built-in HUD, compiled in so it works on every platform.
pub const DEFAULT_HUD: &str = include_str!("../../assets/ui/hud.html");

/// Finds the HUD markup: user file → module layout → built-in.
fn load_hud_source(state: &BattleState) -> (String, String) {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        let path = std::path::Path::new(USER_UI_DIR).join("hud.html");
        if let Ok(text) = std::fs::read_to_string(&path) {
            return (text, path.display().to_string());
        }
    }
    if let Some(layout) = state.battle.rules().ui_layout("hud") {
        return (layout.html.clone(), "module layout 'hud'".into());
    }
    (DEFAULT_HUD.to_string(), "built-in assets/ui/hud.html".into())
}

fn request_initial_build(mut layout: ResMut<HtmlLayout>) {
    layout.dirty = true;
}

fn reload_hotkey(keys: Res<ButtonInput<KeyCode>>, mut layout: ResMut<HtmlLayout>) {
    if keys.just_pressed(KeyCode::F6) {
        layout.dirty = true;
    }
}

/// (Re)builds the HUD from source when flagged dirty.
fn rebuild_layout(
    mut commands: Commands,
    mut layout: ResMut<HtmlLayout>,
    state: Res<BattleState>,
    data: Res<UiData>,
    assets: Res<AssetServer>,
    old: Query<Entity, With<HtmlRoot>>,
    mut log: ResMut<LogMessages>,
) {
    if !layout.dirty {
        return;
    }
    layout.dirty = false;
    for e in &old {
        commands.entity(e).despawn();
    }
    let (source, origin) = load_hud_source(&state);
    let doc = dom::parse(&source);
    layout.sheet = css::parse_stylesheet(&doc.css);
    let mut warnings = Vec::new();
    let mut ctx = BuildCtx { sheet: &layout.sheet, globals: &data.values, assets: &assets, warnings: &mut warnings };
    build::spawn_document(&mut commands, &doc.roots, &mut ctx);
    for w in warnings {
        log.push(format!("UI: {w}"));
    }
    if origin != layout.origin || layout.source != source {
        info!("HUD loaded from {origin}");
    }
    layout.source = source;
    layout.origin = origin;
}

/// Turns `data-command` strings into game actions. **Add new commands here.**
///
/// | command | effect |
/// |---|---|
/// | `end_turn`, `next_unit`, `cancel`, `restart` | same as the keyboard shortcuts |
/// | `ability:<id>` | use / start targeting an ability of the selected unit |
/// | `select:<unit id>` | select a squad member (see the `squad` list) |
/// | `editor` | toggle the module editor |
/// | `cinematics` | toggle cinematic cameras |
/// | `reload_ui` | rebuild the HUD from its source |
/// | `toggle:<name>` | flip the layout flag `ui.<name>` (show/hide panels) |
/// | `mission:<id>` | start a mission from `assets/missions/` |
/// | `skirmish` | start a procedural skirmish |
/// | `model:<id>` | Character Lab: show this model on the selected unit |
/// | `anim:<slot>` | Character Lab: play this animation slot |
/// | `lab_walk` | Character Lab: toggle the walk loop |
/// | `gear:<what>:<member>[:<slot>]` | change a squad member's loadout (see [`crate::loadout`]) |
/// | `apply_loadout` | restart the mission with the new loadouts |
/// | `log:<text>` | print a message (handy while designing layouts) |
#[allow(clippy::too_many_arguments)]
pub fn run_ui_commands(
    mut commands: MessageReader<UiCommand>,
    mut player: MessageWriter<PlayerCommand>,
    mut editor: MessageWriter<EditorCommand>,
    mut lab: MessageWriter<crate::lab::LabCommand>,
    mut director: ResMut<CameraDirector>,
    mut layout: ResMut<HtmlLayout>,
    mut selection: ResMut<Selection>,
    mut flags: ResMut<UiFlags>,
    mut state: ResMut<BattleState>,
    mut log: ResMut<LogMessages>,
) {
    for UiCommand(cmd) in commands.read() {
        let (name, arg) = cmd.split_once(':').map_or((cmd.as_str(), ""), |(n, a)| (n, a));
        match name.trim() {
            "end_turn" => {
                player.write(PlayerCommand::EndTurn);
            }
            "next_unit" => {
                player.write(PlayerCommand::NextUnit);
            }
            "cancel" => {
                player.write(PlayerCommand::CancelTargeting);
            }
            "restart" => {
                player.write(PlayerCommand::Restart);
            }
            "ability" => {
                player.write(PlayerCommand::UseAbility(arg.trim().into()));
            }
            "select" => {
                let id = arg.trim().parse().ok().map(tactics_core::unit::UnitId);
                let ok = id.and_then(|id| state.battle.unit(id)).is_some_and(|u| {
                    u.team == tactics_core::unit::Team::Player && u.is_alive() && state.battle.active_team() == u.team
                });
                if ok {
                    selection.select(id);
                }
            }
            "editor" => {
                editor.write(EditorCommand::Toggle);
            }
            "cinematics" => {
                director.enabled = !director.enabled;
                log.push(format!("Cinematic cameras {}", if director.enabled { "ON" } else { "OFF" }));
            }
            "reload_ui" => layout.dirty = true,
            "toggle" => {
                let f = flags.0.entry(arg.trim().to_string()).or_insert(false);
                *f = !*f;
            }
            "mission" => {
                player.write(PlayerCommand::StartMission(arg.trim().to_string()));
            }
            "skirmish" => {
                player.write(PlayerCommand::Skirmish);
            }
            "model" => {
                lab.write(crate::lab::LabCommand::SetModel(arg.trim().to_string()));
            }
            "anim" => {
                lab.write(crate::lab::LabCommand::Play(arg.trim().to_string()));
            }
            "lab_walk" => {
                lab.write(crate::lab::LabCommand::ToggleWalk);
            }
            "gear" => match crate::loadout::command(arg, state.battle.rules_mut()) {
                Some(msg) => log.push(msg),
                None => log.push(format!("UI: bad gear command '{arg}'")),
            },
            "apply_loadout" => {
                player.write(PlayerCommand::Restart);
            }
            "log" => log.push(arg.to_string()),
            other => log.push(format!("UI: unknown command '{other}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in HUD parses and uses only supported CSS.
    #[test]
    fn default_hud_is_valid() {
        let doc = dom::parse(DEFAULT_HUD);
        assert!(!doc.roots.is_empty());
        let sheet = css::parse_stylesheet(&doc.css);
        assert!(sheet.rules.len() > 10);
        for rule in &sheet.rules {
            let bad = style::unsupported(&rule.declarations);
            assert!(bad.is_empty(), "unsupported CSS in default HUD: {bad:?}");
        }
    }
}


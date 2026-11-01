//! Heads-up display built with `bevy_ui`.
//!
//! Layout (absolute-positioned panels over the 3D view):
//! ```text
//! ┌ turn / round ─────────────────────────── combat log ┐
//! │ [module editor (F1)]                                 │
//! │                 floating combat text                 │
//! │                                       hover/odds info│
//! └ selected unit ── [ability buttons…] [Next][End Turn] ┘
//! ```
//! The ability bar is **generated from the selected unit's abilities**, so
//! moves added by modules get a button (and a number-key hotkey) automatically.
//! Buttons are finger-sized and send the same [`PlayerCommand`]s as the
//! keyboard, so the game is fully playable on touch screens.

use std::collections::VecDeque;

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, observe};
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::grid_to_world;
use crate::input::target_for;
use crate::playback::Playback;
use crate::state::{BattleReset, BattleState, ModuleLoadLog, PlayerCommand, Selection};
use crate::theme;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogMessages>()
            .add_systems(Startup, (spawn_hud, show_module_log))
            .add_systems(Update, handle_restart.in_set(FrameSet::Logic))
            .add_systems(
                Update,
                (
                    update_turn_text,
                    update_unit_panel,
                    update_hover_info,
                    rebuild_ability_bar,
                    update_log,
                    update_game_over,
                    style_buttons,
                    animate_floating_text,
                )
                    .in_set(FrameSet::Present),
            );
    }
}

// ----------------------------------------------------------------------
// Markers
// ----------------------------------------------------------------------

#[derive(Component)]
struct TurnText;
/// The selected-unit panel (hidden while the editor is open).
#[derive(Component)]
pub struct UnitPanel;
#[derive(Component)]
struct UnitPanelText;
#[derive(Component)]
struct HoverInfoText;
#[derive(Component)]
struct LogText;
#[derive(Component)]
struct GameOverPanel;
#[derive(Component)]
struct GameOverText;
/// Container whose children are the selected unit's ability buttons.
#[derive(Component)]
struct AbilityBar;

/// Visual state of a HUD button, used by [`style_buttons`].
#[derive(Component, Default, Clone, Copy, PartialEq, Eq)]
pub enum ButtonLook {
    #[default]
    Normal,
    /// Currently selected (e.g. the ability being targeted).
    Active,
    /// Can't be used right now (still clickable: it explains why in the log).
    Disabled,
}

/// Recent combat log lines.
#[derive(Resource, Default)]
pub struct LogMessages(VecDeque<String>);

impl LogMessages {
    const MAX: usize = 8;

    pub fn push(&mut self, line: String) {
        info!("{line}");
        self.0.push_back(line);
        while self.0.len() > Self::MAX {
            self.0.pop_front();
        }
    }
}

/// Text that floats up from a world position and fades (damage numbers etc).
#[derive(Component)]
pub struct FloatingText {
    world: Vec3,
    age: f32,
}

impl FloatingText {
    const LIFETIME: f32 = 1.3;

    pub fn spawn(commands: &mut Commands, world: Vec3, text: String, color: Color) {
        commands.spawn((
            FloatingText { world, age: 0.0 },
            Text::new(text),
            TextFont { font_size: FontSize::Px(22.0), ..default() },
            TextColor(color),
            TextShadow::default(),
            Node { position_type: PositionType::Absolute, ..default() },
            // Don't block clicks on the board.
            Pickable::IGNORE,
        ));
    }
}

// ----------------------------------------------------------------------
// Construction helpers (also used by the editor)
// ----------------------------------------------------------------------

pub fn text(size: f32, color: Color) -> (TextFont, TextColor) {
    (TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

pub fn panel() -> (BackgroundColor, BorderColor) {
    (BackgroundColor(theme::UI_PANEL), BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.08)))
}

pub fn panel_node(extra: Node) -> Node {
    Node {
        position_type: PositionType::Absolute,
        padding: UiRect::all(px(10)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(6)),
        flex_direction: FlexDirection::Column,
        ..extra
    }
}

/// A button that sends message `msg` when activated. `size` = (min width, height).
pub fn message_button<M: Message + Clone>(label: &str, sub: &str, msg: M, size: (f32, f32), look: ButtonLook) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        look,
        Node {
            min_width: px(size.0),
            height: px(size.1),
            margin: UiRect::left(px(6)),
            padding: UiRect::horizontal(px(8)),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(theme::UI_BUTTON),
        observe(move |_: On<Activate>, mut out: MessageWriter<M>| {
            out.write(msg.clone());
        }),
        children![
            (Text::new(label.to_string()), text(15.0, theme::UI_TEXT), Pickable::IGNORE),
            (Text::new(sub.to_string()), text(11.0, theme::UI_DIM), Pickable::IGNORE),
        ],
    )
}

/// A finger-sized HUD button.
fn hud_button(label: &str, key: &str, command: PlayerCommand, look: ButtonLook) -> impl Bundle {
    message_button(label, key, command, (88.0, 56.0), look)
}

fn spawn_hud(mut commands: Commands) {
    // Top-left: turn indicator.
    commands.spawn((
        panel_node(Node { top: px(12), left: px(12), ..default() }),
        panel(),
        children![(TurnText, Text::new(""), text(20.0, theme::UI_TEXT))],
    ));

    // Top-right: combat log.
    commands.spawn((
        panel_node(Node { top: px(12), right: px(12), max_width: px(440), ..default() }),
        panel(),
        children![(LogText, Text::new(""), text(13.0, theme::UI_DIM))],
    ));

    // Bottom-left: selected unit.
    commands.spawn((
        UnitPanel,
        panel_node(Node { bottom: px(80), left: px(12), min_width: px(220), ..default() }),
        panel(),
        children![(UnitPanelText, Text::new(""), text(15.0, theme::UI_TEXT))],
    ));

    // Right, above the buttons: hover info / odds.
    commands.spawn((
        panel_node(Node { bottom: px(80), right: px(12), max_width: px(380), ..default() }),
        panel(),
        children![(HoverInfoText, Text::new(""), text(14.0, theme::UI_TEXT))],
    ));

    // Bottom row: [ability buttons...] [Next] [End Turn]. Wraps on narrow screens.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(6),
            right: px(12),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::WrapReverse,
            justify_content: JustifyContent::FlexEnd,
            ..default()
        },
        Pickable::IGNORE,
        children![
            (AbilityBar, Node { flex_direction: FlexDirection::Row, ..default() }, Pickable::IGNORE),
            hud_button("Next", "Tab", PlayerCommand::NextUnit, ButtonLook::Normal),
            hud_button("End Turn", "Enter", PlayerCommand::EndTurn, ButtonLook::Normal),
        ],
    ));

    // Centre: game-over panel (hidden until the battle ends).
    commands.spawn((
        GameOverPanel,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            display: Display::None,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            panel_node(Node {
                position_type: PositionType::Relative,
                align_items: AlignItems::Center,
                padding: UiRect::all(px(24)),
                ..default()
            }),
            panel(),
            children![
                (GameOverText, Text::new(""), text(32.0, theme::UI_TEXT)),
                (Node { height: px(16), ..default() }),
                hud_button("New Battle", "F5", PlayerCommand::Restart, ButtonLook::Normal),
            ],
        )],
    ));
}

fn show_module_log(load: Res<ModuleLoadLog>, mut log: ResMut<LogMessages>) {
    for line in &load.0 {
        log.push(line.clone());
    }
    log.push("F1: module editor".into());
}

// ----------------------------------------------------------------------
// Updates
// ----------------------------------------------------------------------

fn update_turn_text(state: Res<BattleState>, mut q: Single<&mut Text, With<TurnText>>) {
    if !state.is_changed() {
        return;
    }
    let b = &state.battle;
    let team = match b.active_team() {
        Team::Player => "XCOM TURN",
        Team::Enemy => "ALIEN ACTIVITY",
    };
    let alive = |t| b.living(t).count();
    q.0 = format!(
        "Round {} - {team}\nSquad {}  vs  Hostiles {}",
        b.round(),
        alive(Team::Player),
        alive(Team::Enemy)
    );
}

fn update_unit_panel(state: Res<BattleState>, selection: Res<Selection>, mut q: Single<&mut Text, With<UnitPanelText>>) {
    let b = &state.battle;
    let s = match selection.unit.and_then(|id| b.unit(id)) {
        Some(u) => {
            let class = b.rules().class(&u.class).map_or(u.class.as_str(), |c| c.name.as_str());
            let mut status = Vec::new();
            if u.status.overwatch.is_some() {
                status.push("Overwatch".to_string());
            }
            if u.status.defense_bonus > 0 {
                status.push(format!("Defending +{}", u.status.defense_bonus));
            }
            format!(
                "{} ({class})\nHP {}/{}   AP {}/{}\n{}  ammo {}/{}\nAim {}  Mobility {}{}",
                u.name,
                u.hp,
                u.stats.max_hp,
                u.action_points,
                b.rules().combat.action_points,
                u.weapon.name,
                u.weapon.ammo,
                u.weapon.clip_size,
                u.stats.aim,
                u.stats.mobility / 2,
                if status.is_empty() { String::new() } else { format!("\n[{}]", status.join(", ")) },
            )
        }
        None => "No unit selected\nTap a soldier".to_string(),
    };
    if q.0 != s {
        q.0 = s;
    }
}

/// Formats a shot preview with its modifier breakdown.
fn describe_preview(name: &str, hp: (i32, i32), p: &ShotPreview, out: &mut String) {
    out.push_str(&format!(
        "{name}  HP {}/{}\nHIT {}%   CRIT {}%   DMG {}-{}\n",
        hp.0, hp.1, p.hit_chance, p.crit_chance, p.damage.0, p.damage.1
    ));
    for (label, value) in &p.breakdown {
        out.push_str(&format!("  {label}: {value:+}\n"));
    }
}

fn update_hover_info(state: Res<BattleState>, selection: Res<Selection>, mut q: Single<&mut Text, With<HoverInfoText>>) {
    let b = &state.battle;
    let mut s = String::new();
    if let (Some(sel), Some(hover)) = (selection.unit, selection.hovered) {
        // Which ability are we previewing? The pending one, or the primary
        // attack when hovering an enemy.
        let ability = selection.pending.clone().or_else(|| {
            b.unit_at(hover)
                .filter(|u| u.team == Team::Enemy)
                .and_then(|_| b.unit(sel).and_then(|u| b.primary_attack(u)).map(|d| d.id.clone()))
        });
        if let Some(def) = ability.as_ref().and_then(|a| b.ability(a)) {
            s = format!("[{}]\n", def.name);
            match target_for(b, def, hover).map(|t| (t, b.preview(sel, &def.id, t))) {
                Some((_, Ok(previews))) if !previews.is_empty() => {
                    for (victim, p) in &previews {
                        if let Some(v) = b.unit(*victim) {
                            describe_preview(&v.name, (v.hp, v.stats.max_hp), p, &mut s);
                        }
                    }
                }
                Some((_, Ok(_))) => s.push_str(&def.description),
                Some((_, Err(e))) => s.push_str(&format!("{e}")),
                None => s.push_str("Pick a target (Esc to cancel)"),
            }
        } else if let Some(cost) = selection.range.as_ref().and_then(|r| r.cost(hover)).filter(|c| *c > 0) {
            let ap = b.move_cost_ap(sel, cost);
            let cover = tactics_core::cover::best_cover_at(b.map(), hover);
            s = format!("{} - {ap} AP\nCover: {cover:?}", if ap > 1 { "Dash" } else { "Move" });
        }
    }
    if s.is_empty() {
        s = "Hover an enemy for odds,\na tile to move".into();
    }
    if q.0 != s {
        q.0 = s;
    }
}

/// What the ability bar was last built for: (unit, pending ability, revision).
type AbilityBarKey = (Option<UnitId>, Option<AbilityId>, u64);

/// Rebuilds the ability buttons whenever the selection, pending ability or
/// battle state changes.
fn rebuild_ability_bar(
    mut commands: Commands,
    state: Res<BattleState>,
    selection: Res<Selection>,
    bar: Single<Entity, With<AbilityBar>>,
    mut last_key: Local<Option<AbilityBarKey>>,
) {
    let key = (selection.unit, selection.pending.clone(), state.revision);
    if last_key.as_ref() == Some(&key) {
        return;
    }
    *last_key = Some(key);

    let mut bar = commands.entity(*bar);
    bar.despawn_related::<Children>();
    let b = &state.battle;
    let Some(unit) = selection.unit.and_then(|id| b.unit(id)) else { return };

    bar.with_children(|parent| {
        for (slot, id) in unit.abilities.iter().enumerate() {
            let Some(def) = b.ability(id) else { continue };
            // Probe with no target: target errors are fine (they'll be picked
            // next), cost errors mean the button is disabled.
            let probe = Action::UseAbility { unit: unit.id, ability: id.clone(), target: Target::None };
            let usable = matches!(
                b.validate(&probe),
                Ok(())
                    | Err(ActionError::WrongTarget
                        | ActionError::OutOfRange
                        | ActionError::NoLineOfSight
                        | ActionError::InvalidTarget(_)
                        | ActionError::NothingToHeal)
            );
            let look = if selection.pending.as_ref() == Some(id) {
                ButtonLook::Active
            } else if usable {
                ButtonLook::Normal
            } else {
                ButtonLook::Disabled
            };
            let cost = match def.cost {
                ApCost::Points(n) => format!("{n} AP"),
                ApCost::EndsTurn => "ends turn".into(),
            };
            let mut sub = format!("{} | {cost}", slot + 1);
            if let Some(left) = b.charges_left(unit, def) {
                sub.push_str(&format!(" | x{left}"));
            }
            let cd = unit.cooldown(id);
            if cd > 0 {
                sub.push_str(&format!(" | cd {cd}"));
            }
            parent.spawn(hud_button(&def.name, &sub, PlayerCommand::UseAbility(id.clone()), look));
        }
    });
}

fn update_log(log: Res<LogMessages>, mut q: Single<&mut Text, With<LogText>>) {
    if log.is_changed() {
        q.0 = log.0.iter().cloned().collect::<Vec<_>>().join("\n");
    }
}

fn update_game_over(
    state: Res<BattleState>,
    playback: Res<Playback>,
    mut panel: Single<&mut Node, With<GameOverPanel>>,
    mut text: Single<&mut Text, With<GameOverText>>,
) {
    // Wait for the final animations before showing the result.
    let show = state.battle.is_over() && !playback.is_busy();
    let display = if show { Display::Flex } else { Display::None };
    if panel.display != display {
        panel.display = display;
    }
    if show {
        text.0 = match state.battle.winner() {
            Some(Team::Player) => "MISSION ACCOMPLISHED".into(),
            _ => "MISSION FAILED".into(),
        };
    }
}

fn style_buttons(mut q: Query<(&Hovered, Has<Pressed>, &ButtonLook, &mut BackgroundColor), With<Button>>) {
    for (hovered, pressed, look, mut bg) in &mut q {
        let color = match (look, hovered.get(), pressed) {
            (_, _, true) | (ButtonLook::Active, _, _) => theme::UI_BUTTON_PRESSED,
            (ButtonLook::Disabled, _, _) => theme::UI_BUTTON_DISABLED,
            (_, true, false) => theme::UI_BUTTON_HOVER,
            _ => theme::UI_BUTTON,
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
}

fn animate_floating_text(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<(&Camera, &GlobalTransform), With<CameraRig>>,
    mut q: Query<(Entity, &mut FloatingText, &mut Node, &mut TextColor)>,
) {
    let (camera, cam_tr) = *camera;
    for (e, mut ft, mut node, mut color) in &mut q {
        ft.age += time.delta_secs();
        if ft.age >= FloatingText::LIFETIME {
            commands.entity(e).despawn();
            continue;
        }
        let world = ft.world + Vec3::Y * ft.age * 0.6;
        if let Ok(screen) = camera.world_to_viewport(cam_tr, world) {
            node.left = px(screen.x - 30.0);
            node.top = px(screen.y - 12.0);
        }
        let alpha = 1.0 - (ft.age / FloatingText::LIFETIME).powi(2);
        color.0.set_alpha(alpha);
    }
}

/// Restart: build a fresh procedural skirmish with a new seed, keeping the
/// current (possibly live-edited) ruleset.
#[allow(clippy::too_many_arguments)]
fn handle_restart(
    mut commands_in: MessageReader<PlayerCommand>,
    mut reset: MessageWriter<BattleReset>,
    mut state: ResMut<BattleState>,
    mut playback: ResMut<Playback>,
    mut selection: ResMut<Selection>,
    mut log: ResMut<LogMessages>,
    mut rig: Single<&mut CameraRig>,
    floating: Query<Entity, With<FloatingText>>,
    mut commands: Commands,
) {
    if !commands_in.read().any(|c| *c == PlayerCommand::Restart) {
        return;
    }
    let seed = state.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let rules = state.battle.rules().clone();
    state.battle = scenario::skirmish(&rules, seed);
    state.seed = seed;
    state.revision += 1;
    playback.clear();
    *selection = Selection::default();
    log.0.clear();
    log.push(format!("New skirmish (seed {seed:#x})"));
    for e in &floating {
        commands.entity(e).despawn();
    }

    let map = state.battle.map();
    let far = grid_to_world(GridPos::new(map.width() - 1, map.height() - 1));
    rig.bounds = Rect::from_corners(Vec2::ZERO, Vec2::new(far.x, far.z));
    rig.focus_on(GridPos::new(3, 3));
    reset.write(BattleReset);
}

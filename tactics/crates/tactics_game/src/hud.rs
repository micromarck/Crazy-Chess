//! Heads-up display built with `bevy_ui`.
//!
//! Layout (absolute-positioned panels over the 3D view):
//! ```text
//! ┌ turn / round ─────────────────────────── combat log ┐
//! │                                                     │
//! │                 floating combat text                │
//! │                                                     │
//! └ selected unit ─── hover info ───────── [buttons] ───┘
//! ```
//! Buttons are large enough for fingers and send the same [`PlayerCommand`]s
//! as the keyboard shortcuts, so the game is fully playable on touch screens.

use std::collections::VecDeque;

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, observe};
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::grid_to_world;
use crate::playback::Playback;
use crate::state::{BattleReset, BattleState, PlayerCommand, Selection};
use crate::theme;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogMessages>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, handle_restart.in_set(FrameSet::Logic))
            .add_systems(
                Update,
                (
                    update_turn_text,
                    update_unit_panel,
                    update_hover_info,
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

/// Recent combat log lines.
#[derive(Resource, Default)]
pub struct LogMessages(VecDeque<String>);

impl LogMessages {
    const MAX: usize = 7;

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
// Construction
// ----------------------------------------------------------------------

fn text(size: f32, color: Color) -> (TextFont, TextColor) {
    (TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn panel() -> (BackgroundColor, BorderColor) {
    (BackgroundColor(theme::UI_PANEL), BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.08)))
}

fn panel_node(extra: Node) -> Node {
    Node {
        position_type: PositionType::Absolute,
        padding: UiRect::all(px(10)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(6)),
        flex_direction: FlexDirection::Column,
        ..extra
    }
}

/// A finger-sized button that emits `command` when activated.
fn hud_button(label: &str, key: &str, command: PlayerCommand) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        Node {
            min_width: px(96),
            height: px(56),
            margin: UiRect::left(px(6)),
            padding: UiRect::horizontal(px(10)),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(theme::UI_BUTTON),
        observe(move |_: On<Activate>, mut out: MessageWriter<PlayerCommand>| {
            out.write(command);
        }),
        children![
            (Text::new(label.to_string()), text(16.0, theme::UI_TEXT), Pickable::IGNORE),
            (Text::new(key.to_string()), text(11.0, theme::UI_DIM), Pickable::IGNORE),
        ],
    )
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
        panel_node(Node { top: px(12), right: px(12), max_width: px(420), ..default() }),
        panel(),
        children![(LogText, Text::new(""), text(13.0, theme::UI_DIM))],
    ));

    // Bottom-left: selected unit.
    commands.spawn((
        panel_node(Node { bottom: px(12), left: px(12), min_width: px(220), ..default() }),
        panel(),
        children![(UnitPanelText, Text::new(""), text(15.0, theme::UI_TEXT))],
    ));

    // Bottom-centre (above buttons on narrow screens): hover info.
    commands.spawn((
        panel_node(Node { bottom: px(84), right: px(12), max_width: px(360), ..default() }),
        panel(),
        children![(HoverInfoText, Text::new(""), text(14.0, theme::UI_TEXT))],
    ));

    // Bottom-right: action buttons.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            right: px(12),
            flex_direction: FlexDirection::Row,
            ..default()
        },
        children![
            hud_button("Next", "Tab", PlayerCommand::NextUnit),
            hud_button("Overwatch", "O", PlayerCommand::Overwatch),
            hud_button("Hunker", "H", PlayerCommand::Hunker),
            hud_button("Reload", "R", PlayerCommand::Reload),
            hud_button("End Turn", "Enter", PlayerCommand::EndTurn),
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
            panel_node(Node { position_type: PositionType::Relative, align_items: AlignItems::Center, padding: UiRect::all(px(24)), ..default() }),
            panel(),
            children![
                (GameOverText, Text::new(""), text(32.0, theme::UI_TEXT)),
                (Node { height: px(16), ..default() }),
                hud_button("New Battle", "F5", PlayerCommand::Restart),
            ],
        )],
    ));
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
    let s = match selection.unit.and_then(|id| state.battle.unit(id)) {
        Some(u) => {
            let mut status = Vec::new();
            if u.status.overwatch {
                status.push("Overwatch");
            }
            if u.status.hunkered {
                status.push("Hunkered");
            }
            format!(
                "{}\nHP {}/{}   AP {}/{}\n{}  ammo {}/{}\nAim {}  Mobility {}{}",
                u.name,
                u.hp,
                u.stats.max_hp,
                u.action_points,
                Unit::MAX_ACTION_POINTS,
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

fn update_hover_info(state: Res<BattleState>, selection: Res<Selection>, mut q: Single<&mut Text, With<HoverInfoText>>) {
    let b = &state.battle;
    let mut s = String::new();
    if let (Some(sel), Some(hover)) = (selection.unit, selection.hovered) {
        if let Some(target) = b.unit_at(hover).filter(|u| u.team == Team::Enemy) {
            match b.shot_preview(sel, target.id) {
                Ok(p) => {
                    s = format!(
                        "{}  HP {}/{}\nHIT {}%   CRIT {}%   DMG {}-{}\n",
                        target.name, target.hp, target.stats.max_hp, p.hit_chance, p.crit_chance, p.damage.0, p.damage.1
                    );
                    for (label, value) in &p.breakdown {
                        s.push_str(&format!("  {label}: {value:+}\n"));
                    }
                }
                Err(e) => s = format!("{}: {e}", target.name),
            }
        } else if let Some(cost) = selection.range.as_ref().and_then(|r| r.cost(hover)).filter(|c| *c > 0) {
            let ap = b.move_cost_ap(sel, cost);
            let cover = tactics_core::cover::best_cover_at(b.map(), hover);
            s = format!(
                "{} - {ap} AP\nCover: {:?}",
                if ap > 1 { "Dash" } else { "Move" },
                cover
            );
        }
    }
    if s.is_empty() {
        s = "Hover an enemy for odds,\na tile to move".into();
    }
    if q.0 != s {
        q.0 = s;
    }
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

fn style_buttons(mut q: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<Button>>) {
    for (hovered, pressed, mut bg) in &mut q {
        let color = match (hovered.get(), pressed) {
            (_, true) => theme::UI_BUTTON_PRESSED,
            (true, false) => theme::UI_BUTTON_HOVER,
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

/// Restart: build a fresh procedural skirmish with a new seed.
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
    state.battle = scenario::skirmish(seed);
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

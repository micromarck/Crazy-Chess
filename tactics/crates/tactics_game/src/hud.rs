//! Code-driven HUD pieces.
//!
//! The panels and buttons of the battle HUD are an **HTML layout** (see
//! [`html_ui`](crate::html_ui) and `assets/ui/hud.html`). This module keeps
//! what is better done in code:
//!
//! * [`LogMessages`]: the combat log store (shown by the layout's `log` list),
//! * [`FloatingText`]: damage numbers and labels that follow world positions,
//! * button/panel helpers and [`ButtonLook`] styling for code-built UI such as
//!   the module editor,
//! * starting a new battle ([`PlayerCommand::Restart`]).

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
use crate::state::{BattleReset, BattleState, ModuleLoadLog, PlayerCommand, Selection};
use crate::theme;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogMessages>()
            .add_systems(Startup, show_module_log)
            .add_systems(Update, handle_restart.in_set(FrameSet::Logic))
            .add_systems(Update, (style_buttons, animate_floating_text).in_set(FrameSet::Present));
    }
}

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

    /// The stored lines, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }

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

fn show_module_log(load: Res<ModuleLoadLog>, mut log: ResMut<LogMessages>) {
    for line in &load.0 {
        log.push(line.clone());
    }
    log.push("F1: module editor   F6: reload HTML UI".into());
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

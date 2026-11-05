//! In-game module editor (F1 or the "Edit" button).
//!
//! Tweak any loaded ability's numbers while playing (base accuracy, accuracy
//! bonus, half/full cover penalties, crit, damage, AP cost, cooldown, charges,
//! range, area radius, heal amount...). Changes apply to the running battle
//! immediately (hover an enemy to see the new odds).
//!
//! **Save** writes every ability plus the combat rules as a RON data module to
//! `modules/zz_editor_tuning.ron` (desktop). It loads automatically on the next
//! start, after all other modules, so your tuning sticks. Delete the file to
//! go back to the defaults. On web/Android the RON is printed to the log
//! instead.
//!
//! ## Adding an editable field
//! 1. Add a variant to [`EditField`].
//! 2. Return it from [`fields_for`] when it applies to an ability.
//! 3. Handle it in [`EditField::label`], [`EditField::value`] and
//!    [`EditField::adjust`].

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::content::data::DataModule;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::content;
use crate::hud::{ButtonLook, LogMessages, message_button, panel, panel_node, text};
use crate::state::BattleState;
use crate::theme;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorState>()
            .add_message::<EditorCommand>()
            .add_systems(Startup, spawn_editor)
            .add_systems(Update, editor_hotkey.in_set(FrameSet::Input))
            .add_systems(Update, apply_editor_commands.in_set(FrameSet::Logic))
            .add_systems(Update, (rebuild_editor, refresh_values, scroll_editor).chain().in_set(FrameSet::Present));
    }
}

/// One editable number (or toggle) of an ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    ApCost,
    AmmoCost,
    Cooldown,
    Charges,
    Range,
    BaseAccuracy,
    AccuracyBonus,
    HalfCoverPenalty,
    FullCoverPenalty,
    CritBonus,
    FlankCritBonus,
    DamageMin,
    DamageMax,
    DamageBonus,
    AreaRadius,
    CanMiss,
    RangeFalloff,
    DefenseBonus,
    HealAmount,
}

/// The fields that make sense for `def`, in display order.
pub fn fields_for(def: &AbilityDef) -> Vec<EditField> {
    use EditField::*;
    let mut f = vec![ApCost, AmmoCost, Cooldown, Charges];
    if matches!(def.range, AbilityRange::Fixed(_)) && def.targeting != Targeting::SelfOnly {
        f.push(Range);
    }
    match &def.effect {
        AbilityEffect::Attack(p) | AbilityEffect::Overwatch(p) | AbilityEffect::Suppress(p) => {
            f.extend([BaseAccuracy, AccuracyBonus, HalfCoverPenalty, FullCoverPenalty, CritBonus, FlankCritBonus]);
            if matches!(p.damage, DamageSource::Fixed { .. }) {
                f.extend([DamageMin, DamageMax]);
            }
            f.extend([DamageBonus, AreaRadius, CanMiss, RangeFalloff]);
        }
        AbilityEffect::Defend { .. } => f.push(DefenseBonus),
        AbilityEffect::Heal { .. } => f.push(HealAmount),
        AbilityEffect::Reload | AbilityEffect::Shutdown { .. } | AbilityEffect::Cool { .. } => {}
    }
    f
}

impl EditField {
    pub fn label(self) -> &'static str {
        use EditField::*;
        match self {
            ApCost => "AP cost",
            AmmoCost => "Ammo cost",
            Cooldown => "Cooldown (turns)",
            Charges => "Charges",
            Range => "Range (tiles)",
            BaseAccuracy => "Base accuracy",
            AccuracyBonus => "Accuracy bonus",
            HalfCoverPenalty => "Half cover penalty",
            FullCoverPenalty => "Full cover penalty",
            CritBonus => "Crit bonus",
            FlankCritBonus => "Flank crit bonus",
            DamageMin => "Damage min",
            DamageMax => "Damage max",
            DamageBonus => "Damage bonus",
            AreaRadius => "Area radius",
            CanMiss => "Can miss",
            RangeFalloff => "Range falloff",
            DefenseBonus => "Defense bonus",
            HealAmount => "Heal amount",
        }
    }

    /// Current value as display text.
    pub fn value(self, def: &AbilityDef) -> String {
        use EditField::*;
        let p = def.effect.attack_profile();
        let num = |v: i32| v.to_string();
        match self {
            ApCost => match def.cost {
                tactics_core::content::ApCost::Points(n) => format!("{n} AP"),
                tactics_core::content::ApCost::EndsTurn => "ends turn".into(),
            },
            AmmoCost => num(def.ammo_cost),
            Cooldown => def.cooldown.to_string(),
            Charges => def.charges.map_or("unlimited".into(), |c| c.to_string()),
            Range => match def.range {
                AbilityRange::Fixed(r) => format!("{r:.1}"),
                AbilityRange::Weapon => "weapon".into(),
            },
            BaseAccuracy => p.map_or("-".into(), |p| p.base_accuracy.map_or("unit aim".into(), |a| format!("{a}%"))),
            AccuracyBonus => p.map_or("-".into(), |p| format!("{:+}", p.accuracy_bonus)),
            HalfCoverPenalty => p.map_or("-".into(), |p| format!("-{}", p.half_cover_penalty)),
            FullCoverPenalty => p.map_or("-".into(), |p| format!("-{}", p.full_cover_penalty)),
            CritBonus => p.map_or("-".into(), |p| format!("{:+}", p.crit_bonus)),
            FlankCritBonus => p.map_or("-".into(), |p| format!("{:+}", p.flank_crit_bonus)),
            DamageMin | DamageMax => match p.map(|p| p.damage) {
                Some(DamageSource::Fixed { min, max }) => num(if self == DamageMin { min } else { max }),
                _ => "weapon".into(),
            },
            DamageBonus => p.map_or("-".into(), |p| format!("{:+}", p.damage_bonus)),
            AreaRadius => p.map_or("-".into(), |p| {
                if p.area_radius > 0.0 { format!("{:.1}", p.area_radius) } else { "single target".into() }
            }),
            CanMiss => p.map_or("-".into(), |p| if p.can_miss { "yes" } else { "never misses" }.into()),
            RangeFalloff => p.map_or("-".into(), |p| if p.range_falloff { "yes" } else { "no" }.into()),
            DefenseBonus => match def.effect {
                AbilityEffect::Defend { defense_bonus } => format!("+{defense_bonus}"),
                _ => "-".into(),
            },
            HealAmount => match def.effect {
                AbilityEffect::Heal { amount } => format!("{amount} HP"),
                _ => "-".into(),
            },
        }
    }

    /// Steps the value up (`dir > 0`) or down (`dir < 0`).
    pub fn adjust(self, def: &mut AbilityDef, dir: i32) {
        use EditField::*;
        use tactics_core::content::ApCost as Cost;
        let step = |v: &mut i32, by: i32, lo: i32, hi: i32| *v = (*v + by * dir).clamp(lo, hi);
        match self {
            ApCost => {
                // ends turn (0) <-> 1 AP <-> 2 AP <-> 3 AP
                let current = match def.cost {
                    Cost::EndsTurn => 0,
                    Cost::Points(n) => n as i32,
                };
                let next = (current + dir).clamp(0, 3);
                def.cost = if next == 0 { Cost::EndsTurn } else { Cost::Points(next as u8) };
            }
            AmmoCost => step(&mut def.ammo_cost, 1, 0, 10),
            Cooldown => def.cooldown = (def.cooldown as i32 + dir).clamp(0, 10) as u32,
            Charges => {
                def.charges = match (def.charges, dir > 0) {
                    (None, true) => Some(1),
                    (None, false) => None,
                    (Some(1), false) => None,
                    (Some(c), up) => Some(if up { (c + 1).min(9) } else { c - 1 }),
                }
            }
            Range => {
                if let AbilityRange::Fixed(r) = &mut def.range {
                    *r = (*r + 0.5 * dir as f32).clamp(0.5, 30.0);
                }
            }
            DefenseBonus => {
                if let AbilityEffect::Defend { defense_bonus } = &mut def.effect {
                    step(defense_bonus, 5, 0, 100);
                }
            }
            HealAmount => {
                if let AbilityEffect::Heal { amount } = &mut def.effect {
                    step(amount, 1, 1, 20);
                }
            }
            _ => {
                let Some(p) = def.effect.attack_profile_mut() else { return };
                match self {
                    BaseAccuracy => {
                        // "unit aim" (None) <-> 50..=100 in steps of 5.
                        p.base_accuracy = match p.base_accuracy {
                            None if dir > 0 => Some(50),
                            None => None,
                            Some(a) if a + 5 * dir < 50 => None,
                            Some(a) => Some((a + 5 * dir).min(100)),
                        }
                    }
                    AccuracyBonus => step(&mut p.accuracy_bonus, 5, -100, 100),
                    HalfCoverPenalty => step(&mut p.half_cover_penalty, 5, 0, 100),
                    FullCoverPenalty => step(&mut p.full_cover_penalty, 5, 0, 100),
                    CritBonus => step(&mut p.crit_bonus, 5, -100, 100),
                    FlankCritBonus => step(&mut p.flank_crit_bonus, 5, 0, 100),
                    DamageMin | DamageMax => {
                        if let DamageSource::Fixed { min, max } = &mut p.damage {
                            if self == DamageMin {
                                step(min, 1, 0, *max);
                            } else {
                                step(max, 1, *min, 30);
                            }
                        }
                    }
                    DamageBonus => step(&mut p.damage_bonus, 1, -10, 10),
                    AreaRadius => p.area_radius = (p.area_radius + 0.5 * dir as f32).clamp(0.0, 5.0),
                    CanMiss => p.can_miss = !p.can_miss,
                    RangeFalloff => p.range_falloff = !p.range_falloff,
                    _ => {}
                }
            }
        }
    }
}

/// Requests from the editor UI.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    Toggle,
    Prev,
    Next,
    Adjust(EditField, i32),
    Save,
}

#[derive(Resource, Default)]
pub struct EditorState {
    pub open: bool,
    /// Index into the ruleset's abilities (sorted by id).
    pub index: usize,
    /// Set when the panel layout must be rebuilt.
    dirty: bool,
}

#[derive(Component)]
struct EditorRoot;
#[derive(Component)]
struct EditorBody;
#[derive(Component)]
struct EditorTitle;
/// The value label of one field row.
#[derive(Component)]
struct FieldValue(EditField);

fn spawn_editor(mut commands: Commands) {
    commands.spawn((
        EditorRoot,
        // Above the HTML HUD.
        GlobalZIndex(10),
        ScrollPosition::default(),
        panel_node(Node {
            top: px(132),
            bottom: px(80),
            left: px(12),
            width: px(350),
            overflow: Overflow::scroll_y(),
            display: Display::None,
            ..default()
        }),
        panel(),
        children![
            (Text::new("MODULE EDITOR"), text(12.0, theme::UI_DIM)),
            (
                Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, margin: UiRect::vertical(px(6)), ..default() },
                children![
                    message_button("<", "", EditorCommand::Prev, (36.0, 36.0), ButtonLook::Normal),
                    (EditorTitle, Text::new(""), text(16.0, theme::UI_TEXT), Node { flex_grow: 1.0, margin: UiRect::horizontal(px(8)), ..default() }),
                    message_button(">", "", EditorCommand::Next, (36.0, 36.0), ButtonLook::Normal),
                ],
            ),
            (EditorBody, Node { flex_direction: FlexDirection::Column, ..default() }),
            (
                Node { flex_direction: FlexDirection::Row, margin: UiRect::top(px(8)), ..default() },
                children![
                    message_button("Save module", "", EditorCommand::Save, (120.0, 36.0), ButtonLook::Normal),
                    message_button("Close", "F1", EditorCommand::Toggle, (70.0, 36.0), ButtonLook::Normal),
                ],
            ),
        ],
    ));
}

fn editor_hotkey(keys: Res<ButtonInput<KeyCode>>, mut out: MessageWriter<EditorCommand>) {
    if keys.just_pressed(KeyCode::F1) {
        out.write(EditorCommand::Toggle);
    }
}

/// The ability the editor currently shows.
fn current_id(state: &BattleState, index: usize) -> Option<AbilityId> {
    state.battle.rules().abilities().nth(index).map(|d| d.id.clone())
}

fn apply_editor_commands(
    mut commands: MessageReader<EditorCommand>,
    mut editor: ResMut<EditorState>,
    mut state: ResMut<BattleState>,
    mut log: ResMut<LogMessages>,
    mut root: Single<&mut Node, With<EditorRoot>>,
) {
    let count = state.battle.rules().abilities().count().max(1);
    for command in commands.read() {
        match *command {
            EditorCommand::Toggle => {
                editor.open = !editor.open;
                root.display = if editor.open { Display::Flex } else { Display::None };
                // The HTML HUD hides its left-column panels via `data-if="!editor.open"`.
                editor.dirty = true;
            }
            EditorCommand::Prev => {
                editor.index = (editor.index + count - 1) % count;
                editor.dirty = true;
            }
            EditorCommand::Next => {
                editor.index = (editor.index + 1) % count;
                editor.dirty = true;
            }
            EditorCommand::Adjust(field, dir) => {
                if let Some(id) = current_id(&state, editor.index)
                    && let Some(def) = state.rules_mut().ability_mut(&id) {
                        field.adjust(def, dir);
                    }
            }
            EditorCommand::Save => {
                let module = DataModule::abilities_from(
                    state.battle.rules(),
                    ModuleInfo {
                        description: "Ability tuning saved from the in-game editor.".into(),
                        ..ModuleInfo::new("editor_tuning", "Editor Tuning")
                    },
                );
                let ron = module.to_ron();
                match content::save_user_module(&ron) {
                    Ok(path) => log.push(format!("Saved tuning to {path} (loads on next start)")),
                    Err(e) => {
                        info!("Editor module:\n{ron}");
                        log.push(format!("Not saved: {e}"));
                    }
                }
            }
        }
    }
}

/// Rebuilds the field rows when the selected ability changes.
fn rebuild_editor(
    mut commands: Commands,
    mut editor: ResMut<EditorState>,
    state: Res<BattleState>,
    body: Single<Entity, With<EditorBody>>,
    mut title: Single<&mut Text, With<EditorTitle>>,
) {
    if !editor.dirty || !editor.open {
        return;
    }
    editor.dirty = false;
    let rules = state.battle.rules();
    let count = rules.abilities().count();
    let Some(def) = rules.abilities().nth(editor.index.min(count.saturating_sub(1))) else { return };
    title.0 = format!("{} ({}/{count})", def.name, editor.index + 1);

    let used_by: Vec<_> = rules
        .classes()
        .filter(|c| c.abilities.contains(&def.id))
        .map(|c| c.name.clone())
        .collect();
    let header = format!(
        "id: {}   type: {}\nused by: {}\n{}",
        def.id,
        def.effect.kind_name(),
        if used_by.is_empty() { "-".into() } else { used_by.join(", ") },
        def.description
    );

    let mut body = commands.entity(*body);
    body.despawn_related::<Children>();
    body.with_children(|parent| {
        parent.spawn((Text::new(header), text(12.0, theme::UI_DIM), Node { margin: UiRect::bottom(px(6)), ..default() }));
        for field in fields_for(def) {
            parent.spawn((
                Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, margin: UiRect::top(px(3)), ..default() },
                children![
                    (Text::new(field.label()), text(12.0, theme::UI_TEXT), Node { width: px(150), ..default() }),
                    (FieldValue(field), Text::new(field.value(def)), text(13.0, theme::TRACER), Node { width: px(90), ..default() }),
                    message_button("-", "", EditorCommand::Adjust(field, -1), (34.0, 30.0), ButtonLook::Normal),
                    message_button("+", "", EditorCommand::Adjust(field, 1), (34.0, 30.0), ButtonLook::Normal),
                ],
            ));
        }
    });
}

/// Keeps value labels in sync after edits.
fn refresh_values(editor: Res<EditorState>, state: Res<BattleState>, mut values: Query<(&FieldValue, &mut Text)>) {
    if !editor.open || !state.is_changed() {
        return;
    }
    let Some(def) = current_id(&state, editor.index).and_then(|id| state.battle.ability(&id)) else { return };
    for (field, mut text) in &mut values {
        let v = field.0.value(def);
        if text.0 != v {
            text.0 = v;
        }
    }
}

/// Scrolls the editor with the mouse wheel or a one-finger drag over it.
fn scroll_editor(
    editor: Res<EditorState>,
    scroll: Res<AccumulatedMouseScroll>,
    touches: Res<Touches>,
    hover_map: Res<HoverMap>,
    parents: Query<&ChildOf>,
    mut root: Single<(Entity, &mut ScrollPosition), With<EditorRoot>>,
) {
    if !editor.open {
        return;
    }
    let (root_entity, scroll_pos) = &mut *root;
    // Is any pointer hovering the editor or one of its descendants?
    let over = hover_map.values().flat_map(|hits| hits.keys()).any(|e| {
        let mut current = *e;
        loop {
            if current == *root_entity {
                return true;
            }
            match parents.get(current) {
                Ok(parent) => current = parent.parent(),
                Err(_) => return false,
            }
        }
    });
    if !over {
        return;
    }
    let wheel = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y * 40.0,
        MouseScrollUnit::Pixel => scroll.delta.y,
    };
    let drag: f32 = touches.iter().map(|t| t.delta().y).sum();
    let dy = -(wheel + drag);
    if dy != 0.0 {
        // Bevy clamps the upper bound to the content size during layout.
        scroll_pos.0.y = (scroll_pos.0.y + dy).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_of_every_base_ability_adjusts_without_panicking() {
        let (rules, _) = content::build_ruleset();
        for def in rules.abilities() {
            let mut d = def.clone();
            for field in fields_for(def) {
                for dir in [1, 1, -1, -1, -1] {
                    field.adjust(&mut d, dir);
                    let _ = field.value(&d);
                }
            }
        }
    }

    #[test]
    fn cover_penalty_steps_by_five() {
        let rules = Ruleset::standard();
        let mut shoot = rules.ability(&"shoot".into()).unwrap().clone();
        EditField::HalfCoverPenalty.adjust(&mut shoot, -1);
        assert_eq!(EditField::HalfCoverPenalty.value(&shoot), "-15");
    }
}

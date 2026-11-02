//! Small, glanceable feedback right where the player is looking:
//!
//! * a **cursor tooltip** with the one thing that matters for what's under
//!   the pointer: hit chance and damage on an enemy, AP cost and cover on a
//!   tile, what to do next while aiming ("click again or Space to fire");
//! * a **banner** across the middle of the screen when a turn starts or the
//!   alarm sounds ([`Banner`] messages, sent by playback).
//!
//! The full odds breakdown stays in the HUD's hover panel.

use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use tactics_core::cover::best_cover_at;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::input::{pointer_over_ui, target_for};
use crate::state::{BattleState, Selection};
use crate::theme;

pub struct TooltipPlugin;

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Banner>()
            .add_systems(Startup, spawn_widgets)
            .add_systems(Update, (update_tooltip, show_banner).in_set(FrameSet::Present));
    }
}

/// Show `text` big in the middle of the screen for a moment.
#[derive(Message, Debug, Clone)]
pub struct Banner {
    pub text: String,
    pub color: Color,
}

#[derive(Component)]
struct CursorTip;

#[derive(Component)]
struct BannerText {
    age: f32,
}

const BANNER_TIME: f32 = 1.6;

fn spawn_widgets(mut commands: Commands) {
    commands.spawn((
        CursorTip,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(theme::UI_TEXT),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(px(8), px(4)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.05, 0.07, 0.85)),
        Visibility::Hidden,
        GlobalZIndex(50),
        Pickable::IGNORE,
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: percent(38),
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(40),
            Pickable::IGNORE,
        ))
        .with_child((
            BannerText { age: BANNER_TIME },
            Text::new(""),
            TextFont { font_size: FontSize::Px(44.0), ..default() },
            TextColor(Color::NONE),
            TextShadow::default(),
            Pickable::IGNORE,
        ));
}

/// The tooltip text for the current pointer position (empty = hide).
pub fn tooltip_text(state: &BattleState, selection: &Selection) -> String {
    let b = &state.battle;
    let (Some(sel), Some(hover)) = (selection.unit, selection.hovered) else { return String::new() };
    let Some(unit) = b.unit(sel) else { return String::new() };

    // Aimed / targeting: odds of the pending ability on the hovered (or aimed) target.
    if let Some(def) = selection.pending.as_ref().and_then(|a| b.ability(a)) {
        let mut target = target_for(b, def, hover);
        // While aimed, keep describing the aimed shot unless the pointer is on
        // another valid target (the camera reframing moves things around).
        if let Some(aim) = selection.aim {
            let other_ok = target.is_some_and(|t| {
                t != aim && b.validate(&Action::UseAbility { unit: sel, ability: def.id.clone(), target: t }).is_ok()
            });
            if !other_ok {
                target = Some(aim);
            }
        }
        let aimed = target.is_some() && target == selection.aim;
        let odds = target.and_then(|t| b.preview(sel, &def.id, t).ok()).map(|previews| {
            let enemies: Vec<String> = previews
                .iter()
                .filter_map(|(v, p)| b.unit(*v).map(|u| (u, p)))
                .map(|(u, p)| {
                    if def.effect.attack_profile().is_some_and(|a| a.area_radius > 0.0) {
                        format!("{} {}-{}", u.name, p.damage.0, p.damage.1)
                    } else {
                        format!("{}%  {}-{} dmg  crit {}%", p.hit_chance, p.damage.0, p.damage.1, p.crit_chance)
                    }
                })
                .collect();
            enemies.join("\n")
        });
        let action = Action::UseAbility { unit: sel, ability: def.id.clone(), target: target.unwrap_or(Target::None) };
        return match (target, b.validate(&action)) {
            (None, _) => format!("{}: not a valid target", def.name),
            (Some(_), Err(e)) => format!("{}: {e}", def.name),
            (Some(_), Ok(())) => {
                let what = odds.filter(|o| !o.is_empty()).unwrap_or_else(|| def.name.clone());
                if aimed {
                    format!("{what}\nClick again or Space to fire")
                } else {
                    format!("{what}\nClick to aim")
                }
            }
        };
    }

    // An enemy: primary attack odds.
    if let Some(enemy) = b.unit_at(hover).filter(|u| u.team != unit.team && b.is_revealed(u.id)) {
        let Some(def) = b.primary_attack(unit) else { return String::new() };
        return match b.preview(sel, &def.id, Target::Unit(enemy.id)) {
            Ok(p) if !p.is_empty() => {
                let p = &p[0].1;
                let cover = match p.cover.level {
                    CoverLevel::Full => "full cover",
                    CoverLevel::Half => "half cover",
                    CoverLevel::None if p.cover.flanked => "FLANKED",
                    CoverLevel::None => "no cover",
                };
                let usable = b.validate(&Action::UseAbility { unit: sel, ability: def.id.clone(), target: Target::Unit(enemy.id) });
                match usable {
                    Ok(()) => format!("{}%  {}-{} dmg  ({cover})\nClick to aim", p.hit_chance, p.damage.0, p.damage.1),
                    Err(e) => format!("{}: {e}", enemy.name),
                }
            }
            _ => enemy.name.clone(),
        };
    }

    // A tile in reach: what moving there costs and what it offers.
    if let Some(cost) = selection.range.as_ref().and_then(|r| r.cost(hover)).filter(|c| *c > 0) {
        let ap = b.move_cost_ap(sel, cost);
        let kind = if ap >= unit.action_points { "Dash" } else { "Move" };
        let cover = match best_cover_at(b.map(), hover) {
            CoverLevel::Full => "full cover",
            CoverLevel::Half => "half cover",
            CoverLevel::None => "no cover",
        };
        let height = b.map().elevation(hover);
        let high = if height > b.map().elevation(unit.pos) { "  high ground" } else { "" };
        return format!("{kind}  {ap} AP  {cover}{high}");
    }
    String::new()
}

#[allow(clippy::too_many_arguments)]
fn update_tooltip(
    state: Res<BattleState>,
    selection: Res<Selection>,
    window: Single<&Window>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    playback: Res<crate::playback::Playback>,
    director: Res<crate::camera_director::CameraDirector>,
    mut tip: Single<(&mut Text, &mut Node, &mut Visibility), With<CursorTip>>,
) {
    let (text, node, vis) = &mut *tip;
    let cursor = window.cursor_position();
    let active = player_can_act_now(&state, &playback, &director);
    let content = if active && cursor.is_some() && !pointer_over_ui(&hover_map, &ui_nodes) {
        tooltip_text(&state, &selection)
    } else {
        String::new()
    };
    if content.is_empty() {
        **vis = Visibility::Hidden;
        return;
    }
    let c = cursor.unwrap_or_default();
    // Keep it on screen: flip to the left/top near the edges.
    let (w, h) = (window.width(), window.height());
    let x = if c.x > w - 260.0 { c.x - 250.0 } else { c.x + 18.0 };
    let y = if c.y > h - 80.0 { c.y - 56.0 } else { c.y + 16.0 };
    node.left = px(x);
    node.top = px(y);
    if text.0 != content {
        text.0 = content;
    }
    **vis = Visibility::Inherited;
}

/// Same as the `player_can_act` run condition, as a plain function.
fn player_can_act_now(
    state: &BattleState,
    playback: &crate::playback::Playback,
    director: &crate::camera_director::CameraDirector,
) -> bool {
    !playback.is_busy() && !director.is_active() && !state.battle.is_over() && state.battle.active_team() == Team::Player
}

fn show_banner(
    time: Res<Time<Real>>,
    mut banners: MessageReader<Banner>,
    mut q: Single<(&mut BannerText, &mut Text, &mut TextColor)>,
) {
    let (banner, text, color) = &mut *q;
    if let Some(b) = banners.read().last() {
        banner.age = 0.0;
        text.0 = b.text.clone();
        color.0 = b.color;
    }
    if banner.age >= BANNER_TIME {
        color.0.set_alpha(0.0);
        return;
    }
    banner.age += time.delta_secs();
    // Quick fade in, hold, fade out.
    let k = banner.age / BANNER_TIME;
    let alpha = if k < 0.12 { k / 0.12 } else if k > 0.7 { (1.0 - k) / 0.3 } else { 1.0 };
    color.0.set_alpha(alpha.clamp(0.0, 1.0));
}

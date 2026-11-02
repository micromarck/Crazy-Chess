//! Small, glanceable feedback right where the player is looking:
//!
//! * a **cursor tooltip** with the one thing that matters for what's under
//!   the pointer: hit chance and damage on an enemy, AP cost and cover on a
//!   tile, what to do next while aiming ("click again or Space to fire");
//! * a **banner** across the middle of the screen when a turn starts or the
//!   alarm sounds ([`Banner`] messages, sent by playback).
//!
//! * **zone labels** ("EVAC", "OBJECTIVE"...) floating over marked zones.
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
            .add_systems(Update, (update_tooltip, show_banner, zone_labels).in_set(FrameSet::Present));
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

/// What a soldier would face after moving to `to`.
#[derive(Debug, Default, Clone)]
pub struct MoveIntel {
    /// Revealed enemies it could shoot from there.
    pub targets: Vec<UnitId>,
    /// Of those, the ones it would flank (they have cover, but not facing it).
    pub flanked: Vec<UnitId>,
    /// Enemies on overwatch that would get a shot during the walk.
    pub overwatchers: Vec<UnitId>,
}

/// Looks ahead at a move: targets and flanks from the destination, and
/// overwatch reaction fire along the way.
pub fn move_intel(battle: &Battle, unit: &Unit, to: GridPos, path: &[GridPos]) -> MoveIntel {
    use tactics_core::visibility::has_line_of_sight;
    let map = battle.map();
    let mut intel = MoveIntel::default();
    let reach = unit.weapon.max_range.min(unit.stats.sight);
    for e in battle.living(unit.team.opponent()).filter(|e| battle.is_revealed(e.id)) {
        if to.distance(e.pos) <= reach && has_line_of_sight(map, to, e.pos) {
            intel.targets.push(e.id);
            let c = tactics_core::cover::cover_against(map, e.pos, to);
            if c.flanked && best_cover_at(map, e.pos) != CoverLevel::None {
                intel.flanked.push(e.id);
            }
        }
        if let Some(ow) = e.status.overwatch.as_ref().and_then(|a| battle.ability(a)) {
            let r = battle.ability_range(e, ow).min(e.stats.sight);
            if e.weapon.ammo > 0 && path.iter().any(|p| e.pos.distance(*p) <= r && has_line_of_sight(map, e.pos, *p)) {
                intel.overwatchers.push(e.id);
            }
        }
    }
    intel
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
                        format!("{}  {}-{} dmg  crit {}%{}", odds_text(p), p.damage.0, p.damage.1, p.crit_chance, block_text(p))
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

    // A friendly soldier: who they are and how they're doing.
    if let Some(friend) = b.unit_at(hover).filter(|u| u.team == unit.team) {
        let morale = if friend.fearless {
            String::new()
        } else {
            format!("  {}", b.rules().combat.morale.level(friend.fear).name())
        };
        let hint = if friend.id == sel { "" } else if friend.can_act() { "\nClick to select" } else { "\nNo AP left" };
        return format!("{}  HP {}/{}  {} AP{morale}{hint}", friend.name, friend.hp, friend.stats.max_hp, friend.action_points);
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
                let mut notes = Vec::new();
                if enemy.status.overwatch.is_some() {
                    notes.push("on overwatch");
                }
                if enemy.status.suppressed_by.is_some() {
                    notes.push("suppressed");
                }
                if enemy.ai.dormant {
                    notes.push("unaware");
                }
                let morale = b.rules().combat.morale.level(enemy.fear);
                if !enemy.fearless && morale != tactics_core::content::FearLevel::Steady {
                    notes.push(morale.name());
                }
                let notes = if notes.is_empty() { String::new() } else { format!("\n{}", notes.join(", ")) };
                match usable {
                    Ok(()) => format!(
                        "{}  {}-{} dmg  ({cover}{}){notes}\nClick to aim",
                        odds_text(p),
                        p.damage.0,
                        p.damage.1,
                        block_text(p)
                    ),
                    Err(e) => format!("{}: {e}{notes}", enemy.name),
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
        let path = selection.range.as_ref().and_then(|r| r.path_to(hover)).unwrap_or_default();
        let intel = move_intel(b, unit, hover, &path);
        let mut lines = vec![format!("{kind}  {ap} AP  {cover}{high}")];
        match (intel.targets.len(), intel.flanked.len()) {
            (0, _) => {}
            (n, 0) => lines.push(format!("{n} target{} in sight", if n == 1 { "" } else { "s" })),
            (n, f) => lines.push(format!("{n} target{} in sight, {f} FLANKED", if n == 1 { "" } else { "s" })),
        }
        if !intel.overwatchers.is_empty() {
            lines.push(format!("! Overwatch: {} shot{} on the way", intel.overwatchers.len(), if intel.overwatchers.len() == 1 { "" } else { "s" }));
        }
        return lines.join("\n");
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
    mut tip: Single<(&mut Text, &mut Node, &mut Visibility, &mut TextColor), With<CursorTip>>,
) {
    let (text, node, vis, color) = &mut *tip;
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
    // Colour by the odds when the text leads with a hit chance.
    let odds = content.split('%').next().and_then(|n| n.trim().parse::<i32>().ok());
    color.0 = match odds {
        Some(p) if p >= 70 => theme::HEAL,
        Some(p) if p >= 40 => theme::TRACER,
        Some(_) => theme::TARGET,
        None => theme::UI_TEXT,
    };
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

/// A floating label over a marked zone.
#[derive(Component)]
struct ZoneLabelText(String);

fn zone_labels(
    mut commands: Commands,
    state: Res<BattleState>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::camera::CameraRig>>,
    window: Single<&Window>,
    mut labels: Query<(Entity, &ZoneLabelText, &mut Text, &mut Node, &mut Visibility, &mut TextColor)>,
) {
    let (w, h) = (window.width(), window.height());
    let (cam, cam_tr) = *camera;
    let battle = &state.battle;
    let zones: Vec<_> = battle.zones().iter().filter(|z| z.visible).collect();
    let mut seen = Vec::new();
    for (e, tag, mut text, mut node, mut vis, mut color) in &mut labels {
        let Some(z) = zones.iter().find(|z| z.tag == tag.0) else {
            commands.entity(e).despawn();
            continue;
        };
        seen.push(z.tag.clone());
        let (cx, cy) = z.center();
        let ground = crate::coords::tile_top(battle.map(), GridPos::new(cx.round() as i32, cy.round() as i32));
        let world = Vec3::new(
            (z.min.x + z.max.x) as f32 / 2.0 * crate::coords::TILE_SIZE,
            ground.y + 2.4,
            -((z.min.y + z.max.y) as f32 / 2.0) * crate::coords::TILE_SIZE,
        );
        // Off-screen (or behind the camera): pin the label to the screen
        // edge with an arrow pointing the way.
        let screen = cam.world_to_viewport(cam_tr, world).ok();
        let to_point = world - cam_tr.translation();
        let behind = to_point.dot(*cam_tr.forward()) < 0.0;
        // Area free of HUD panels (roster left, log/minimap right, bars
        // top and bottom) where edge labels may sit.
        let safe = Rect::new(260.0, 150.0, (w - 230.0).max(300.0), (h - 170.0).max(200.0));
        let label = match screen.filter(|_| !behind) {
            Some(p) if p.x > 60.0 && p.x < w - 60.0 && p.y > 40.0 && p.y < h - 110.0 => {
                node.left = px(p.x - 40.0);
                node.top = px(p.y - 12.0);
                z.label.clone()
            }
            other => {
                // Direction on screen (flip when behind).
                let centre = safe.center();
                let raw = other.unwrap_or_else(|| {
                    let right = cam_tr.right().dot(to_point);
                    let up = cam_tr.up().dot(to_point);
                    centre + Vec2::new(right, -up) * 100.0
                });
                let mut d = raw - centre;
                if behind {
                    d = -d;
                }
                let d = if d.length() < 1.0 { Vec2::Y } else { d };
                let half = safe.half_size();
                let k = (half.x / d.x.abs().max(1e-3)).min(half.y / d.y.abs().max(1e-3));
                let at = centre + d * k;
                node.left = px(at.x - 40.0);
                node.top = px(at.y - 12.0);
                let arrow = if d.x.abs() > d.y.abs() {
                    if d.x > 0.0 { ">" } else { "<" }
                } else if d.y > 0.0 {
                    "v"
                } else {
                    "^"
                };
                match arrow {
                    ">" | "v" => format!("{} {arrow}", z.label),
                    _ => format!("{arrow} {}", z.label),
                }
            }
        };
        *vis = Visibility::Inherited;
        if text.0 != label {
            text.0 = label;
        }
        color.0 = match z.kind.as_str() {
            "evac" => theme::ZONE_EVAC,
            "danger" => theme::ZONE_DANGER,
            _ => theme::ZONE_OBJECTIVE,
        };
    }
    for z in zones.iter().filter(|z| !seen.contains(&z.tag)) {
        commands.spawn((
            ZoneLabelText(z.tag.clone()),
            Text::new(z.label.clone()),
            TextFont { font_size: FontSize::Px(16.0), ..default() },
            TextColor(theme::ZONE_EVAC),
            TextShadow::default(),
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
            GlobalZIndex(5),
            Pickable::IGNORE,
        ));
    }
}

/// "4 rounds x 38%" for bursts and pellets, "38%" for single shots.
fn odds_text(p: &tactics_core::combat::ShotPreview) -> String {
    if p.rounds > 1 { format!("{} rounds x {}%", p.rounds, p.hit_chance) } else { format!("{}%", p.hit_chance) }
}

/// ", cover stops 30%" when cover can soak rounds that would hit.
fn block_text(p: &tactics_core::combat::ShotPreview) -> String {
    if p.cover_block > 0 { format!(", stops {}%", p.cover_block) } else { String::new() }
}

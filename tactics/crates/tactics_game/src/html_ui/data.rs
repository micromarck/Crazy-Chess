//! The data HTML layouts can bind to, rebuilt every frame from game state.
//!
//! **To expose something new to layouts, add a `v.set(...)` (value) or a
//! `v.lists.insert(...)` (list for `data-for`) here** and document it in
//! `UI.md`. Values are strings; booleans are `"true"`/`"false"`.

use std::collections::BTreeMap;

use bevy::prelude::*;
use tactics_core::prelude::*;

use super::{UiData, UiFlags};
use crate::missions::MissionCatalog;
use super::bind::UiValues;
use crate::camera_director::CameraDirector;
use crate::editor::EditorState;
use crate::hud::LogMessages;
use crate::input::target_for;
use crate::playback::Playback;
use crate::state::{BattleState, Selection};

/// Rebuilds [`UiData`] (only marks it changed when something differs).
#[allow(clippy::too_many_arguments)] // Bevy systems take their inputs as parameters.
pub fn collect_ui_data(
    state: Res<BattleState>,
    selection: Res<Selection>,
    log: Res<LogMessages>,
    playback: Res<Playback>,
    editor: Res<EditorState>,
    director: Res<CameraDirector>,
    catalog: Res<MissionCatalog>,
    flags: Res<UiFlags>,
    mut data: ResMut<UiData>,
) {
    let mut v = UiValues::default();
    let b = &state.battle;
    let bool_str = |x: bool| if x { "true" } else { "false" };

    // Turn / battle.
    let player_turn = b.active_team() == Team::Player;
    v.set("turn.round", b.round());
    v.set("turn.team", if player_turn { "XCOM" } else { "ALIEN" });
    v.set("turn.label", if player_turn { "XCOM TURN" } else { "ALIEN ACTIVITY" });
    v.set("turn.is_player", bool_str(player_turn));
    v.set("squad.alive", b.living(Team::Player).count());
    v.set("enemies.alive", b.living(Team::Enemy).count());
    // "Over" only once the final animations have played.
    let over = b.is_over() && !playback.is_busy();
    v.set("battle.over", bool_str(over));
    v.set("battle.won", bool_str(over && b.winner() == Some(Team::Player)));
    v.set(
        "battle.result",
        match (over, b.winner()) {
            (true, Some(Team::Player)) => "MISSION ACCOMPLISHED",
            (true, _) => "MISSION FAILED",
            _ => "",
        },
    );
    v.set("battle.seed", format!("{:#x}", state.seed));
    v.set("busy", bool_str(playback.is_busy() || director.is_active()));

    // Selected unit.
    let unit = selection.unit.and_then(|id| b.unit(id));
    v.set("unit.selected", bool_str(unit.is_some()));
    if let Some(u) = unit {
        v.set("unit.name", &u.name);
        v.set("unit.class", b.rules().class(&u.class).map_or(u.class.as_str(), |c| c.name.as_str()));
        v.set("unit.hp", u.hp);
        v.set("unit.max_hp", u.stats.max_hp);
        v.set("unit.hp_percent", (100 * u.hp / u.stats.max_hp.max(1)).clamp(0, 100));
        v.set("unit.ap", u.action_points);
        v.set("unit.max_ap", b.rules().combat.action_points);
        v.set("unit.weapon", &u.weapon.name);
        v.set("unit.ammo", u.weapon.ammo);
        v.set("unit.clip", u.weapon.clip_size);
        v.set("unit.aim", u.stats.aim);
        v.set("unit.mobility", u.stats.mobility / 2);
        v.set("unit.defense", u.stats.defense);
        let mut status = Vec::new();
        if u.status.overwatch.is_some() {
            status.push("Overwatch".to_string());
        }
        if u.status.defense_bonus > 0 {
            status.push(format!("Defending +{}", u.status.defense_bonus));
        }
        v.set("unit.status", status.join(", "));
    }

    // Targeting / hover.
    let pending = selection.pending.as_ref().and_then(|a| b.ability(a));
    v.set("targeting", bool_str(pending.is_some()));
    v.set("targeting.name", pending.map_or("", |d| d.name.as_str()));
    v.set("hover.info", hover_info(&state, &selection));

    // Log.
    let lines: Vec<String> = log.lines().cloned().collect();
    v.set("log.text", lines.join("\n"));
    v.set("log.last", lines.last().cloned().unwrap_or_default());
    v.lists.insert("log".into(), lines.iter().map(|l| item(&[("text", l.clone())])).collect());

    // Missions.
    let current = state.mission.as_deref().and_then(|id| catalog.get(id));
    v.set("mission.active", bool_str(current.is_some()));
    v.set("mission.name", current.map_or("Skirmish", |m| m.def.name.as_str()));
    v.set("mission.description", current.map_or("", |m| m.def.description.as_str()));
    v.set("missions.ready", bool_str(catalog.ready));
    v.lists.insert(
        "missions".into(),
        catalog
            .missions
            .iter()
            .map(|m| {
                item(&[
                    ("id", m.def.id.clone()),
                    ("name", m.def.name.clone()),
                    ("description", m.def.description.clone()),
                    ("file", m.file.clone()),
                    ("state", if state.mission.as_deref() == Some(m.def.id.as_str()) { "current" } else { "" }.into()),
                ])
            })
            .collect(),
    );

    // Layout flags (toggle:<name>).
    for (name, on) in &flags.0 {
        v.set(&format!("ui.{name}"), bool_str(*on));
    }

    // Misc toggles.
    v.set("editor.open", bool_str(editor.open));
    v.set("cinematics", bool_str(director.enabled));

    // Lists.
    v.lists.insert("abilities".into(), unit.map(|u| abilities(b, u, &selection)).unwrap_or_default());
    v.lists.insert(
        "squad".into(),
        b.living(Team::Player)
            .map(|u| {
                item(&[
                    ("id", u.id.0.to_string()),
                    ("name", u.name.clone()),
                    ("hp", u.hp.to_string()),
                    ("max_hp", u.stats.max_hp.to_string()),
                    ("hp_percent", (100 * u.hp / u.stats.max_hp.max(1)).clamp(0, 100).to_string()),
                    ("ap", u.action_points.to_string()),
                    ("state", if Some(u.id) == selection.unit { "selected" } else if u.can_act() { "ready" } else { "done" }.into()),
                ])
            })
            .collect(),
    );

    if data.values != v {
        data.values = v;
    }
}

fn item(fields: &[(&str, String)]) -> BTreeMap<String, String> {
    fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// The selected unit's abilities: `id name description key cost charges cooldown state`.
/// `state` is `active` (being targeted), `ready` or `disabled`.
fn abilities(b: &Battle, unit: &Unit, selection: &Selection) -> Vec<BTreeMap<String, String>> {
    unit.abilities
        .iter()
        .enumerate()
        .filter_map(|(slot, id)| {
            let def = b.ability(id)?;
            // Target errors are fine (the target is picked next); cost errors disable.
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
            let state = if selection.pending.as_ref() == Some(id) {
                "active"
            } else if usable {
                "ready"
            } else {
                "disabled"
            };
            let cost = match def.cost {
                ApCost::Points(n) => format!("{n} AP"),
                ApCost::EndsTurn => "ends turn".into(),
            };
            Some(item(&[
                ("id", id.to_string()),
                ("name", def.name.clone()),
                ("description", def.description.clone()),
                ("key", (slot + 1).to_string()),
                ("cost", cost),
                ("charges", b.charges_left(unit, def).map_or(String::new(), |c| format!("x{c}"))),
                ("cooldown", match unit.cooldown(id) {
                    0 => String::new(),
                    n => format!("cd {n}"),
                }),
                ("state", state.into()),
            ]))
        })
        .collect()
}

/// Odds breakdown for the hovered target, or move cost for a hovered tile.
pub fn hover_info(state: &BattleState, selection: &Selection) -> String {
    let b = &state.battle;
    let mut s = String::new();
    if let (Some(sel), Some(hover)) = (selection.unit, selection.hovered) {
        // Preview the pending ability, or the primary attack on a hovered enemy.
        let ability = selection.pending.clone().or_else(|| {
            b.unit_at(hover)
                .filter(|u| u.team == Team::Enemy)
                .and_then(|_| b.unit(sel).and_then(|u| b.primary_attack(u)).map(|d| d.id.clone()))
        });
        if let Some(def) = ability.as_ref().and_then(|a| b.ability(a)) {
            s = format!("[{}]\n", def.name);
            match target_for(b, def, hover).map(|t| b.preview(sel, &def.id, t)) {
                Some(Ok(previews)) if !previews.is_empty() => {
                    for (victim, p) in &previews {
                        if let Some(v) = b.unit(*victim) {
                            s.push_str(&format!(
                                "{}  HP {}/{}\nHIT {}%   CRIT {}%   DMG {}-{}\n",
                                v.name, v.hp, v.stats.max_hp, p.hit_chance, p.crit_chance, p.damage.0, p.damage.1
                            ));
                            for (label, value) in &p.breakdown {
                                s.push_str(&format!("  {label}: {value:+}\n"));
                            }
                        }
                    }
                }
                Some(Ok(_)) => s.push_str(&def.description),
                Some(Err(e)) => s.push_str(&e.to_string()),
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
    s.trim_end().to_string()
}

//! Squad loadout editing from the HUD.
//!
//! The player's squad is the ruleset's `"player"` [`SquadDef`]; each member
//! has a loadout preset and individual overrides (see
//! `tactics_core::content::gear`). The Loadout panel cycles them with
//! `gear:<command>:<member>[:<slot>]` UI commands; changes apply when the
//! mission (re)starts (`apply_loadout`).
//!
//! | command | effect |
//! |---|---|
//! | `gear:preset:<i>` | next preset (or none) for member `i`; clears overrides |
//! | `gear:weapon:<i>` | next weapon this class may use |
//! | `gear:armor:<i>` | next armor (or none) |
//! | `gear:item:<i>:<slot>` | next utility item (or none) in slot 0/1 |

use std::collections::BTreeMap;

use tactics_core::content::{ItemSlot, Loadout, Ruleset, SquadMember, UTILITY_SLOTS};

const SQUAD: &str = "player";

/// Applies a `gear:...` command. Returns a message for the log.
pub fn command(arg: &str, rules: &mut Ruleset) -> Option<String> {
    let mut parts = arg.split(':').map(str::trim);
    let what = parts.next()?;
    let index: usize = parts.next()?.parse().ok()?;
    let slot: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let snapshot = rules.clone();
    let member = rules.squad_mut(SQUAD)?.members.get_mut(index)?;
    let current = snapshot.gear_for(&member.class, member.loadout.as_deref(), &member.gear);

    match what {
        "preset" => {
            let ids: Vec<Option<String>> = std::iter::once(None)
                .chain(
                    snapshot
                        .loadouts()
                        .filter(|l| snapshot.check_gear(&member.class, &snapshot.gear_for(&member.class, Some(&l.id), &Loadout::default())).is_empty())
                        .map(|l| Some(l.id.clone())),
                )
                .collect();
            member.loadout = next(&ids, &member.loadout);
            member.gear = Loadout::default();
        }
        "weapon" => {
            let ids: Vec<Option<String>> = snapshot
                .weapons()
                .filter(|w| w.classes.is_empty() || w.classes.contains(&member.class))
                .filter(|w| is_player_weapon(&snapshot, &w.id))
                .map(|w| Some(w.id.clone()))
                .collect();
            member.gear.weapon = next(&ids, &current.weapon);
        }
        "armor" => {
            let ids = item_choices(&snapshot, member, ItemSlot::Armor);
            member.gear.armor = next(&ids, &current.armor);
            if member.gear.armor.is_none() {
                // "None" must override a preset's armor: use an explicit empty set.
                member.gear.armor = Some(String::new());
            }
        }
        "item" if slot < UTILITY_SLOTS => {
            let ids = item_choices(&snapshot, member, ItemSlot::Utility);
            let mut items: Vec<Option<String>> = (0..UTILITY_SLOTS).map(|i| current.items.get(i).cloned()).collect();
            items[slot] = next(&ids, &items[slot]);
            member.gear.items = items.into_iter().flatten().collect();
            if member.gear.items.is_empty() {
                member.gear.items = vec![String::new()];
            }
        }
        _ => return None,
    }
    let name = member.name.clone();
    let class = member.class.clone();
    let gear = rules.squad_mut(SQUAD).map(|s| s.members[index].clone())?;
    let resolved = normalize(rules.gear_for(&class, gear.loadout.as_deref(), &gear.gear));
    let problems = rules.check_gear(&class, &resolved);
    Some(if problems.is_empty() {
        format!("{name}: {} (restart the mission to apply)", describe(rules, &resolved))
    } else {
        format!("{name}: {}", problems.join(", "))
    })
}

/// Empty ids stand for "explicitly nothing" (`Ruleset::gear_for` already
/// drops them; kept for gear built by hand).
pub fn normalize(mut gear: Loadout) -> Loadout {
    if gear.armor.as_deref() == Some("") {
        gear.armor = None;
    }
    gear.items.retain(|i| !i.is_empty());
    gear
}

/// Weapons the enemy uses (plasma pistol, drone laser) aren't offered.
fn is_player_weapon(rules: &Ruleset, id: &str) -> bool {
    rules.classes().any(|c| c.weapon == id && c.model == "soldier") || rules.loadouts().any(|l| l.gear.weapon.as_deref() == Some(id))
}

fn item_choices(rules: &Ruleset, member: &SquadMember, slot: ItemSlot) -> Vec<Option<String>> {
    std::iter::once(None)
        .chain(
            rules
                .items()
                .filter(|i| i.slot == slot && (i.classes.is_empty() || i.classes.contains(&member.class)))
                .map(|i| Some(i.id.clone())),
        )
        .collect()
}

fn next(options: &[Option<String>], current: &Option<String>) -> Option<String> {
    let cur = current.as_ref().filter(|s| !s.is_empty()).cloned();
    let i = options.iter().position(|o| *o == cur).map_or(0, |i| (i + 1) % options.len().max(1));
    options.get(i).cloned().flatten()
}

fn item_name(rules: &Ruleset, id: Option<&String>) -> String {
    match id.filter(|s| !s.is_empty()) {
        None => "-".into(),
        Some(id) => rules.item(id).map_or(id.clone(), |i| i.name.clone()),
    }
}

/// One-line summary: "Shotgun, Plated Armor, Frag Grenade + Combat Stims".
pub fn describe(rules: &Ruleset, gear: &Loadout) -> String {
    let weapon = gear.weapon.as_deref().and_then(|w| rules.weapon(w)).map_or("-".to_string(), |w| w.name.clone());
    let items: Vec<String> = gear.items.iter().map(|i| item_name(rules, Some(i))).collect();
    format!("{weapon}, {}, {}", item_name(rules, gear.armor.as_ref()), if items.is_empty() { "no items".into() } else { items.join(" + ") })
}

/// Rows for the HUD's `loadout` list.
pub fn rows(rules: &Ruleset) -> Vec<BTreeMap<String, String>> {
    let Some(squad) = rules.squad(SQUAD) else { return Vec::new() };
    squad
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let gear = normalize(rules.gear_for(&m.class, m.loadout.as_deref(), &m.gear));
            let class = rules.class(&m.class);
            // Stats with items applied.
            let mut stats = class.map(|c| c.stats).unwrap_or_default();
            for item in gear.armor.iter().chain(gear.items.iter()).filter_map(|id| rules.item(id)) {
                stats.max_hp += item.stats.max_hp;
                stats.mobility = (stats.mobility as i32 + item.stats.mobility).max(2) as u32;
                stats.aim += item.stats.aim;
                stats.defense += item.stats.defense;
                stats.bravery += item.stats.bravery;
            }
            let preset = m.loadout.as_deref().and_then(|p| rules.loadout(p)).map_or("Custom".to_string(), |l| {
                if m.gear.is_empty() { l.name.clone() } else { format!("{} (modified)", l.name) }
            });
            let weapon = gear.weapon.as_deref().and_then(|w| rules.weapon(w)).map_or("-".into(), |w| w.name.clone());
            let field = |k: &str, v: String| (k.to_string(), v);
            BTreeMap::from([
                field("index", i.to_string()),
                field("name", m.name.clone()),
                field("class", class.map_or(m.class.clone(), |c| c.name.clone())),
                field("preset", preset),
                field("weapon", weapon),
                field("armor", item_name(rules, gear.armor.as_ref())),
                field("item0", item_name(rules, gear.items.first())),
                field("item1", item_name(rules, gear.items.get(1))),
                field(
                    "stats",
                    format!("HP {}  Mob {}  Aim {}  Def {}  Brave {}", stats.max_hp, stats.mobility / 2, stats.aim, stats.defense, stats.bravery),
                ),
                field("problems", rules.check_gear(&m.class, &gear).join(", ")),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_gear_stays_legal_and_applies() {
        let (mut rules, _) = crate::content::build_ruleset();
        for cmd in ["preset:0", "weapon:1", "armor:1", "armor:1", "item:1:0", "item:1:1", "item:2:0", "preset:2", "weapon:2"] {
            for _ in 0..7 {
                let msg = command(cmd, &mut rules).expect(cmd);
                let squad = rules.squad("player").unwrap();
                for (i, m) in squad.members.iter().enumerate() {
                    let gear = normalize(rules.gear_for(&m.class, m.loadout.as_deref(), &m.gear));
                    assert!(rules.check_gear(&m.class, &gear).is_empty(), "{cmd} member {i}: {msg}");
                }
            }
        }
        // The edited squad deploys with its gear.
        let m = rules.squad("player").unwrap().members[1].clone();
        let mut b = tactics_core::battle::Battle::new(tactics_core::grid::Map::from_ascii("...."), rules, 1);
        let id = b
            .spawn_equipped(&m.class, "x", tactics_core::unit::Team::Player, tactics_core::grid::GridPos::new(0, 0), m.loadout.as_deref(), &m.gear)
            .unwrap();
        assert!(!rows(b.rules()).is_empty());
        assert!(b.unit(id).is_some());
    }
}

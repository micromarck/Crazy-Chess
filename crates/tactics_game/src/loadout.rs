//! Squad loadout editing from the HUD.
//!
//! The player's squad is the ruleset's `"player"` [`SquadDef`]; each member
//! has a class, a loadout preset and individual overrides (see
//! `tactics_core::content::gear`). The Loadout panel shows one soldier at a
//! time (picked from the list) with a row per choice and arrows to step
//! through the options; changes apply when the mission (re)starts
//! (`apply_loadout`).
//!
//! | command | effect |
//! |---|---|
//! | `gear:select:<i>` | show member `i` |
//! | `gear:class:<i>[:<dir>]` | next (`1`) / previous (`-1`) class: soldiers retrain as soldier classes, frames refit as frames; gear that doesn't fit goes back to the class's default |
//! | `gear:preset:<i>[:<dir>]` | next / previous quick loadout (or none); clears the hand-picked changes |
//! | `gear:weapon:<i>[:<dir>]` | next / previous weapon this class may use |
//! | `gear:armor:<i>[:<dir>]` | next / previous armour (or none) |
//! | `gear:item:<i>:<slot>[:<dir>]` | next / previous utility item (or none) in slot 0/1 |
//! | `gear:reset:<i>` | back to the class's default gear |

use std::collections::BTreeMap;

use tactics_core::content::{ItemSlot, Loadout, Ruleset, SquadMember, UTILITY_SLOTS};

const SQUAD: &str = "player";

/// Applies a `gear:...` command. Returns a message for the log (`None`: bad
/// command). `selected` is the member the panel shows (`gear:select`).
pub fn command(arg: &str, rules: &mut Ruleset, selected: &mut usize) -> Option<String> {
    let mut parts = arg.split(':').map(str::trim);
    let what = parts.next()?;
    let index: usize = parts.next()?.parse().ok()?;
    let rest: Vec<i64> = parts.filter_map(|s| s.parse().ok()).collect();
    // Items take a slot before the direction.
    let (slot, dir) = if what == "item" { (rest.first().copied().unwrap_or(0) as usize, rest.get(1).copied().unwrap_or(1)) } else { (0, rest.first().copied().unwrap_or(1)) };
    let back = dir < 0;
    let snapshot = rules.clone();
    let member = rules.squad_mut(SQUAD)?.members.get_mut(index)?;
    let current = snapshot.gear_for(&member.class, member.loadout.as_deref(), &member.gear);

    match what {
        "select" => {
            *selected = index;
            return Some(String::new());
        }
        "class" => {
            let classes = crate::roster::family(&member.class, &snapshot);
            if classes.is_empty() {
                return None;
            }
            let i = classes.iter().position(|c| *c == member.class).unwrap_or(0);
            let n = classes.len();
            member.class = classes[if back { (i + n - 1) % n } else { (i + 1) % n }].to_string();
            // Gear that doesn't fit the new class: back to its default.
            if !snapshot.check_gear(&member.class, &snapshot.gear_for(&member.class, member.loadout.as_deref(), &member.gear)).is_empty() {
                member.loadout = None;
                member.gear = Loadout::default();
            }
        }
        "reset" => {
            member.loadout = None;
            member.gear = Loadout::default();
        }
        "preset" => {
            let ids: Vec<Option<String>> = std::iter::once(None)
                .chain(
                    snapshot
                        .loadouts()
                        .filter(|l| snapshot.check_gear(&member.class, &snapshot.gear_for(&member.class, Some(&l.id), &Loadout::default())).is_empty())
                        .map(|l| Some(l.id.clone())),
                )
                .collect();
            member.loadout = step(&ids, &member.loadout, back);
            member.gear = Loadout::default();
        }
        "weapon" => {
            let ids = weapon_choices(&snapshot, &member.class);
            member.gear.weapon = step(&ids, &current.weapon, back);
        }
        "armor" => {
            let ids = item_choices(&snapshot, member, ItemSlot::Armor);
            member.gear.armor = step(&ids, &current.armor, back);
            if member.gear.armor.is_none() {
                // "None" must override a preset's armor: use an explicit empty set.
                member.gear.armor = Some(String::new());
            }
        }
        "item" if slot < UTILITY_SLOTS => {
            let ids = item_choices(&snapshot, member, ItemSlot::Utility);
            let mut items: Vec<Option<String>> = (0..UTILITY_SLOTS).map(|i| current.items.get(i).cloned()).collect();
            items[slot] = step(&ids, &items[slot], back);
            member.gear.items = items.into_iter().flatten().collect();
            if member.gear.items.is_empty() {
                member.gear.items = vec![String::new()];
            }
        }
        _ => return None,
    }
    *selected = index;
    let name = member.name.clone();
    let class = member.class.clone();
    let gear = rules.squad_mut(SQUAD).map(|s| s.members[index].clone())?;
    let resolved = normalize(rules.gear_for(&class, gear.loadout.as_deref(), &gear.gear));
    let problems = rules.check_gear(&class, &resolved);
    let class_name = rules.class(&class).map_or(class.clone(), |c| c.name.clone());
    Some(if problems.is_empty() {
        format!("{name} ({class_name}): {} (restart the mission to apply)", describe(rules, &resolved))
    } else {
        format!("{name}: {}", problems.join(", "))
    })
}

/// Weapons `class` may carry (player weapons only).
fn weapon_choices(rules: &Ruleset, class: &str) -> Vec<Option<String>> {
    rules
        .weapons()
        .filter(|w| rules.check_gear(class, &Loadout { weapon: Some(w.id.clone()), ..Default::default() }).is_empty())
        .filter(|w| is_player_weapon(rules, &w.id))
        .map(|w| Some(w.id.clone()))
        .collect()
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

/// Weapons the enemy uses (plasma pistol, drone laser) aren't offered:
/// only those made for named classes, used by soldiers or in a loadout.
fn is_player_weapon(rules: &Ruleset, id: &str) -> bool {
    rules.weapon(id).is_some_and(|w| !w.classes.is_empty())
        || rules.classes().any(|c| c.weapon == id && c.model == "soldier")
        || rules.loadouts().any(|l| l.gear.weapon.as_deref() == Some(id))
}

fn item_choices(rules: &Ruleset, member: &SquadMember, slot: ItemSlot) -> Vec<Option<String>> {
    std::iter::once(None)
        .chain(
            rules
                .items()
                .filter(|i| i.slot == slot)
                .filter(|i| {
                    let gear = match slot {
                        ItemSlot::Armor => Loadout { armor: Some(i.id.clone()), ..Default::default() },
                        ItemSlot::Utility => Loadout { items: vec![i.id.clone()], ..Default::default() },
                    };
                    rules.check_gear(&member.class, &gear).is_empty()
                })
                .map(|i| Some(i.id.clone())),
        )
        .collect()
}

/// The option after (or with `back`, before) `current`, wrapping round.
fn step(options: &[Option<String>], current: &Option<String>, back: bool) -> Option<String> {
    let cur = current.as_ref().filter(|s| !s.is_empty()).cloned();
    let n = options.len().max(1);
    let i = match options.iter().position(|o| *o == cur) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None => 0,
    };
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

/// Rows for the HUD's `loadout` list (the squad, `selected` highlighted).
pub fn rows(rules: &Ruleset, selected: usize) -> Vec<BTreeMap<String, String>> {
    let Some(squad) = rules.squad(SQUAD) else { return Vec::new() };
    squad
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let field = |k: &str, v: String| (k.to_string(), v);
            BTreeMap::from([
                field("index", i.to_string()),
                field("name", m.name.clone()),
                field("class", rules.class(&m.class).map_or(m.class.clone(), |c| c.name.clone())),
                field("state", if i == selected.min(squad.members.len().saturating_sub(1)) { "selected".into() } else { String::new() }),
            ])
        })
        .collect()
}

/// The selected member's page (`lo.*` values): class, gear with a short
/// description of each, resulting stats and abilities.
pub fn detail(rules: &Ruleset, selected: usize) -> Vec<(String, String)> {
    let Some(squad) = rules.squad(SQUAD) else { return Vec::new() };
    if squad.members.is_empty() {
        return Vec::new();
    }
    let index = selected.min(squad.members.len() - 1);
    let m = &squad.members[index];
    let gear = normalize(rules.gear_for(&m.class, m.loadout.as_deref(), &m.gear));
    let class = rules.class(&m.class);
    // Stats with items applied.
    let mut stats = class.map(|c| c.stats).unwrap_or_default();
    let mut sight_percent = 0;
    for item in gear.armor.iter().chain(gear.items.iter()).filter_map(|id| rules.item(id)) {
        sight_percent += item.stats.sight_percent;
        stats.sight += item.stats.sight;
        stats.max_hp += item.stats.max_hp;
        stats.mobility = (stats.mobility as i32 + item.stats.mobility).max(2) as u32;
        stats.aim += item.stats.aim;
        stats.defense += item.stats.defense;
        stats.bravery += item.stats.bravery;
    }
    let machine = class.is_some_and(|c| c.heat.is_some());
    let vital = if machine { format!("Heat limit {}", stats.max_hp) } else { format!("HP {}", stats.max_hp) };
    let sight = stats.sight * (1.0 + sight_percent as f32 / 100.0);
    let stats_line =
        format!("{vital}   Move {}   Aim {}   Defence {}   Sight {:.0}", stats.mobility / 2, stats.aim, stats.defense, sight);
    let class_info = match class {
        Some(c) if machine => format!("Robot frame - runs on heat (cools {} a turn)", c.heat.map_or(0, |h| h.cooling)),
        Some(_) => "Soldier".to_string(),
        None => String::new(),
    };
    let weapon = gear.weapon.as_deref().and_then(|w| rules.weapon(w));
    let weapon_info = weapon.map_or(String::new(), |w| {
        let shots = if w.pellets > 1 {
            format!("{} pellets", w.pellets)
        } else if w.burst > 1 {
            format!("{} rounds", w.burst)
        } else {
            "single shot".into()
        };
        let mut s = if w.cone > 0.0 {
            format!("{}-{} dmg, {:.0} degree cone of fire to {:.0} tiles, {} bursts", w.damage_min, w.damage_max, w.cone, w.max_range, w.clip_size)
        } else {
            format!("{}-{} dmg, {shots}, range {:.0}, clip {}", w.damage_min, w.damage_max, w.max_range, w.clip_size)
        };
        if w.ignores_cover {
            s += ", ignores cover";
        }
        if machine && w.heat > 0 {
            s += &format!(", +{} heat", w.heat);
        }
        s
    });
    let item_info = |id: Option<&String>| id.filter(|s| !s.is_empty()).and_then(|i| rules.item(i)).map_or("nothing".to_string(), |i| i.description.clone());
    // Everything this soldier will be able to do.
    let mut abilities: Vec<String> = Vec::new();
    for a in class.map(|c| c.abilities.clone()).unwrap_or_default().iter().chain(gear.armor.iter().chain(gear.items.iter()).filter_map(|i| rules.item(i)).flat_map(|i| i.abilities.iter())) {
        if let Some(d) = rules.ability(a)
            && !abilities.contains(&d.name)
        {
            abilities.push(d.name.clone());
        }
    }
    let preset = m.loadout.as_deref().and_then(|p| rules.loadout(p));
    let preset_name = match (preset, m.gear.is_empty()) {
        (Some(l), true) => l.name.clone(),
        (Some(l), false) => format!("{} (changed)", l.name),
        (None, true) => "Class default".into(),
        (None, false) => "Custom".into(),
    };
    let field = |k: &str, v: String| (format!("lo.{k}"), v);
    vec![
        field("index", index.to_string()),
        field("name", m.name.clone()),
        field("class", class.map_or(m.class.clone(), |c| c.name.clone())),
        field("class_info", class_info),
        field("weapon", weapon.map_or("-".into(), |w| w.name.clone())),
        field("weapon_info", weapon_info),
        field("armor", item_name(rules, gear.armor.as_ref())),
        field("armor_info", item_info(gear.armor.as_ref())),
        field("item0", item_name(rules, gear.items.first())),
        field("item0_info", item_info(gear.items.first())),
        field("item1", item_name(rules, gear.items.get(1))),
        field("item1_info", item_info(gear.items.get(1))),
        field("preset", preset_name),
        field("preset_info", preset.map_or("The class's standard kit".to_string(), |l| l.description.clone())),
        field("stats", stats_line),
        field("abilities", abilities.join(", ")),
        field("problems", rules.check_gear(&m.class, &gear).join(", ")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_gear_stays_legal_and_applies() {
        let (mut rules, _) = crate::content::build_ruleset();
        for cmd in ["preset:0", "weapon:1", "armor:1", "armor:1", "item:1:0", "item:1:1", "item:2:0", "preset:2", "weapon:2"] {
            for _ in 0..7 {
                let msg = command(cmd, &mut rules, &mut 0).expect(cmd);
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
        assert!(!rows(b.rules(), 0).is_empty());
        assert!(b.unit(id).is_some());
    }

    /// The class can be changed from the loadout (within its family), gear
    /// stays legal, arrows go both ways, and the detail page fills in.
    #[test]
    fn class_changes_and_both_directions() {
        let (mut rules, _) = crate::content::build_ruleset();
        let mut sel = 0;
        let class0 = rules.squad("player").unwrap().members[0].class.clone();
        let soldiers = crate::roster::family(&class0, &rules);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..soldiers.len() {
            command("class:0:1", &mut rules, &mut sel).unwrap();
            let m = &rules.squad("player").unwrap().members[0];
            assert!(soldiers.contains(&m.class.as_str()), "stays a soldier: {}", m.class);
            assert!(rules.check_gear(&m.class, &normalize(rules.gear_for(&m.class, m.loadout.as_deref(), &m.gear))).is_empty());
            seen.insert(m.class.clone());
        }
        assert_eq!(seen.len(), soldiers.len(), "every soldier class reachable");
        assert_eq!(rules.squad("player").unwrap().members[0].class, class0, "full circle");
        // Back undoes forward.
        command("weapon:1:1", &mut rules, &mut sel).unwrap();
        let w = rules.squad("player").unwrap().members[1].gear.weapon.clone();
        command("weapon:1:1", &mut rules, &mut sel).unwrap();
        command("weapon:1:-1", &mut rules, &mut sel).unwrap();
        assert_eq!(rules.squad("player").unwrap().members[1].gear.weapon, w);
        assert_eq!(sel, 1, "editing a soldier shows them");
        command("select:2", &mut rules, &mut sel).unwrap();
        assert_eq!(sel, 2);
        let page: std::collections::BTreeMap<String, String> = detail(&rules, sel).into_iter().collect();
        assert_eq!(page["lo.index"], "2");
        assert!(!page["lo.weapon_info"].is_empty() && !page["lo.abilities"].is_empty() && page["lo.problems"].is_empty());
        // The class change reaches the saved roster.
        let mut roster = crate::roster::Roster::default();
        roster.seed_from(&rules);
        command("class:0:1", &mut rules, &mut sel).unwrap();
        roster.pull_gear(&rules);
        let name = rules.squad("player").unwrap().members[0].name.clone();
        assert_eq!(roster.soldiers.iter().find(|s| s.name == name).unwrap().class, rules.squad("player").unwrap().members[0].class);
    }
}

//! The **roster**: your soldiers, saved between sessions.
//!
//! Each [`SoldierRecord`] keeps a soldier's name, class, looks
//! ([`Appearance`]), loadout and service record (missions, kills, wounds).
//! The roster is saved (desktop) to `tbsium_roster.ron` in the working
//! directory whenever it changes, and loaded at start; the first time, it
//! is created from the modules' `"player"` squad.
//!
//! Soldiers marked **in squad** (up to [`SQUAD_SIZE`]) are the ones that
//! deploy: the roster writes them into the ruleset's `"player"` squad, so
//! missions and skirmishes use them automatically. After each battle their
//! records are updated; with the *permadeath* setting, soldiers who fall are
//! marked KIA and leave the squad (otherwise they're just wounded).
//!
//! The **Barracks** panel (HUD button) edits it with `roster:<command>`:
//!
//! | command | effect |
//! |---|---|
//! | `roster:select:<id>` | pick a soldier to edit |
//! | `roster:squad` | toggle the selected soldier in/out of the squad |
//! | `roster:class` | next class |
//! | `roster:look:<field>` | cycle a look field (`build`, `height`, `skin`, `hair`, `hair_color`, `headgear`, `armor`, `accent`, `backpack`) |
//! | `roster:random_look`, `roster:rename` | new random look / name |
//! | `roster:recruit`, `roster:dismiss` | add a random recruit / remove the selected soldier |
//!
//! Looks change on the battlefield immediately; class and squad changes
//! apply when the mission (re)starts.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use tactics_core::content::appearance::{name_seed, random_name};
use tactics_core::content::{Appearance, Loadout, Ruleset, SquadMember};
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::state::BattleState;

pub struct RosterPlugin;

impl Plugin for RosterPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Roster::load())
            .add_systems(Update, (sync_squad, record_results).in_set(FrameSet::Logic));
    }
}

/// Most soldiers deployed at once.
pub const SQUAD_SIZE: usize = 6;
const FILE: &str = "tbsium_roster.ron";
/// Classes a recruit can be (the player classes of the bundled modules;
/// ones whose module isn't loaded are skipped).
const SOLDIER_CLASSES: [&str; 6] = ["rifleman", "assault", "sniper", "grenadier", "heavy", "augmented"];
/// Robot frames (the Robot Frames module): built, not recruited.
const ROBOT_CLASSES: [&str; 2] = ["frame_light", "frame_heavy"];

/// The classes of the same family as `class` (soldiers retrain as
/// soldiers, frames refit as frames) that the rules know.
pub(crate) fn family(class: &str, rules: &Ruleset) -> Vec<&'static str> {
    let list: &[&'static str] = if ROBOT_CLASSES.contains(&class) { &ROBOT_CLASSES } else { &SOLDIER_CLASSES };
    list.iter().copied().filter(|c| rules.class(c).is_some()).collect()
}

/// One saved soldier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoldierRecord {
    pub id: u32,
    pub name: String,
    pub class: String,
    pub appearance: Appearance,
    /// Loadout preset and individual overrides (see `content::gear`).
    pub loadout: Option<String>,
    pub gear: Loadout,
    pub in_squad: bool,
    pub missions: u32,
    pub kills: u32,
    pub wounds: u32,
    /// Killed in action (permadeath): stays on the memorial, can't deploy.
    pub kia: bool,
}

impl Default for SoldierRecord {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            class: "rifleman".into(),
            appearance: Appearance::default(),
            loadout: None,
            gear: Loadout::default(),
            in_squad: false,
            missions: 0,
            kills: 0,
            wounds: 0,
            kia: false,
        }
    }
}

/// All saved soldiers.
#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Roster {
    pub soldiers: Vec<SoldierRecord>,
    pub next_id: u32,
    /// Soldier being edited in the Barracks.
    pub selected: Option<u32>,
    /// Needs writing into the squad / to disk.
    #[serde(skip)]
    pub dirty: bool,
    /// Battle (seed) whose results were already recorded.
    #[serde(skip)]
    recorded: Option<u64>,
    /// Squad member shown in the Loadout panel.
    #[serde(skip)]
    pub loadout_selected: usize,
}

impl Roster {
    /// Loads the saved roster (desktop, Android), or starts empty (filled from the
    /// modules' squad on first sync).
    pub fn load() -> Self {
        if let Some(text) = crate::read_save(FILE) {
            match Self::from_text(&text) {
                Ok(mut r) => {
                    r.dirty = true;
                    return r;
                }
                Err(e) => warn!("couldn't read {FILE}: {e} (starting a new roster)"),
            }
        }
        Self { dirty: true, ..Default::default() }
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .map_err(|e| e.to_string())
    }

    pub fn to_text(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(4)).unwrap_or_default()
    }

    fn save(&self) {
        if let Err(e) = crate::write_save(FILE, &self.to_text()) {
            warn!("couldn't save the roster: {e}");
        }
    }

    pub fn get(&self, id: u32) -> Option<&SoldierRecord> {
        self.soldiers.iter().find(|s| s.id == id)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut SoldierRecord> {
        self.soldiers.iter_mut().find(|s| s.id == id)
    }

    fn selected_mut(&mut self) -> Option<&mut SoldierRecord> {
        let id = self.selected?;
        self.get_mut(id)
    }

    /// Soldiers who deploy, in roster order.
    pub fn squad(&self) -> impl Iterator<Item = &SoldierRecord> {
        self.soldiers.iter().filter(|s| s.in_squad && !s.kia).take(SQUAD_SIZE)
    }

    /// First run: adopt the modules' squad, giving everyone a look.
    pub fn seed_from(&mut self, rules: &Ruleset) {
        if !self.soldiers.is_empty() {
            return;
        }
        for m in rules.squad("player").map(|s| s.members.clone()).unwrap_or_default() {
            let id = self.next_id;
            self.next_id += 1;
            self.soldiers.push(SoldierRecord {
                id,
                name: m.name.clone(),
                class: m.class.clone(),
                appearance: m.appearance.unwrap_or_else(|| Appearance::from_name(&m.name)),
                loadout: m.loadout.clone(),
                gear: m.gear.clone(),
                in_squad: true,
                ..Default::default()
            });
        }
        self.selected = self.soldiers.first().map(|s| s.id);
    }

    /// Writes the deploying soldiers into the ruleset's `"player"` squad.
    pub fn write_squad(&self, rules: &mut Ruleset) {
        let members: Vec<SquadMember> = self
            .squad()
            .map(|s| SquadMember {
                class: s.class.clone(),
                name: s.name.clone(),
                loadout: s.loadout.clone(),
                gear: s.gear.clone(),
                appearance: Some(s.appearance),
            })
            .collect();
        if members.is_empty() {
            return; // keep the modules' squad rather than deploy nobody
        }
        match rules.squad_mut("player") {
            Some(squad) => squad.members = members,
            None => rules.add_squad(tactics_core::content::SquadDef { id: "player".into(), members }),
        }
    }

    /// Copies loadout edits made on the squad (Loadout panel: class and
    /// gear) back into the records.
    pub fn pull_gear(&mut self, rules: &Ruleset) {
        let Some(squad) = rules.squad("player") else { return };
        for m in &squad.members {
            if let Some(s) = self.soldiers.iter_mut().find(|s| s.name == m.name && !s.kia) {
                s.class = m.class.clone();
                s.loadout = m.loadout.clone();
                s.gear = m.gear.clone();
            }
        }
        self.dirty = true;
    }

    /// Adds a recruit (a soldier, or with `robot` a robot frame) of a
    /// random class the rules know.
    fn recruit(&mut self, rules: &Ruleset, robot: bool) -> Option<u32> {
        let classes = family(if robot { ROBOT_CLASSES[0] } else { SOLDIER_CLASSES[0] }, rules);
        if classes.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let seed = name_seed(&format!("recruit-{id}-{}", self.soldiers.len())) ^ (id as u64).wrapping_mul(0x9E37_79B9);
        let class = classes[(seed % classes.len() as u64) as usize];
        let in_squad = self.squad().count() < SQUAD_SIZE;
        let name = if robot { self.designation(class, seed) } else { self.unique_name(seed) };
        self.soldiers.push(SoldierRecord {
            id,
            name,
            class: class.into(),
            appearance: Appearance::random(seed),
            loadout: None,
            in_squad,
            ..Default::default()
        });
        Some(id)
    }

    /// A robot's name: model letter and serial, e.g. "Strider S-07".
    fn designation(&self, class: &str, seed: u64) -> String {
        let (model, letter) = if class == "frame_heavy" { ("Bastion", 'B') } else { ("Strider", 'S') };
        (0..100u64)
            .map(|i| format!("{model} {letter}-{:02}", (seed.wrapping_add(i * 37) % 90) + 10))
            .find(|n| !self.soldiers.iter().any(|s| &s.name == n))
            .unwrap_or_else(|| format!("{model} {letter}-{}", self.next_id))
    }

    fn unique_name(&self, seed: u64) -> String {
        (0..50u64)
            .map(|i| random_name(seed.wrapping_add(i * 7919)))
            .find(|n| !self.soldiers.iter().any(|s| &s.name == n))
            .unwrap_or_else(|| format!("Recruit {}", self.next_id))
    }

    /// Applies a `roster:...` command. Returns a log line.
    pub fn command(&mut self, arg: &str, rules: &Ruleset) -> Option<String> {
        let (cmd, rest) = arg.split_once(':').unwrap_or((arg, ""));
        let msg = match cmd.trim() {
            "select" => {
                let id: u32 = rest.trim().parse().ok()?;
                self.get(id)?;
                self.selected = Some(id);
                return None;
            }
            "recruit" => {
                let robot = rest.trim() == "robot";
                let Some(id) = self.recruit(rules, robot) else {
                    return Some("No robot frames are loaded (Robot Frames module)".into());
                };
                self.selected = Some(id);
                let s = self.get(id)?;
                format!("Recruited {} ({})", s.name, s.class)
            }
            "dismiss" => {
                let id = self.selected?;
                let name = self.get(id)?.name.clone();
                self.soldiers.retain(|s| s.id != id);
                self.selected = self.soldiers.first().map(|s| s.id);
                format!("{name} left the squad for good")
            }
            "squad" => {
                let full = self.squad().count() >= SQUAD_SIZE;
                let s = self.selected_mut()?;
                if s.kia {
                    return Some(format!("{} is KIA", s.name));
                }
                if !s.in_squad && full {
                    return Some(format!("The squad is full ({SQUAD_SIZE})"));
                }
                s.in_squad = !s.in_squad;
                format!("{} {} (restart the mission to apply)", s.name, if s.in_squad { "joins the squad" } else { "stays at base" })
            }
            "class" => {
                let s = self.selected_mut()?;
                let classes = family(&s.class, rules);
                if classes.is_empty() {
                    return None;
                }
                let i = classes.iter().position(|c| *c == s.class).map_or(0, |i| (i + 1) % classes.len());
                s.class = classes[i].into();
                // Gear from another class may not fit: fall back to the class default.
                if !rules.check_gear(&s.class, &rules.gear_for(&s.class, s.loadout.as_deref(), &s.gear)).is_empty() {
                    s.loadout = None;
                    s.gear = Loadout::default();
                }
                format!("{} retrains as {} (restart to apply)", s.name, s.class)
            }
            "look" => {
                let s = self.selected_mut()?;
                if !s.appearance.cycle(rest.trim()) {
                    return Some(format!("unknown look field '{rest}'"));
                }
                return self.touch();
            }
            "random_look" => {
                let s = self.selected_mut()?;
                let seed = name_seed(&s.name) ^ (s.appearance.armor as u64 * 31 + s.appearance.skin as u64 * 7 + s.missions as u64 + 1).wrapping_mul(0xA24B_AED4);
                s.appearance = Appearance::random(seed);
                return self.touch();
            }
            "rename" => {
                let seed = self.selected.map_or(0, |id| id as u64 * 104_729 + self.soldiers.len() as u64 * 13 + self.next_id as u64);
                let name = self.unique_name(seed ^ 0x5151);
                let s = self.selected_mut()?;
                let old = std::mem::replace(&mut s.name, name.clone());
                format!("{old} is now {name} (restart to apply)")
            }
            _ => return Some(format!("unknown roster command '{arg}'")),
        };
        self.dirty = true;
        Some(msg)
    }

    fn touch(&mut self) -> Option<String> {
        self.dirty = true;
        None
    }

    /// Updates service records after a battle. `kills` by soldier name;
    /// `fallen` = soldiers who went down.
    pub fn record(&mut self, deployed: &[String], kills: &BTreeMap<String, u32>, fallen: &[String], permadeath: bool) {
        for s in self.soldiers.iter_mut().filter(|s| deployed.contains(&s.name)) {
            s.missions += 1;
            s.kills += kills.get(&s.name).copied().unwrap_or(0);
            if fallen.contains(&s.name) {
                s.wounds += 1;
                if permadeath {
                    s.kia = true;
                    s.in_squad = false;
                }
            }
        }
        self.dirty = true;
    }

    /// Rows for the HUD's `roster` list.
    pub fn rows(&self) -> Vec<BTreeMap<String, String>> {
        self.soldiers
            .iter()
            .map(|s| {
                let state = if self.selected == Some(s.id) {
                    "selected"
                } else if s.kia {
                    "done"
                } else if s.in_squad {
                    "ready"
                } else {
                    ""
                };
                let tag = if s.kia { "KIA" } else if s.in_squad { "SQUAD" } else { "" };
                BTreeMap::from([
                    ("id".to_string(), s.id.to_string()),
                    ("name".to_string(), s.name.clone()),
                    ("class".to_string(), s.class.clone()),
                    ("tag".to_string(), tag.to_string()),
                    ("record".to_string(), format!("{} missions, {} kills", s.missions, s.kills)),
                    ("state".to_string(), state.to_string()),
                ])
            })
            .collect()
    }
}

/// Keeps the squad, the battlefield looks and the save file in step with
/// the roster.
fn sync_squad(
    mut roster: ResMut<Roster>,
    mut state: ResMut<BattleState>,
    mut refresh: MessageWriter<crate::characters::RefreshUnitVisuals>,
    campaign: Res<crate::campaign::Campaign>,
) {
    // A campaign deploys its own soldiers.
    if campaign.active() {
        return;
    }
    if roster.is_added() {
        let rules = state.battle.rules().clone();
        roster.seed_from(&rules);
    }
    if !roster.dirty {
        return;
    }
    roster.dirty = false;
    roster.write_squad(state.battle.rules_mut());
    // Show look changes right away on soldiers already in the field.
    let mut changed = false;
    let looks: Vec<(String, Appearance)> = roster.soldiers.iter().map(|s| (s.name.clone(), s.appearance)).collect();
    let ids: Vec<(UnitId, Appearance)> = state
        .battle
        .units()
        .iter()
        .filter(|u| u.team == Team::Player)
        .filter_map(|u| looks.iter().find(|(n, _)| *n == u.name).map(|(_, a)| (u.id, *a)).filter(|(_, a)| u.appearance != Some(*a)))
        .collect();
    for (id, a) in ids {
        state.battle.set_unit_appearance(id, a);
        changed = true;
    }
    if changed {
        refresh.write(crate::characters::RefreshUnitVisuals);
    }
    roster.save();
}

/// When a battle ends, credit the deployed soldiers.
fn record_results(
    mut roster: ResMut<Roster>,
    state: Res<BattleState>,
    playback: Res<crate::playback::Playback>,
    settings: Res<crate::settings::GameSettings>,
    campaign: Res<crate::campaign::Campaign>,
) {
    let b = &state.battle;
    if !b.is_over() || playback.is_busy() || roster.recorded == Some(state.seed) || campaign.active() {
        return;
    }
    if state.mission.as_deref() == Some(crate::lab::LAB_MISSION) {
        return;
    }
    roster.recorded = Some(state.seed);
    let deployed: Vec<String> = b.units().iter().filter(|u| u.team == Team::Player).map(|u| u.name.clone()).collect();
    let fallen: Vec<String> = b.units().iter().filter(|u| u.team == Team::Player && !u.is_alive()).map(|u| u.name.clone()).collect();
    roster.record(&deployed, &state.stats.kills_by, &fallen, settings.permadeath);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Ruleset {
        crate::content::build_ruleset().0
    }

    #[test]
    fn seeds_from_the_squad_and_round_trips() {
        let mut r = Roster::default();
        r.seed_from(&rules());
        assert_eq!(r.soldiers.len(), 4);
        assert!(r.soldiers.iter().all(|s| s.in_squad));
        r.command("recruit", &rules()).unwrap();
        assert_eq!(r.soldiers.len(), 5);
        let back = Roster::from_text(&r.to_text()).unwrap();
        assert_eq!(back.soldiers, r.soldiers);
    }

    #[test]
    fn squad_written_into_rules_with_looks_and_capped() {
        let mut rules = rules();
        let mut r = Roster::default();
        r.seed_from(&rules);
        for _ in 0..5 {
            r.command("recruit", &rules);
        }
        assert_eq!(r.squad().count(), SQUAD_SIZE);
        r.write_squad(&mut rules);
        let members = &rules.squad("player").unwrap().members;
        assert_eq!(members.len(), SQUAD_SIZE);
        assert!(members.iter().all(|m| m.appearance.is_some()));
        // Selecting and editing.
        let id = r.soldiers[0].id;
        r.command(&format!("select:{id}"), &rules);
        let before = r.soldiers[0].appearance;
        r.command("look:armor", &rules);
        assert_ne!(r.soldiers[0].appearance, before);
        r.command("squad", &rules);
        assert!(!r.soldiers[0].in_squad);
    }

    #[test]
    fn results_credit_kills_and_permadeath_marks_kia() {
        let mut r = Roster::default();
        r.seed_from(&rules());
        let a = r.soldiers[0].name.clone();
        let b = r.soldiers[1].name.clone();
        let kills = BTreeMap::from([(a.clone(), 2)]);
        r.record(&[a.clone(), b.clone()], &kills, std::slice::from_ref(&b), false);
        assert_eq!((r.soldiers[0].missions, r.soldiers[0].kills), (1, 2));
        assert_eq!(r.soldiers[1].wounds, 1);
        assert!(!r.soldiers[1].kia);
        r.record(std::slice::from_ref(&b), &BTreeMap::new(), std::slice::from_ref(&b), true);
        assert!(r.soldiers[1].kia && !r.soldiers[1].in_squad);
    }
}

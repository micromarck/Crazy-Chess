//! The campaign in the game: save slots, the Campaign panel, launching
//! story and side missions, and the debrief when one ends. The rules live in
//! [`tactics_core::campaign`]; this module saves, shows and launches.
//!
//! | command | effect |
//! |---|---|
//! | `campaign:new:<slot>` / `campaign:load:<slot>` / `campaign:delete:<slot>` | start, continue or erase the campaign in save slot 1-4 |
//! | `campaign:quit` | save and leave campaign mode (back to skirmish) |
//! | `campaign:story` | launch the next story mission |
//! | `campaign:side:<i>` | launch side mission `i` |
//! | `campaign:squad:<soldier id>` | put a soldier in or out of the squad |
//! | `campaign:recruit` | hire a recruit |
//! | `campaign:buy:<id>` / `campaign:sell:<id>` | shop |
//! | `campaign:shop:<tab>` | shop tab: `weapons`, `armor`, `items`, `augments` |
//! | `campaign:reload` / `campaign:giveup` | after a failed story mission |
//! | `campaign:view:<tab>` | right-hand tab: `soldiers`, `skills`, `tech`, `shop` |
//! | `campaign:pick:<soldier id>` | show this soldier's skill tree |
//! | `campaign:learn:<soldier id>:<skill>` | spend a skill point |
//! | `campaign:research:<tech>` | research a tech (funds) |
//! | `campaign:treat:<soldier id>:<condition>` | the infirmary: cure a drug side effect (funds) |
//!
//! While a campaign is open its soldiers are the squad (the skirmish
//! Barracks roster is left alone); gear is edited in the Loadout panel as
//! usual, and whatever isn't in the stash has to be bought before launching.
//! Leaving a campaign mission early (restarting it, starting another
//! mission) counts as failing it.

use std::collections::BTreeMap;

use bevy::prelude::*;
use tactics_core::campaign::{self, CampaignDef, CampaignState, MissionResult, Status};
use tactics_core::content::{Appearance, ItemSlot, Ruleset, SquadDef, SquadMember};
use tactics_core::unit::Team;

use crate::FrameSet;
use crate::hud::LogMessages;
use crate::missions::StartBattle;
use crate::state::BattleState;

/// Save slots.
pub const SLOTS: u32 = 4;

fn slot_file(n: u32) -> String {
    format!("tbsium_campaign_{n}.ron")
}

pub struct CampaignPlugin;

impl Plugin for CampaignPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Campaign::new())
            .add_message::<CampaignCommand>()
            .add_systems(Update, (run_commands, track_battle).chain().in_set(FrameSet::Logic));
    }
}

/// A `campaign:...` command from the HUD.
#[derive(Message, Debug, Clone)]
pub struct CampaignCommand(pub String);

#[derive(Resource)]
pub struct Campaign {
    pub def: CampaignDef,
    /// The open campaign (`None`: skirmish mode).
    pub state: Option<CampaignState>,
    pub slot: u32,
    /// What each save slot holds (for the slot list).
    pub slots: Vec<Option<CampaignState>>,
    /// `BattleState::battle_id` when the mission was launched (waiting for it to start).
    launched_at: Option<u64>,
    /// `BattleState::battle_id` of the campaign mission being played.
    playing: Option<u64>,
    /// Feedback for the last command.
    pub message: String,
    pub shop_tab: String,
    /// Right-hand tab of the panel.
    pub view: String,
    /// Soldier whose skill tree is shown.
    pub picked: Option<u32>,
}

impl Campaign {
    fn new() -> Self {
        let mut c = Self {
            def: campaign::ghost_protocol(),
            state: None,
            slot: 0,
            slots: Vec::new(),
            launched_at: None,
            playing: None,
            message: String::new(),
            shop_tab: "items".into(),
            view: "soldiers".into(),
            picked: None,
        };
        c.refresh_slots();
        c
    }

    pub fn active(&self) -> bool {
        self.state.is_some()
    }

    fn refresh_slots(&mut self) {
        self.slots = (1..=SLOTS)
            .map(|n| crate::read_save(&slot_file(n)).filter(|t| !t.trim().is_empty()).and_then(|t| CampaignState::from_text(&t).ok()))
            .collect();
    }

    fn save(&mut self) {
        // Slot 0 is no slot (tests, or nothing chosen yet).
        if self.slot == 0 {
            return;
        }
        if let Some(s) = &self.state
            && let Err(e) = crate::write_save(&slot_file(self.slot), &s.to_text())
        {
            warn!("couldn't save the campaign: {e}");
        }
        self.refresh_slots();
    }
}

/// The campaign's deploying soldiers become the ruleset's `"player"` squad.
pub fn write_squad(s: &CampaignState, rules: &mut Ruleset) {
    let members: Vec<SquadMember> = s
        .squad()
        .map(|c| SquadMember {
            class: c.class.clone(),
            name: c.name.clone(),
            loadout: c.loadout.clone(),
            gear: c.gear.clone(),
            appearance: Some(c.appearance.unwrap_or_else(|| Appearance::from_name(&c.name))),
        })
        .collect();
    match rules.squad_mut("player") {
        Some(sq) => sq.members = members,
        None => rules.add_squad(SquadDef { id: "player".into(), members }),
    }
}

/// Loadout edits (made on the squad in the Loadout panel) back into the soldiers.
pub fn pull_gear(s: &mut CampaignState, rules: &Ruleset) {
    let Some(squad) = rules.squad("player") else { return };
    for m in &squad.members {
        if let Some(c) = s.soldiers.iter_mut().find(|c| c.name == m.name && !c.dead) {
            c.class = m.class.clone();
            c.loadout = m.loadout.clone();
            c.gear = m.gear.clone();
        }
    }
}

const RECRUIT_NAMES: [&str; 24] = [
    "Pvt. Adeyemi", "Pvt. Brandt", "Pvt. Castillo", "Pvt. Dvorak", "Pvt. Eriksen", "Pvt. Farah", "Pvt. Gallo", "Pvt. Haddad",
    "Pvt. Ibarra", "Pvt. Jensen", "Pvt. Kaur", "Pvt. Lindqvist", "Pvt. Mbeki", "Pvt. Nakamura", "Pvt. Orlov", "Pvt. Petrova",
    "Pvt. Quint", "Pvt. Rossi", "Pvt. Sato", "Pvt. Tanaka", "Pvt. Umar", "Pvt. Varga", "Pvt. Weiss", "Pvt. Yilmaz",
];
const RECRUIT_CLASSES: [&str; 5] = ["rifleman", "assault", "sniper", "grenadier", "heavy"];

#[allow(clippy::too_many_arguments)]
fn run_commands(
    mut commands: MessageReader<CampaignCommand>,
    mut camp: ResMut<Campaign>,
    mut state: ResMut<BattleState>,
    mut start: MessageWriter<StartBattle>,
    mut log: ResMut<LogMessages>,
    mut flags: ResMut<crate::html_ui::UiFlags>,
    mut roster: ResMut<crate::roster::Roster>,
) {
    for CampaignCommand(cmd) in commands.read() {
        let mut parts = cmd.split(':').map(str::trim);
        let what = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("");
        let arg2 = parts.next().unwrap_or("");
        let slot = arg.parse::<u32>().ok().filter(|n| (1..=SLOTS).contains(n));
        let def = camp.def.clone();
        let msg: Result<String, String> = match what {
            "new" => match slot {
                Some(n) => {
                    let seed = state.seed ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (state.revision << 7);
                    let s = CampaignState::new(&def, state.battle.rules(), &format!("{} - slot {n}", def.name), seed);
                    write_squad(&s, state.battle.rules_mut());
                    camp.state = Some(s);
                    camp.slot = n;
                    camp.save();
                    Ok(format!("New campaign: {}.", def.name))
                }
                None => Err("which slot?".into()),
            },
            "load" => match slot.and_then(|n| camp.slots.get(n as usize - 1).cloned().flatten().map(|s| (n, s))) {
                Some((n, s)) => {
                    write_squad(&s, state.battle.rules_mut());
                    camp.state = Some(s);
                    camp.slot = n;
                    Ok(format!("Campaign loaded (slot {n})."))
                }
                None => Err("that slot is empty".into()),
            },
            "delete" => match slot {
                Some(n) if camp.active() && camp.slot == n => Err("quit this campaign before erasing its slot".into()),
                Some(n) => {
                    let _ = crate::write_save(&slot_file(n), "");
                    camp.refresh_slots();
                    Ok(format!("Slot {n} erased."))
                }
                None => Err("which slot?".into()),
            },
            "quit" => {
                if let Some(mut s) = camp.state.take() {
                    pull_gear(&mut s, state.battle.rules());
                    camp.state = Some(s);
                    camp.save();
                    camp.state = None;
                }
                camp.playing = None;
                camp.launched_at = None;
                // Back to the Barracks squad.
                roster.dirty = true;
                Ok("Campaign saved. Back to skirmish.".into())
            }
            _ => {
                let Some(mut s) = camp.state.take() else {
                    camp.message = "No campaign open.".into();
                    continue;
                };
                pull_gear(&mut s, state.battle.rules());
                let rules = state.battle.rules().clone();
                let r = match what {
                    "story" | "side" => {
                        let side = if what == "side" { arg.parse::<usize>().ok() } else { None };
                        match s.launch(&def, &rules, side) {
                            Ok(()) => {
                                write_squad(&s, state.battle.rules_mut());
                                let request = match side {
                                    None => StartBattle::Mission(def.chapters[s.chapter].mission.clone()),
                                    Some(i) => StartBattle::Def(Box::new(tactics_core::levelgen::generate(s.side_ops[i].params()))),
                                };
                                start.write(request);
                                camp.launched_at = Some(state.battle_id);
                                if let Some(f) = flags.0.get_mut("campaign") {
                                    *f = false;
                                }
                                Ok(match side {
                                    None => format!("Launching {}.", def.chapters[s.chapter].title),
                                    Some(i) => format!("Launching side mission: {}.", s.side_ops[i].name),
                                })
                            }
                            Err(e) => Err(e),
                        }
                    }
                    "squad" => arg.parse::<u32>().map_err(|_| "which soldier?".to_string()).and_then(|id| s.toggle_squad(id)).map(|_| {
                        write_squad(&s, state.battle.rules_mut());
                        "Squad changed.".to_string()
                    }),
                    "recruit" => {
                        let used: Vec<&str> = s.soldiers.iter().map(|c| c.name.as_str()).collect();
                        let name = RECRUIT_NAMES
                            .iter()
                            .find(|n| !used.contains(n))
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| format!("Pvt. Recruit {}", s.next_id));
                        s.recruit(&rules, &RECRUIT_CLASSES, &name).map(|id| {
                            write_squad(&s, state.battle.rules_mut());
                            let c = s.soldiers.iter().find(|c| c.id == id).map(|c| c.class.clone()).unwrap_or_default();
                            let class = rules.class(&c).map_or(c.clone(), |k| k.name.clone());
                            format!("{name} ({class}) joins for {}.", campaign::RECRUIT_COST)
                        })
                    }
                    "buy" => s.buy(&rules, arg).map(|_| format!("Bought {}.", campaign::item_name(&rules, arg))),
                    "sell" => s.sell(&rules, arg).map(|_| format!("Sold {}.", campaign::item_name(&rules, arg))),
                    "shop" => {
                        camp.shop_tab = arg.to_string();
                        Ok(String::new())
                    }
                    "view" => {
                        camp.view = arg.to_string();
                        Ok(String::new())
                    }
                    "pick" => {
                        camp.picked = arg.parse().ok();
                        camp.view = "skills".into();
                        Ok(String::new())
                    }
                    "learn" => match arg.parse::<u32>() {
                        Ok(id) => s.learn(id, arg2),
                        Err(_) => Err("which soldier?".into()),
                    },
                    "research" => s.research(arg),
                    "treat" => match arg.parse::<u32>() {
                        Ok(id) => s.treat(id, arg2),
                        Err(_) => Err("which soldier?".into()),
                    },
                    "reload" => s.reload().map(|_| {
                        write_squad(&s, state.battle.rules_mut());
                        "Checkpoint reloaded.".to_string()
                    }),
                    "giveup" => {
                        s.give_up();
                        Ok("You gave up the campaign.".into())
                    }
                    other => Err(format!("unknown campaign command '{other}'")),
                };
                camp.state = Some(s);
                camp.save();
                r
            }
        };
        match msg {
            Ok(m) => {
                if !m.is_empty() {
                    log.push(m.clone());
                }
                camp.message = m;
            }
            Err(e) => {
                log.push(format!("Campaign: {e}"));
                camp.message = e;
            }
        }
    }
}

/// Applies rank bonuses when a campaign mission starts, and the result when
/// it ends (or is abandoned).
fn track_battle(
    mut camp: ResMut<Campaign>,
    mut state: ResMut<BattleState>,
    playback: Res<crate::playback::Playback>,
    mut log: ResMut<LogMessages>,
    mut flags: ResMut<crate::html_ui::UiFlags>,
) {
    // The launched mission has started: veterans get their bonuses.
    if let Some(at) = camp.launched_at
        && state.battle_id != at
    {
        camp.launched_at = None;
        camp.playing = Some(state.battle_id);
        if let Some(s) = &camp.state {
            // Rank, skills and techs: stats, and the skills' extra abilities.
            let bonuses: Vec<_> = state
                .battle
                .units()
                .iter()
                .filter(|u| u.team == Team::Player)
                .filter_map(|u| s.soldiers.iter().find(|c| c.name == u.name).map(|c| (u.id, s.deploy_bonus(c))))
                .collect();
            for (id, (b, abilities)) in bonuses {
                state.battle.apply_bonus(id, b);
                for (a, n) in abilities {
                    state.battle.grant_ability(id, &tactics_core::content::AbilityId::new(&a), n);
                }
            }
        }
    }
    let Some(rev) = camp.playing else { return };
    let b = &state.battle;
    // (`battle_id` changes only when another battle starts - not on every move.)
    let result = if state.battle_id != rev {
        // Left the mission before it ended: a failure (the squad pulls out).
        Some(MissionResult { won: false, fallen: Vec::new(), kills: BTreeMap::new(), doses: BTreeMap::new() })
    } else if b.is_over() && !playback.is_busy() {
        let fallen = b
            .units()
            .iter()
            .filter(|u| u.home_team() == Team::Player && (!u.is_alive() || u.status.controlled == u32::MAX))
            .map(|u| u.name.clone())
            .collect();
        let doses = b
            .units()
            .iter()
            .filter(|u| u.home_team() == Team::Player && !u.doses.is_empty())
            .map(|u| (u.name.clone(), u.doses.iter().map(|d| d.to_string()).collect()))
            .collect();
        Some(MissionResult { won: b.winner() == Some(Team::Player), fallen, kills: state.stats.kills_by.clone(), doses })
    } else {
        None
    };
    let Some(result) = result else { return };
    camp.playing = None;
    let def = camp.def.clone();
    if let Some(mut s) = camp.state.take() {
        camp.message = String::new();
        if s.status == Status::InMission {
            let report = s.finish(&def, &result);
            for line in report.lines() {
                log.push(line.to_string());
            }
        }
        write_squad(&s, state.battle.rules_mut());
        camp.state = Some(s);
        camp.save();
        flags.0.insert("campaign".into(), true);
    }
}

/// Panel data (`camp.*` values and `camp_*` lists).
pub fn fill_ui(camp: &Campaign, rules: &Ruleset, v: &mut crate::html_ui::bind::UiValues) {
    let yes = |b: bool| if b { "yes" } else { "" };
    v.set("camp.active", yes(camp.active()));
    v.set("camp.name", &camp.def.name);
    v.set("camp.message", &camp.message);
    v.lists.insert(
        "camp_slots".into(),
        camp.slots
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let label = match s {
                    None => "Empty".to_string(),
                    Some(s) => format!(
                        "Chapter {}/{} - {} funds - {} soldiers{}",
                        (s.chapter + 1).min(camp.def.chapters.len()),
                        camp.def.chapters.len(),
                        s.funds,
                        s.alive().count(),
                        match s.status {
                            Status::Won => " - WON",
                            Status::Lost => " - lost",
                            Status::StoryFailed => " - mission failed",
                            _ => "",
                        }
                    ),
                };
                BTreeMap::from([
                    ("n".to_string(), (i + 1).to_string()),
                    ("label".to_string(), label),
                    ("used".to_string(), yes(s.is_some()).to_string()),
                ])
            })
            .collect(),
    );
    let Some(s) = &camp.state else { return };
    let ch = s.current_chapter(&camp.def);
    v.set("camp.title", &s.title);
    v.set("camp.chapter", format!("{}/{}", (s.chapter + 1).min(camp.def.chapters.len()), camp.def.chapters.len()));
    v.set("camp.funds", s.funds);
    v.set("camp.report", &s.last_report);
    v.set("camp.ready", yes(s.status == Status::Ready));
    v.set("camp.failed", yes(s.status == Status::StoryFailed));
    v.set("camp.over", yes(matches!(s.status, Status::Won | Status::Lost)));
    v.set("camp.won", yes(s.status == Status::Won));
    v.set("camp.in_mission", yes(s.status == Status::InMission));
    v.set("camp.story_title", ch.map_or(String::new(), |c| c.title.clone()));
    v.set("camp.story_briefing", ch.map_or(String::new(), |c| c.briefing.clone()));
    v.set("camp.story_reward", ch.map_or(0, |c| c.reward));
    v.set("camp.has_story", yes(ch.is_some() && s.status == Status::Ready));
    v.set("camp.recruit_cost", campaign::RECRUIT_COST);
    let short = s.shortfall(rules);
    v.set(
        "camp.shortfall",
        if short.is_empty() {
            String::new()
        } else {
            format!(
                "Missing from the stash: {}",
                short.iter().map(|(id, n)| format!("{n} x {}", campaign::item_name(rules, id))).collect::<Vec<_>>().join(", ")
            )
        },
    );
    v.set("camp.has_sides", yes(s.side_ops.iter().any(|o| !o.done) && s.status == Status::Ready));
    v.lists.insert(
        "camp_sides".into(),
        s.side_ops
            .iter()
            .enumerate()
            .map(|(i, o)| {
                BTreeMap::from([
                    ("i".to_string(), i.to_string()),
                    ("name".to_string(), o.name.clone()),
                    ("info".to_string(), format!("{} - {} - pays {}", o.theme, o.objective, o.reward)),
                    ("state".to_string(), if o.done { "done".into() } else { String::new() }),
                ])
            })
            .collect(),
    );
    v.lists.insert(
        "camp_soldiers".into(),
        s.soldiers
            .iter()
            .map(|c| {
                let class = rules.class(&c.class).map_or(c.class.clone(), |k| k.name.clone());
                let next = c.next_rank_in().map_or("top rank".into(), |n| format!("{n} XP to next rank"));
                let effects = campaign::side_effects();
                let conditions: Vec<String> = c
                    .conditions
                    .iter()
                    .filter_map(|k| effects.iter().find(|e| e.id == k.id).map(|e| (e, k)))
                    .map(|(e, k)| format!("{}{}", e.name, k.missions_left.map_or(" (lasting)".to_string(), |n| format!(" ({n})"))))
                    .collect();
                let info = if c.dead {
                    format!("KIA - {} missions, {} kills", c.missions, c.kills)
                } else {
                    let mut t = format!("{} {} - {} XP ({next}) - {} missions, {} kills", c.rank_name(), class, c.xp, c.missions, c.kills);
                    if !conditions.is_empty() {
                        t += &format!(" - {}", conditions.join(", "));
                    }
                    t
                };
                BTreeMap::from([
                    ("id".to_string(), c.id.to_string()),
                    ("name".to_string(), c.name.clone()),
                    ("info".to_string(), info),
                    (
                        "state".to_string(),
                        if c.dead { "dead".into() } else if c.in_squad { "selected".into() } else { String::new() },
                    ),
                    ("squad".to_string(), if c.dead { "-".into() } else if c.in_squad { "In squad".into() } else { "Reserve".into() }),
                ])
            })
            .collect(),
    );
    // The infirmary: every lasting side effect, with its cure.
    let effects = campaign::side_effects();
    v.lists.insert(
        "camp_conditions".into(),
        s.alive()
            .flat_map(|c| c.conditions.iter().map(move |k| (c, k)))
            .filter_map(|(c, k)| effects.iter().find(|e| e.id == k.id).map(|e| (c, k, e)))
            .map(|(c, k, e)| {
                BTreeMap::from([
                    ("soldier".to_string(), c.id.to_string()),
                    ("id".to_string(), e.id.to_string()),
                    (
                        "name".to_string(),
                        format!("{}: {}", c.name, e.name),
                    ),
                    (
                        "info".to_string(),
                        format!("{} - {} - cure {}", e.description, k.missions_left.map_or("lasting".to_string(), |n| format!("{n} more mission{}", if n == 1 { "" } else { "s" })), e.treat_cost),
                    ),
                ])
            })
            .collect(),
    );
    v.set("camp.has_conditions", yes(s.alive().any(|c| !c.conditions.is_empty())));
    // The shop: one tab at a time.
    let tab = camp.shop_tab.as_str();
    let mut shop: Vec<(String, String, u32)> = Vec::new();
    match tab {
        "weapons" => {
            for w in rules.weapons() {
                let p = campaign::price(rules, &w.id);
                let soldier_weapon = !w.classes.is_empty() || rules.classes().any(|c| c.weapon == w.id && c.model == "soldier");
                if p > 0 && soldier_weapon {
                    shop.push((w.id.clone(), w.name.clone(), p));
                }
            }
        }
        _ => {
            let slot = match tab {
                "armor" => ItemSlot::Armor,
                "augments" => ItemSlot::Augment,
                _ => ItemSlot::Utility,
            };
            for i in rules.items().filter(|i| i.slot == slot) {
                let p = campaign::price(rules, &i.id);
                if p > 0 {
                    shop.push((i.id.clone(), i.name.clone(), p));
                }
            }
        }
    }
    v.set("camp.tab", tab);
    v.set("camp.view", &camp.view);
    // The tech tree.
    v.lists.insert(
        "camp_techs".into(),
        campaign::techs()
            .into_iter()
            .map(|t| {
                let done = s.techs.iter().any(|x| x == t.id);
                let missing: Vec<String> = t
                    .requires
                    .iter()
                    .filter(|r| !s.techs.iter().any(|x| x == *r))
                    .map(|r| campaign::techs().into_iter().find(|x| x.id == *r).map_or(r.to_string(), |x| x.name.to_string()))
                    .collect();
                let (state, why) = if done {
                    ("done", "researched".to_string())
                } else if !missing.is_empty() {
                    ("locked", format!("needs {}", missing.join(", ")))
                } else if s.chapter < t.min_chapter {
                    ("locked", format!("from chapter {}", t.min_chapter + 1))
                } else {
                    ("open", format!("{} funds", t.cost))
                };
                BTreeMap::from([
                    ("id".to_string(), t.id.to_string()),
                    ("name".to_string(), t.name.to_string()),
                    ("info".to_string(), format!("{} - {why}", t.description)),
                    ("state".to_string(), state.to_string()),
                    ("can".to_string(), yes(state == "open").to_string()),
                ])
            })
            .collect(),
    );
    // The picked soldier's skill tree.
    let picked = camp.picked.and_then(|id| s.soldiers.iter().find(|c| c.id == id && !c.dead)).or_else(|| s.alive().next());
    v.set("camp.picked", picked.map_or(String::new(), |c| c.name.clone()));
    v.set("camp.picked_id", picked.map_or(String::new(), |c| c.id.to_string()));
    v.set(
        "camp.picked_info",
        picked.map_or(String::new(), |c| format!("{} - {} skill point{} (one per rank)", c.rank_name(), c.skill_points(), if c.skill_points() == 1 { "" } else { "s" })),
    );
    let all = campaign::skills();
    v.lists.insert(
        "camp_skills".into(),
        all.iter()
            .map(|k| {
                let (state, can) = match picked {
                    None => ("locked", false),
                    Some(c) => {
                        let known = c.skills.iter().any(|x| x == k.id);
                        let prev_ok = all.iter().find(|p| p.branch == k.branch && p.tier + 1 == k.tier).is_none_or(|p| c.skills.iter().any(|x| x == p.id));
                        if known {
                            ("done", false)
                        } else if prev_ok && c.skill_points() > 0 {
                            ("open", true)
                        } else {
                            ("locked", false)
                        }
                    }
                };
                BTreeMap::from([
                    ("id".to_string(), k.id.to_string()),
                    ("name".to_string(), format!("{} {}", k.branch, "I".repeat(k.tier as usize)) + ": " + k.name),
                    ("info".to_string(), k.description.to_string()),
                    ("state".to_string(), state.to_string()),
                    ("can".to_string(), yes(can).to_string()),
                ])
            })
            .collect(),
    );
    v.lists.insert(
        "camp_shop".into(),
        shop.into_iter()
            .map(|(id, name, p)| {
                let owned = s.stash.get(&id).copied().unwrap_or(0);
                let lock = if s.unlocked(&id) { String::new() } else { campaign::unlocked_by(&id).map_or(String::new(), |t| format!(" - needs {}", t.name)) };
                BTreeMap::from([
                    ("locked".to_string(), lock.clone()),
                    ("can_buy".to_string(), yes(lock.is_empty()).to_string()),
                    ("id".to_string(), id),
                    ("name".to_string(), name),
                    ("price".to_string(), p.to_string()),
                    ("owned".to_string(), owned.to_string()),
                    ("can_sell".to_string(), yes(owned > 0).to_string()),
                ])
            })
            .collect(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tactics_core::action::Action;

    /// Moving during a campaign mission must not count as leaving it (the
    /// battle's `revision` changes with every action; `battle_id` doesn't).
    #[test]
    fn moving_in_a_campaign_mission_doesnt_abandon_it() {
        let (rules, _) = crate::content::build_ruleset();
        let def = campaign::ghost_protocol();
        let mut s = CampaignState::new(&def, &rules, "Test", 1);
        let mut rules = rules;
        s.launch(&def, &rules, None).unwrap();
        write_squad(&s, &mut rules);
        let battle = tactics_core::scenario::skirmish(&rules, 7);
        let mut app = App::new();
        let mut camp = Campaign::new();
        camp.state = Some(s);
        camp.playing = Some(0);
        app.insert_resource(camp)
            .insert_resource(BattleState::new(battle))
            .init_resource::<crate::playback::Playback>()
            .init_resource::<LogMessages>()
            .init_resource::<crate::html_ui::UiFlags>()
            .add_systems(Update, track_battle);
        app.update();
        // A move.
        {
            let mut state = app.world_mut().resource_mut::<BattleState>();
            let id = state.battle.living(Team::Player).find(|u| u.can_act()).map(|u| u.id).unwrap();
            let to = state.battle.movement_range(id).iter().map(|(p, _)| p).find(|p| *p != state.battle.unit(id).unwrap().pos).unwrap();
            state.apply_quietly(Action::Move { unit: id, to }).unwrap();
        }
        app.update();
        let camp = app.world().resource::<Campaign>();
        assert_eq!(camp.state.as_ref().unwrap().status, Status::InMission, "still playing: {}", camp.state.as_ref().unwrap().last_report);
        assert_eq!(camp.playing, Some(0));
        // A different battle starting does count as leaving.
        app.world_mut().resource_mut::<BattleState>().battle_id += 1;
        app.update();
        let camp = app.world().resource::<Campaign>();
        assert_eq!(camp.state.as_ref().unwrap().status, Status::StoryFailed);
    }
}

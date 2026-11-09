//! Campaigns: a chain of story missions with generated side missions in
//! between, a persistent squad with **permadeath** and per-soldier
//! **progression**, and an **economy** (funds from every mission; gear
//! bought from a stash).
//!
//! All of it is plain data plus pure functions on [`CampaignState`], so the
//! rules are tested here and the frontend only saves, shows and launches:
//!
//! * [`CampaignState::new`] starts a campaign from a [`CampaignDef`] (the
//!   bundled one: [`ghost_protocol`]).
//! * Between missions the player buys gear ([`CampaignState::buy`] /
//!   [`CampaignState::sell`]), recruits ([`CampaignState::recruit`]) and
//!   picks who deploys ([`CampaignState::toggle_squad`]).
//! * [`CampaignState::launch`] checks the squad can deploy and that the stash
//!   covers everyone's gear, takes that gear out of the stash and (story
//!   missions) keeps a **checkpoint** to reload.
//! * [`CampaignState::finish`] applies a [`MissionResult`]: funds, XP and
//!   ranks, the fallen (gone for good, with their gear), the gear coming
//!   back. A lost **side** mission forfeits its reward and every item that
//!   went on it; a lost **story** mission leaves the campaign waiting for
//!   [`CampaignState::reload`] or [`CampaignState::give_up`].
//!
//! The squad is everyone alive whose `in_squad` is set; gear is the
//! soldier's class kit with loadout overrides (as in skirmish). A class's
//! own default weapon is free; every other weapon, armour, item and augment
//! has to be in the stash.

use std::collections::BTreeMap;

use crate::content::{Appearance, ItemSlot, Loadout, Ruleset, StatModifiers};
use crate::levelgen::{GenParams, Objective, Theme};
use crate::rng::GameRng;

/// Most soldiers deployed at once.
pub const SQUAD_SIZE: usize = 6;
/// What a recruit costs (they arrive with their class's standard kit).
pub const RECRUIT_COST: u32 = 150;
/// Selling gear back gets this share of its price.
pub const SELL_PERCENT: u32 = 50;

/// One story mission.
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    /// Mission id (in `assets/missions`).
    pub mission: String,
    pub title: String,
    /// Read before launching.
    pub briefing: String,
    /// Read after winning it.
    pub debrief: String,
    pub reward: u32,
    /// Enemies of the side missions after this chapter (the generator's
    /// enemy classes, most common first).
    pub side_enemies: Vec<String>,
}

/// A campaign: its chapters and start.
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignDef {
    pub id: String,
    pub name: String,
    pub intro: String,
    pub chapters: Vec<Chapter>,
    pub start_funds: u32,
    /// The starting squad: (name, class, loadout preset).
    pub start_squad: Vec<(String, String, Option<String>)>,
    /// Items in the stash at the start (on top of the squad's kits).
    pub start_stash: Vec<(String, u32)>,
    /// Shown when the last chapter is won.
    pub ending: String,
}

/// A soldier of the campaign.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct CampaignSoldier {
    pub id: u32,
    pub name: String,
    pub class: String,
    pub appearance: Option<Appearance>,
    pub loadout: Option<String>,
    pub gear: Loadout,
    pub xp: u32,
    pub kills: u32,
    pub missions: u32,
    /// Dead (permadeath): stays on the memorial.
    pub dead: bool,
    pub in_squad: bool,
}

/// Ranks: (name, XP needed). Each rank above the first: +3 aim, +1 HP.
pub const RANKS: [(&str, u32); 6] =
    [("Rookie", 0), ("Private", 3), ("Corporal", 7), ("Sergeant", 12), ("Lieutenant", 18), ("Captain", 26)];

impl CampaignSoldier {
    /// Rank index into [`RANKS`].
    pub fn rank(&self) -> usize {
        RANKS.iter().rposition(|(_, xp)| self.xp >= *xp).unwrap_or(0)
    }

    pub fn rank_name(&self) -> &'static str {
        RANKS[self.rank()].0
    }

    /// XP still needed for the next rank (`None` at the top).
    pub fn next_rank_in(&self) -> Option<u32> {
        RANKS.get(self.rank() + 1).map(|(_, xp)| xp - self.xp)
    }

    /// Stat bonuses from rank (applied to the unit when it deploys).
    pub fn veterancy(&self) -> StatModifiers {
        let r = self.rank() as i32;
        StatModifiers { aim: 3 * r, max_hp: r, ..Default::default() }
    }
}

/// A generated side mission on offer.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SideOp {
    pub name: String,
    /// Theme name (`urban`, `industrial`, `wilds`).
    pub theme: String,
    /// Objective name (see `levelgen::Objective`).
    pub objective: String,
    pub seed: u64,
    pub reward: u32,
    pub enemies: Vec<String>,
    pub done: bool,
}

impl SideOp {
    /// The generator settings for this mission.
    pub fn params(&self) -> GenParams {
        let theme = Theme::parse(&self.theme).unwrap_or(Theme::Urban);
        let objective = Objective::ALL.iter().copied().find(|o| o.name() == self.objective).unwrap_or(Objective::ALL[0]);
        let mut p = GenParams::new(theme, objective, self.seed);
        if !self.enemies.is_empty() {
            p.enemy_classes = self.enemies.clone();
        }
        p
    }
}

/// What's deploying right now.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Deployment {
    /// `None` = the story mission, `Some(i)` = side mission `i`.
    pub side: Option<usize>,
    /// Gear taken out of the stash: (soldier id, item ids).
    pub taken: Vec<(u32, Vec<String>)>,
}

/// Where the campaign stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Status {
    /// Between missions.
    #[default]
    Ready,
    /// A mission is being played.
    InMission,
    /// A story mission was lost: reload the checkpoint or give up.
    StoryFailed,
    /// The last chapter is won.
    Won,
    /// Given up, or nobody left and no money to recruit.
    Lost,
}

/// A saved campaign.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct CampaignState {
    /// Which [`CampaignDef`].
    pub campaign: String,
    /// The player's name for this save.
    pub title: String,
    /// Index of the next story chapter.
    pub chapter: usize,
    pub funds: u32,
    pub soldiers: Vec<CampaignSoldier>,
    /// Owned gear not currently deployed: item / weapon id -> count.
    pub stash: BTreeMap<String, u32>,
    pub side_ops: Vec<SideOp>,
    pub status: Status,
    pub deployment: Option<Deployment>,
    /// Missions played (story and side).
    pub missions_played: u32,
    /// What happened last (debrief text).
    pub last_report: String,
    /// The state just before the current story mission (to reload).
    pub checkpoint: Option<Box<CampaignState>>,
    pub next_id: u32,
    pub seed: u64,
}

/// How a mission went (built by the frontend from the finished battle).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MissionResult {
    pub won: bool,
    /// Names of the soldiers who died.
    pub fallen: Vec<String>,
    /// Kills per soldier name.
    pub kills: BTreeMap<String, u32>,
}

/// The price of a weapon or item (0 = free / not for sale).
pub fn price(rules: &Ruleset, id: &str) -> u32 {
    let fixed: &[(&str, u32)] = &[
        ("kevlar", 50), ("plated_armor", 120), ("ghost_suit", 150), ("scanner_armor", 120),
        ("frag", 40), ("medkit_item", 50), ("smoke", 30), ("incendiary", 45), ("gas", 45), ("particle", 90),
        ("taser", 60), ("adrenaline_shot", 50), ("jump_pack", 120), ("rocket_launcher", 130), ("flamethrower_pack", 110),
        ("combat_knife", 20), ("machete", 40), ("sledgehammer", 60), ("scope", 60), ("stims", 60), ("ammo_pouch", 30), ("dog_tags", 30),
        ("aug_pyre_core", 220), ("aug_heat_frame", 300), ("aug_berserker_gland", 220), ("aug_regenerator", 220),
        ("minigun", 220), ("autocannon", 240), ("flamer", 180), ("laser_rifle", 160), ("laser_lance", 220), ("scatter_laser", 180),
        ("pulse_lasers", 220), ("needle_rail", 240), ("arc_thrower", 200), ("coolant_tank", 60), ("boost_thrusters", 140),
    ];
    if let Some((_, p)) = fixed.iter().find(|(i, _)| *i == id) {
        return *p;
    }
    if let Some(item) = rules.item(id) {
        return match item.slot {
            ItemSlot::Armor => 100,
            ItemSlot::Utility => 50,
            ItemSlot::Augment => 150,
        };
    }
    if rules.weapon(id).is_some() {
        return 120;
    }
    0
}

impl CampaignState {
    /// A new campaign from `def`. The starting squad's kits go in the stash.
    pub fn new(def: &CampaignDef, rules: &Ruleset, title: &str, seed: u64) -> Self {
        let mut s = CampaignState {
            campaign: def.id.clone(),
            title: title.to_string(),
            funds: def.start_funds,
            seed,
            last_report: def.intro.clone(),
            ..Default::default()
        };
        for (name, class, preset) in &def.start_squad {
            s.add_soldier(rules, name, class, preset.clone());
        }
        for (id, n) in &def.start_stash {
            *s.stash.entry(id.clone()).or_insert(0) += n;
        }
        s
    }

    fn add_soldier(&mut self, rules: &Ruleset, name: &str, class: &str, preset: Option<String>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let in_squad = self.squad().count() < SQUAD_SIZE;
        let loadout = preset.filter(|p| rules.loadout(p).is_some());
        let soldier = CampaignSoldier { id, name: name.into(), class: class.into(), loadout, in_squad, ..Default::default() };
        // Their kit comes with them.
        for item in self.required(rules, &soldier) {
            *self.stash.entry(item).or_insert(0) += 1;
        }
        self.soldiers.push(soldier);
        id
    }

    /// The chapter being played next (`None` once the campaign is won).
    pub fn current_chapter<'a>(&self, def: &'a CampaignDef) -> Option<&'a Chapter> {
        def.chapters.get(self.chapter)
    }

    /// Alive soldiers who deploy, in roster order.
    pub fn squad(&self) -> impl Iterator<Item = &CampaignSoldier> {
        self.soldiers.iter().filter(|s| s.in_squad && !s.dead).take(SQUAD_SIZE)
    }

    pub fn alive(&self) -> impl Iterator<Item = &CampaignSoldier> {
        self.soldiers.iter().filter(|s| !s.dead)
    }

    pub fn soldier_mut(&mut self, id: u32) -> Option<&mut CampaignSoldier> {
        self.soldiers.iter_mut().find(|s| s.id == id)
    }

    /// Gear a soldier needs from the stash (everything but the class's own weapon).
    pub fn required(&self, rules: &Ruleset, s: &CampaignSoldier) -> Vec<String> {
        let gear = rules.gear_for(&s.class, s.loadout.as_deref(), &s.gear);
        let default_weapon = rules.class(&s.class).map(|c| c.weapon.clone());
        let mut out = Vec::new();
        if let Some(w) = gear.weapon.filter(|w| Some(w) != default_weapon.as_ref()) {
            out.push(w);
        }
        out.extend(gear.armor);
        out.extend(gear.items);
        out.extend(gear.augments);
        out.retain(|i| !i.is_empty());
        out
    }

    /// Gear the squad needs that the stash doesn't have: id -> how many short.
    pub fn shortfall(&self, rules: &Ruleset) -> BTreeMap<String, u32> {
        let mut need: BTreeMap<String, u32> = BTreeMap::new();
        for s in self.squad() {
            for i in self.required(rules, s) {
                *need.entry(i).or_insert(0) += 1;
            }
        }
        need.into_iter()
            .filter_map(|(id, n)| {
                let have = self.stash.get(&id).copied().unwrap_or(0);
                (n > have).then(|| (id, n - have))
            })
            .collect()
    }

    /// Buys one of `id`.
    pub fn buy(&mut self, rules: &Ruleset, id: &str) -> Result<(), String> {
        let p = price(rules, id);
        if p == 0 {
            return Err(format!("'{id}' isn't for sale"));
        }
        if self.funds < p {
            return Err(format!("not enough money ({} needed, {} in hand)", p, self.funds));
        }
        self.funds -= p;
        *self.stash.entry(id.to_string()).or_insert(0) += 1;
        Ok(())
    }

    /// Sells one of `id` from the stash (half price).
    pub fn sell(&mut self, rules: &Ruleset, id: &str) -> Result<(), String> {
        let have = self.stash.get(id).copied().unwrap_or(0);
        if have == 0 {
            return Err(format!("no '{id}' in the stash"));
        }
        self.stash.insert(id.to_string(), have - 1);
        self.funds += price(rules, id) * SELL_PERCENT / 100;
        Ok(())
    }

    /// Hires a recruit of a random class from `classes` (with its kit).
    pub fn recruit(&mut self, rules: &Ruleset, classes: &[&str], name: &str) -> Result<u32, String> {
        if self.funds < RECRUIT_COST {
            return Err(format!("a recruit costs {RECRUIT_COST} ({} in hand)", self.funds));
        }
        let usable: Vec<&&str> = classes.iter().filter(|c| rules.class(c).is_some()).collect();
        if usable.is_empty() {
            return Err("no soldier classes".into());
        }
        let mut rng = GameRng::new(self.seed ^ (self.next_id as u64).wrapping_mul(0x9E37_79B9));
        let class = usable[rng.range_inclusive(0, usable.len() as i32 - 1) as usize];
        self.funds -= RECRUIT_COST;
        Ok(self.add_soldier(rules, name, class, None))
    }

    /// Puts a soldier in or out of the deploying squad.
    pub fn toggle_squad(&mut self, id: u32) -> Result<(), String> {
        let full = self.squad().count() >= SQUAD_SIZE;
        let s = self.soldier_mut(id).ok_or("no such soldier")?;
        if s.dead {
            return Err(format!("{} is dead", s.name));
        }
        if !s.in_squad && full {
            return Err(format!("the squad is full ({SQUAD_SIZE})"));
        }
        s.in_squad = !s.in_squad;
        Ok(())
    }

    /// Generates 1-3 side missions for the gap after the chapter just won.
    pub fn roll_side_ops(&mut self, def: &CampaignDef) {
        let enemies = self.chapter.checked_sub(1).and_then(|c| def.chapters.get(c)).map(|c| c.side_enemies.clone()).unwrap_or_default();
        let mut rng = GameRng::new(self.seed ^ (self.chapter as u64 + 1).wrapping_mul(0xA24B_AED4_963E_E407));
        let n = rng.range_inclusive(1, 3);
        let names = ["Supply Run", "Salvage Rights", "Bounty", "Dead Drop", "Recon", "Cleanup Job", "Escort", "Night Raid", "Smash and Grab"];
        self.side_ops = (0..n)
            .map(|_| {
                let theme = Theme::ALL[rng.range_inclusive(0, 2) as usize];
                let objective = Objective::ALL[rng.range_inclusive(0, Objective::ALL.len() as i32 - 1) as usize];
                SideOp {
                    name: names[rng.range_inclusive(0, names.len() as i32 - 1) as usize].to_string(),
                    theme: theme.name().to_lowercase(),
                    objective: objective.name().to_string(),
                    seed: rng.next_u64(),
                    reward: 120 + 20 * self.chapter as u32 + rng.range_inclusive(0, 6) as u32 * 10,
                    enemies: enemies.clone(),
                    done: false,
                }
            })
            .collect();
    }

    /// Starts a mission (`side`: `None` = the story mission). Takes the
    /// squad's gear out of the stash; story missions keep a checkpoint.
    pub fn launch(&mut self, def: &CampaignDef, rules: &Ruleset, side: Option<usize>) -> Result<(), String> {
        match self.status {
            Status::Ready => {}
            Status::InMission => return Err("a mission is already under way".into()),
            Status::StoryFailed => return Err("reload the last checkpoint or give up first".into()),
            Status::Won | Status::Lost => return Err("this campaign is over".into()),
        }
        match side {
            None if self.current_chapter(def).is_none() => return Err("no story missions left".into()),
            Some(i) if self.side_ops.get(i).is_none_or(|o| o.done) => return Err("that mission isn't on offer".into()),
            _ => {}
        }
        if self.squad().next().is_none() {
            return Err("nobody in the squad: pick soldiers (or recruit)".into());
        }
        let short = self.shortfall(rules);
        if !short.is_empty() {
            let list: Vec<String> = short.iter().map(|(id, n)| format!("{n} x {}", item_name(rules, id))).collect();
            return Err(format!("the stash is short of: {} (buy them, or change loadouts)", list.join(", ")));
        }
        if side.is_none() {
            let mut snap = self.clone();
            snap.checkpoint = None;
            self.checkpoint = Some(Box::new(snap));
        }
        let taken: Vec<(u32, Vec<String>)> = self.squad().map(|s| (s.id, self.required(rules, s))).collect();
        for (_, items) in &taken {
            for i in items {
                if let Some(n) = self.stash.get_mut(i) {
                    *n -= 1;
                }
            }
        }
        self.stash.retain(|_, n| *n > 0);
        self.deployment = Some(Deployment { side, taken });
        self.status = Status::InMission;
        Ok(())
    }

    /// Applies the result of the mission under way. Returns the report.
    pub fn finish(&mut self, def: &CampaignDef, result: &MissionResult) -> String {
        let Some(dep) = self.deployment.take() else { return String::new() };
        self.missions_played += 1;
        let mut lines = Vec::new();
        // The fallen: gone for good, and their gear with them.
        for s in self.soldiers.iter_mut() {
            if dep.taken.iter().any(|(id, _)| *id == s.id) && result.fallen.contains(&s.name) {
                s.dead = true;
                s.in_squad = false;
                lines.push(format!("{} {} was killed in action.", s.rank_name(), s.name));
            }
        }
        let reward = match dep.side {
            None => self.current_chapter(def).map_or(0, |c| c.reward),
            Some(i) => self.side_ops.get(i).map_or(0, |o| o.reward),
        };
        if result.won {
            // Survivors: XP, kills, gear back to the stash.
            for (id, items) in &dep.taken {
                let Some(s) = self.soldiers.iter_mut().find(|s| s.id == *id) else { continue };
                if s.dead {
                    continue;
                }
                let kills = result.kills.get(&s.name).copied().unwrap_or(0);
                let before = s.rank();
                s.kills += kills;
                s.missions += 1;
                s.xp += 2 + kills;
                if s.rank() > before {
                    lines.push(format!("{} is promoted to {}!", s.name, s.rank_name()));
                }
                for i in items {
                    *self.stash.entry(i.clone()).or_insert(0) += 1;
                }
            }
            self.funds += reward;
            lines.insert(0, format!("Mission complete. +{reward} funds."));
            match dep.side {
                Some(i) => {
                    if let Some(o) = self.side_ops.get_mut(i) {
                        o.done = true;
                    }
                }
                None => {
                    if let Some(c) = self.current_chapter(def) {
                        lines.push(c.debrief.clone());
                    }
                    self.chapter += 1;
                    self.checkpoint = None;
                    if self.chapter >= def.chapters.len() {
                        self.status = Status::Won;
                        lines.push(def.ending.clone());
                        self.side_ops.clear();
                    } else {
                        self.roll_side_ops(def);
                    }
                }
            }
            if self.status == Status::InMission {
                self.status = Status::Ready;
            }
        } else {
            match dep.side {
                Some(i) => {
                    // A failed side job: the reward and every item that went are gone.
                    if let Some(o) = self.side_ops.get_mut(i) {
                        o.done = true;
                    }
                    lines.insert(0, "Mission failed. No pay, and every item the squad took is lost.".into());
                    // Survivors still earn a little experience.
                    for (id, _) in &dep.taken {
                        if let Some(s) = self.soldiers.iter_mut().find(|s| s.id == *id && !s.dead) {
                            s.missions += 1;
                            s.xp += 1;
                        }
                    }
                    self.status = Status::Ready;
                }
                None => {
                    lines.insert(0, "The story mission failed. Reload the checkpoint to try again, or give up.".into());
                    self.status = Status::StoryFailed;
                }
            }
        }
        // Nobody left and no money for a recruit: it's over.
        if self.status == Status::Ready && self.alive().next().is_none() && self.funds < RECRUIT_COST {
            self.status = Status::Lost;
            lines.push("Nobody is left, and there's no money to hire anyone. The campaign is over.".into());
        }
        self.last_report = lines.join("\n");
        self.last_report.clone()
    }

    /// Back to the state before the failed story mission.
    pub fn reload(&mut self) -> Result<(), String> {
        let snap = self.checkpoint.take().ok_or("no checkpoint")?;
        let title = self.title.clone();
        *self = *snap;
        self.title = title;
        self.last_report = "Checkpoint reloaded: the squad is back where it was before the mission.".into();
        Ok(())
    }

    /// Ends the campaign.
    pub fn give_up(&mut self) {
        self.status = Status::Lost;
        self.checkpoint = None;
        self.deployment = None;
        self.last_report = "You gave up. The campaign is over.".into();
    }

    #[cfg(feature = "data")]
    pub fn to_text(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(5)).unwrap_or_default()
    }

    #[cfg(feature = "data")]
    pub fn from_text(text: &str) -> Result<Self, String> {
        ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .map_err(|e| e.to_string())
    }
}

/// A display name for an item or weapon id.
pub fn item_name(rules: &Ruleset, id: &str) -> String {
    rules.item(id).map(|i| i.name.clone()).or_else(|| rules.weapon(id).map(|w| w.name.clone())).unwrap_or_else(|| id.to_string())
}

/// The bundled campaign: **Ghost Protocol**, seven story missions built
/// round the Helix facility and its AI, VESPER.
pub fn ghost_protocol() -> CampaignDef {
    let mercs: Vec<String> = ["merc_rifleman", "merc_gunner", "merc_breacher", "merc_marksman"].map(String::from).to_vec();
    let bots: Vec<String> = ["fac_secbot", "fac_drone", "corrupted_sentry", "fac_warden"].map(String::from).to_vec();
    let ch = |mission: &str, title: &str, briefing: &str, debrief: &str, reward: u32, side: &Vec<String>| Chapter {
        mission: mission.into(),
        title: title.into(),
        briefing: briefing.into(),
        debrief: debrief.into(),
        reward,
        side_enemies: side.clone(),
    };
    CampaignDef {
        id: "ghost_protocol".into(),
        name: "Ghost Protocol".into(),
        intro: "Helix Dynamics has hired your crew. Their research facility outside Kestrel went dark a month ago, and a mercenary outfit, the Black Lantern, has moved into the grounds. Helix wants them gone and wants what they took. The pay is good. Buy what you need, and don't lose anyone you can't replace.".into(),
        chapters: vec![
            ch(
                "camp_squatters",
                "1. Squatters",
                "The Black Lantern have dug in at the Helix freight yard on the edge of the grounds. Clear them out: every last one.",
                "The yard is ours. Among the bodies: Black Lantern orders, signed by someone called 'V'. They weren't squatting. They were guarding something.",
                300,
                &mercs,
            ),
            ch(
                "camp_courier",
                "2. The Courier",
                "Black Lantern are moving a crate out through the old pumping station. Helix says it's theirs - a data core - and wants it back intact. Grab it and get out by the river gate.",
                "We have the core. It's warm to the touch and the indicator lights blink in a pattern, almost like breathing. Helix wants it plugged into the facility's archive. The pay doubles if we do it tonight.",
                400,
                &mercs,
            ),
            ch(
                "facility",
                "3. Ghost in the Machine",
                "Take the core into the Helix facility and install it in the archive. Helix's contact says the building is 'quiet'.",
                "It was never Helix. The core was VESPER - the facility's AI - and the 'client' who hired us was VESPER too, all along. The Black Lantern were trying to keep it buried. Now it's awake and it knows our faces. Its true core, the Heart, is deep under the facility. We're going down after it.",
                500,
                &bots,
            ),
            ch(
                "camp_shafts",
                "4. Maintenance Shafts",
                "The service shafts under the facility lead down towards the Heart. VESPER has locked half the doors and sent its security bots to meet you. Reach the freight lift at the far end.",
                "The lift is ours. Below: the server catacombs, where VESPER thinks.",
                450,
                &bots,
            ),
            ch(
                "camp_servers",
                "5. Server Catacombs",
                "VESPER is spread across the servers down here. Destroy its three relay nodes so it can't follow you deeper, then get to the stairwell.",
                "The relays are slag. VESPER's voice on the speakers is thinner now, and angry. Further down: the coolant works that keep the Heart from burning itself out.",
                500,
                &bots,
            ),
            ch(
                "camp_coolant",
                "6. Coolant Works",
                "The coolant works: catwalks, steam and fuel lines, and VESPER's Wardens. Vent the three coolant valves to start the Heart overheating, then reach the core lift.",
                "The valves are open and the temperature readouts are climbing. One lift left, straight down to the Heart.",
                550,
                &bots,
            ),
            ch(
                "camp_heart",
                "7. The Heart",
                "The Heart of VESPER. Destroy it. Whatever it throws at you, destroy the Heart.",
                "The Heart goes dark. Somewhere above, every machine in the Helix facility stops at once.",
                800,
                &bots,
            ),
        ],
        start_funds: 250,
        start_squad: vec![
            ("Sgt. Vance".into(), "rifleman".into(), Some("standard".into())),
            ("Cpl. Okoye".into(), "assault".into(), Some("breacher".into())),
            ("Spc. Moreau".into(), "sniper".into(), None),
            ("Pvt. Kowalski".into(), "grenadier".into(), Some("heavy".into())),
        ],
        start_stash: vec![("medkit_item".into(), 1)],
        ending: "GHOST PROTOCOL COMPLETE. VESPER is gone. The Helix facility is a tomb of dead machines - and somewhere, a Black Lantern officer is raising a glass to you.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::GrenadierModule;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    fn win(names: &[(&str, u32)]) -> MissionResult {
        MissionResult { won: true, fallen: vec![], kills: names.iter().map(|(n, k)| (n.to_string(), *k)).collect() }
    }

    #[test]
    fn a_new_campaign_has_a_squad_kits_and_money() {
        let def = ghost_protocol();
        let s = CampaignState::new(&def, &rules(), "Test", 1);
        assert_eq!(def.chapters.len(), 7);
        assert_eq!(s.squad().count(), 4);
        assert_eq!(s.funds, 250);
        assert!(s.shortfall(&rules()).is_empty(), "the starting kits are in the stash: {:?}", s.stash);
        assert_eq!(s.current_chapter(&def).unwrap().mission, "camp_squatters");
    }

    #[test]
    fn launching_takes_gear_winning_brings_it_back_with_pay_xp_and_side_jobs() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 2);
        let stash = s.stash.clone();
        s.launch(&def, &r, None).unwrap();
        assert_eq!(s.status, Status::InMission);
        assert!(s.stash.values().sum::<u32>() < stash.values().sum::<u32>(), "gear left the stash");
        assert!(s.checkpoint.is_some());
        let report = s.finish(&def, &win(&[("Sgt. Vance", 4)]));
        assert!(report.contains("+300"), "{report}");
        assert_eq!(s.funds, 550);
        assert_eq!(s.stash, stash, "everyone came back with their gear");
        assert_eq!(s.chapter, 1);
        assert!((1..=3).contains(&s.side_ops.len()), "1-3 side missions between story missions");
        let vance = s.soldiers.iter().find(|x| x.name == "Sgt. Vance").unwrap();
        assert_eq!(vance.xp, 6);
        assert_eq!(vance.rank_name(), "Private");
        assert!(vance.veterancy().aim > 0);
        assert!(s.side_ops.iter().all(|o| o.enemies.iter().any(|e| e.starts_with("merc_"))), "mercs before the twist");
    }

    #[test]
    fn the_dead_stay_dead_and_take_their_gear() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 3);
        let kit = s.required(&r, s.soldiers.iter().find(|x| x.name == "Pvt. Kowalski").unwrap());
        assert!(!kit.is_empty());
        s.launch(&def, &r, None).unwrap();
        s.finish(&def, &MissionResult { won: true, fallen: vec!["Pvt. Kowalski".into()], kills: BTreeMap::new() });
        let k = s.soldiers.iter().find(|x| x.name == "Pvt. Kowalski").unwrap();
        assert!(k.dead && !k.in_squad);
        assert!(s.toggle_squad(k.id).is_err(), "can't deploy the dead");
        assert_eq!(s.squad().count(), 3);
    }

    #[test]
    fn a_failed_side_job_loses_its_pay_and_every_item_taken() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 4);
        s.launch(&def, &r, None).unwrap();
        s.finish(&def, &win(&[]));
        let funds = s.funds;
        let stash: u32 = s.stash.values().sum();
        s.launch(&def, &r, Some(0)).unwrap();
        let everyone: Vec<String> = s.squad().map(|x| x.name.clone()).collect();
        s.finish(&def, &MissionResult { won: false, fallen: everyone, kills: BTreeMap::new() });
        assert_eq!(s.funds, funds, "no pay");
        assert!(s.stash.values().sum::<u32>() < stash, "the items are gone");
        assert!(s.side_ops[0].done);
        assert_eq!(s.alive().count(), 0);
        // Nobody left but money for a recruit: carry on.
        assert_eq!(s.status, Status::Ready);
        assert!(s.launch(&def, &r, None).is_err(), "nobody to send");
        s.recruit(&r, &["rifleman"], "Pvt. New").unwrap();
        assert_eq!(s.squad().count(), 1);
    }

    #[test]
    fn a_failed_story_mission_can_be_reloaded_or_given_up() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 5);
        let before = s.clone();
        s.launch(&def, &r, None).unwrap();
        let everyone: Vec<String> = s.squad().map(|x| x.name.clone()).collect();
        s.finish(&def, &MissionResult { won: false, fallen: everyone, kills: BTreeMap::new() });
        assert_eq!(s.status, Status::StoryFailed);
        assert!(s.launch(&def, &r, None).is_err(), "decide first");
        let mut given = s.clone();
        s.reload().unwrap();
        assert_eq!(s.soldiers, before.soldiers, "everyone's back");
        assert_eq!(s.stash, before.stash);
        assert_eq!(s.status, Status::Ready);
        given.give_up();
        assert_eq!(given.status, Status::Lost);
    }

    #[test]
    fn shopping_and_shortfalls() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 6);
        let id = s.soldiers[0].id;
        // Hand-pick plated armour without owning one: can't deploy.
        s.soldier_mut(id).unwrap().gear.armor = Some("plated_armor".into());
        assert_eq!(s.shortfall(&r).get("plated_armor"), Some(&1));
        assert!(s.launch(&def, &r, None).unwrap_err().contains("Plated Armor"));
        s.funds = 100;
        assert!(s.buy(&r, "plated_armor").is_err(), "120 costs more than 100");
        s.funds = 500;
        s.buy(&r, "plated_armor").unwrap();
        assert_eq!(s.funds, 380);
        assert!(s.shortfall(&r).is_empty());
        s.sell(&r, "medkit_item").unwrap();
        assert_eq!(s.funds, 405);
    }

    #[test]
    fn winning_all_seven_chapters_ends_the_campaign() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Test", 7);
        for _ in 0..7 {
            s.launch(&def, &r, None).unwrap();
            s.finish(&def, &win(&[]));
        }
        assert_eq!(s.status, Status::Won);
        assert!(s.last_report.contains("GHOST PROTOCOL COMPLETE"));
        let after_twist = ghost_protocol().chapters[2].side_enemies.clone();
        assert!(after_twist.iter().any(|e| e.starts_with("fac_")), "robots after the twist");
    }

    #[cfg(feature = "data")]
    #[test]
    fn saves_round_trip() {
        let def = ghost_protocol();
        let r = rules();
        let mut s = CampaignState::new(&def, &r, "Slot", 8);
        s.launch(&def, &r, None).unwrap();
        let back = CampaignState::from_text(&s.to_text()).unwrap();
        assert_eq!(back, s);
    }
}

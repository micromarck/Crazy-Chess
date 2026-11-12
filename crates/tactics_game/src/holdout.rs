//! Hold Out missions (`holdout_*`): no win, so the score is how long the squad
//! lasted. The best round per mission is saved and shown.

use std::collections::BTreeMap;

use bevy::prelude::*;
use tactics_core::unit::Team;

use crate::FrameSet;
use crate::hud::LogMessages;
use crate::state::BattleState;

const FILE: &str = "tbsium_holdout_best.txt";

pub struct HoldoutPlugin;

impl Plugin for HoldoutPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HoldoutBest::load()).add_systems(Update, record.in_set(FrameSet::Logic));
    }
}

/// Is this a Hold Out mission?
pub fn is_holdout(mission: Option<&str>) -> bool {
    mission.is_some_and(|m| m.starts_with("holdout_"))
}

/// Best round reached per Hold Out mission.
#[derive(Resource, Default)]
pub struct HoldoutBest {
    pub best: BTreeMap<String, u32>,
    recorded: Option<u64>,
}

impl HoldoutBest {
    fn load() -> Self {
        let best = crate::read_save(FILE)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('=').and_then(|(k, v)| Some((k.trim().to_string(), v.trim().parse().ok()?))))
            .collect();
        Self { best, recorded: None }
    }

    fn save(&self) {
        let text: String = self.best.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        if let Err(e) = crate::write_save(FILE, &text) {
            warn!("couldn't save Hold Out records: {e}");
        }
    }

    pub fn get(&self, mission: &str) -> u32 {
        self.best.get(mission).copied().unwrap_or(0)
    }
}

/// When the squad falls in a Hold Out mission: the score, and a new record?
fn record(mut best: ResMut<HoldoutBest>, state: Res<BattleState>, playback: Res<crate::playback::Playback>, mut log: ResMut<LogMessages>) {
    let b = &state.battle;
    if !is_holdout(state.mission.as_deref()) || !b.is_over() || playback.is_busy() || best.recorded == Some(state.battle_id) {
        return;
    }
    best.recorded = Some(state.battle_id);
    let id = state.mission.clone().unwrap_or_default();
    let reached = b.round();
    let previous = best.get(&id);
    if b.living(Team::Player).next().is_none() {
        if reached > previous {
            best.best.insert(id, reached);
            best.save();
            log.push(format!("The squad held out to round {reached} - a new best (was {previous})!"));
        } else {
            log.push(format!("The squad held out to round {reached} (best: {previous})."));
        }
    }
}

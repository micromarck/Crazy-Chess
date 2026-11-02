//! Bevy resources wrapping the core battle, plus the "current selection".
//!
//! [`BattleState`] is the single owner of the [`Battle`]. Its
//! [`apply`](BattleState::apply) method is the one place where actions enter
//! the rules engine; it forwards the resulting events to
//! [`Playback`](crate::playback::Playback) and bumps a revision counter that
//! other systems use to invalidate caches. Live content edits (the F1 editor)
//! also bump the revision.

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::content::build_ruleset;
use crate::playback::Playback;

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        let (rules, load_log) = build_ruleset();
        for line in &load_log {
            info!("{line}");
        }
        app.insert_resource(BattleState::new(scenario::demo(&rules, DEFAULT_SEED)))
            .insert_resource(ModuleLoadLog(load_log))
            .init_resource::<Selection>()
            .add_message::<PlayerCommand>()
            .add_message::<BattleReset>();
    }
}

/// Seed of the first battle; restarts pick new seeds.
pub const DEFAULT_SEED: u64 = 0x5EED;

/// Messages produced while loading modules (shown in the combat log at start).
#[derive(Resource, Default)]
pub struct ModuleLoadLog(pub Vec<String>);

/// The authoritative battle and a change counter.
#[derive(Resource)]
pub struct BattleState {
    pub battle: Battle,
    /// Incremented every time the battle or its content changes.
    pub revision: u64,
    /// Seed of the current battle (for restarts and bug reports).
    pub seed: u64,
    /// Id of the mission being played (`None` = skirmish / demo).
    pub mission: Option<String>,
    /// Running totals for the end-of-mission summary.
    pub stats: MissionStats,
}

/// What the squad did this battle (shown on the game-over screen).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MissionStats {
    /// Attack rolls by the squad (area attacks count each victim).
    pub shots: u32,
    pub hits: u32,
    pub crits: u32,
    pub kills: u32,
    pub damage_dealt: i32,
    pub damage_taken: i32,
    pub soldiers_lost: u32,
    /// Kills per soldier name (for the roster's service records).
    pub kills_by: std::collections::BTreeMap<String, u32>,
    /// The squad member who acted last (credited with kills that follow).
    last_actor: Option<UnitId>,
}

impl MissionStats {
    /// Counts the squad-relevant parts of `events` (call before they're
    /// played back; `battle` is the state after them).
    pub fn record(&mut self, battle: &Battle, events: &[GameEvent]) {
        let team = |id: UnitId| battle.unit(id).map(|u| u.team);
        for e in events {
            match e {
                GameEvent::AbilityUsed { unit, .. } => self.last_actor = Some(*unit),
                GameEvent::AttackResolved { attacker, target, outcome, .. } => {
                    self.last_actor = Some(*attacker);
                    let damage = match outcome {
                        ShotOutcome::Hit { damage, .. } => *damage,
                        ShotOutcome::Miss => 0,
                    };
                    if team(*attacker) == Some(Team::Player) && team(*target) == Some(Team::Enemy) {
                        self.shots += 1;
                        if let ShotOutcome::Hit { critical, .. } = outcome {
                            self.hits += 1;
                            self.crits += *critical as u32;
                        }
                        self.damage_dealt += damage;
                    } else if team(*target) == Some(Team::Player) {
                        self.damage_taken += damage;
                    }
                }
                GameEvent::UnitDamaged { unit, amount } => match team(*unit) {
                    Some(Team::Player) => self.damage_taken += amount,
                    Some(Team::Enemy) => self.damage_dealt += amount,
                    None => {}
                },
                GameEvent::UnitKilled { unit } => match team(*unit) {
                    Some(Team::Player) => self.soldiers_lost += 1,
                    Some(Team::Enemy) => {
                        self.kills += 1;
                        if let Some(killer) = self.last_actor.and_then(|a| battle.unit(a)).filter(|u| u.team == Team::Player) {
                            *self.kills_by.entry(killer.name.clone()).or_insert(0) += 1;
                        }
                    }
                    None => {}
                },
                _ => {}
            }
        }
    }

    /// One-line summary for the HUD.
    pub fn summary(&self, rounds: u32) -> String {
        let accuracy = (self.hits * 100).checked_div(self.shots).map_or("-".into(), |a| format!("{a}%"));
        format!(
            "Rounds {rounds}   Kills {}   Accuracy {accuracy} ({}/{}, {} crits)\nDamage dealt {}   taken {}   Soldiers lost {}",
            self.kills, self.hits, self.shots, self.crits, self.damage_dealt, self.damage_taken, self.soldiers_lost
        )
    }
}

impl BattleState {
    pub fn new(battle: Battle) -> Self {
        Self { battle, revision: 0, seed: DEFAULT_SEED, mission: None, stats: MissionStats::default() }
    }

    /// Applies an action and queues its events for animation.
    /// Returns the error (also logged) if the action was illegal.
    pub fn apply(&mut self, action: Action, playback: &mut Playback) -> Result<(), ActionError> {
        match self.battle.apply(action.clone()) {
            Ok(events) => {
                self.revision += 1;
                self.stats.record(&self.battle, &events);
                playback.enqueue(events);
                Ok(())
            }
            Err(err) => {
                warn!("rejected {action:?}: {err}");
                Err(err)
            }
        }
    }

    /// Mutable access to the content for live tuning; marks the state changed.
    pub fn rules_mut(&mut self) -> &mut Ruleset {
        self.revision += 1;
        self.battle.rules_mut()
    }
}

/// What the player is currently looking at / controlling.
#[derive(Resource, Default)]
pub struct Selection {
    /// The friendly unit receiving orders.
    pub unit: Option<UnitId>,
    /// The tile under the mouse/finger, if it's on the map.
    pub hovered: Option<GridPos>,
    /// Ability waiting for a target (targeting mode), if any.
    pub pending: Option<AbilityId>,
    /// Target picked for `pending`, waiting for confirmation (second click,
    /// Space, or pressing the ability again).
    pub aim: Option<Target>,
    /// Cached movement range of `unit`.
    pub range: Option<ReachableSet>,
    range_key: Option<(UnitId, u64)>,
}

impl Selection {
    /// Recomputes the cached movement range if the unit or battle changed.
    pub fn refresh_range(&mut self, state: &BattleState) {
        let key = self.unit.map(|u| (u, state.revision));
        if key != self.range_key {
            self.range_key = key;
            self.range = self.unit.map(|u| state.battle.movement_range(u));
        }
    }

    /// Selects a unit (cancelling any targeting in progress).
    pub fn select(&mut self, unit: Option<UnitId>) {
        if unit != self.unit {
            self.pending = None;
            self.aim = None;
        }
        self.unit = unit;
    }
}

/// High-level player intents from keyboard shortcuts or HUD buttons. Routing
/// both through one message keeps desktop and touch controls identical.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum PlayerCommand {
    NextUnit,
    PreviousUnit,
    /// Select this soldier (roster click) and bring the camera to them.
    Select(UnitId),
    /// Centre the camera on the selected unit.
    FocusSelected,
    /// Fire the aimed ability.
    Confirm,
    /// Enter: confirm if aiming, otherwise end the turn.
    ConfirmOrEndTurn,
    /// Use (self-targeted) or start targeting (others) an ability of the
    /// selected unit.
    UseAbility(AbilityId),
    /// Use the selected unit's n-th ability (hotkeys 1-9).
    AbilitySlot(usize),
    CancelTargeting,
    EndTurn,
    /// Restart the current mission (or a new skirmish).
    Restart,
    /// Start a mission from `assets/missions/` by id.
    StartMission(String),
    /// Start a procedural skirmish.
    Skirmish,
}

/// Sent after the battle has been replaced, so visuals can be rebuilt.
#[derive(Message, Debug, Clone, Copy)]
pub struct BattleReset;

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn stats_count_the_squads_shots_kills_and_losses() {
        let (rules, _) = crate::content::build_ruleset();
        let mut b = Battle::new(Map::from_ascii("......"), rules, 1);
        let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
        let e = b.spawn("sectoid", "E", Team::Enemy, GridPos::new(5, 0)).unwrap();
        let mut s = MissionStats::default();
        let shot = |hit: bool| GameEvent::AttackResolved {
            attacker: a,
            ability: "shoot".into(),
            target: e,
            origin: GridPos::new(0, 0),
            hit_chance: 50,
            outcome: if hit { ShotOutcome::Hit { damage: 3, critical: true } } else { ShotOutcome::Miss },
            reaction: false,
        };
        s.record(&b, &[shot(true), shot(false), GameEvent::UnitKilled { unit: e }, GameEvent::UnitDamaged { unit: a, amount: 2 }]);
        assert_eq!((s.shots, s.hits, s.crits, s.kills, s.damage_dealt, s.damage_taken), (2, 1, 1, 1, 3, 2));
        assert!(s.summary(4).contains("Accuracy 50%"));
    }
}

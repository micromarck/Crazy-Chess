//! Morale: how afraid units are, and what fear does.
//!
//! Every unit has a **fear** value from 0 to 100. Bad things raise it (taking
//! hits, being shot at, allies dying nearby, being flanked or outnumbered);
//! good things lower it (kills, hunkering down, a quiet turn in cover).
//! Gains are scaled by the unit's **bravery** (50 = normal, 100 = immune to
//! gains, 0 = double). Classes marked `fearless` (drones, robots) never feel
//! fear at all.
//!
//! | level | fear | effect |
//! |---|---|---|
//! | Steady | < `shaken_at` | none |
//! | Shaken | ≥ `shaken_at` | `shaken_aim` to hit; the AI gets cautious |
//! | Panicked | ≥ `panicked_at` | `panicked_aim` to hit, loses `panicked_ap_loss` AP per turn; the AI flees or hunkers |
//!
//! The rules are part of [`CombatRules`](super::CombatRules) (`morale`), so
//! data modules can retune or disable them.

/// How afraid a unit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum FearLevel {
    #[default]
    Steady,
    Shaken,
    Panicked,
}

impl FearLevel {
    pub fn name(self) -> &'static str {
        match self {
            FearLevel::Steady => "Steady",
            FearLevel::Shaken => "Shaken",
            FearLevel::Panicked => "Panicked",
        }
    }
}

/// Fear tuning. All gains are before bravery scaling.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct MoraleRules {
    /// `false` turns the whole system off.
    pub enabled: bool,
    pub shaken_at: i32,
    pub panicked_at: i32,
    /// An ally dies within `near_radius` tiles...
    pub ally_killed_near: i32,
    /// ...or anywhere else.
    pub ally_killed_far: i32,
    pub near_radius: f32,
    /// Per hit point lost.
    pub per_damage: i32,
    /// An attack against you missed (suppression).
    pub shot_at: i32,
    /// Turn start: a visible enemy flanks you.
    pub flanked: i32,
    /// Turn start: per visible enemy beyond the number of allies near you.
    pub outnumbered: i32,
    /// Fear lost by whoever lands a kill.
    pub kill_relief: i32,
    /// Fear lost by the killer's allies within `near_radius`.
    pub ally_kill_relief: i32,
    /// Fear lost at the start of each of your turns...
    pub recovery: i32,
    /// ...plus this when no enemy can see you.
    pub hidden_recovery: i32,
    /// Fear lost when hunkering down (any Defend ability).
    pub hunker_relief: i32,
    /// Aim change while shaken / panicked (negative).
    pub shaken_aim: i32,
    pub panicked_aim: i32,
    /// Action points lost at turn start while panicked (never below 1).
    pub panicked_ap_loss: u8,
}

impl Default for MoraleRules {
    fn default() -> Self {
        Self {
            enabled: true,
            shaken_at: 40,
            panicked_at: 75,
            ally_killed_near: 25,
            ally_killed_far: 10,
            near_radius: 6.0,
            per_damage: 4,
            shot_at: 4,
            flanked: 10,
            outnumbered: 4,
            kill_relief: 20,
            ally_kill_relief: 6,
            recovery: 8,
            hidden_recovery: 8,
            hunker_relief: 15,
            shaken_aim: -10,
            panicked_aim: -25,
            panicked_ap_loss: 1,
        }
    }
}

impl MoraleRules {
    pub fn level(&self, fear: i32) -> FearLevel {
        if fear >= self.panicked_at {
            FearLevel::Panicked
        } else if fear >= self.shaken_at {
            FearLevel::Shaken
        } else {
            FearLevel::Steady
        }
    }

    /// Aim modifier for a fear value.
    pub fn aim_modifier(&self, fear: i32) -> i32 {
        if !self.enabled {
            return 0;
        }
        match self.level(fear) {
            FearLevel::Steady => 0,
            FearLevel::Shaken => self.shaken_aim,
            FearLevel::Panicked => self.panicked_aim,
        }
    }

    /// A fear gain after bravery (50 = ×1, 100 = ×0, 0 = ×2). Relief
    /// (negative amounts) is not scaled.
    pub fn scaled(&self, amount: i32, bravery: i32) -> i32 {
        if amount <= 0 {
            amount
        } else {
            (amount * (100 - bravery).clamp(0, 200) + 25) / 50
        }
    }
}

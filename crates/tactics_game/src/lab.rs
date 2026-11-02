//! The **Character Lab**: a test stage for models and animations.
//!
//! Start it with **F7** or from the Missions list (`assets/missions/lab.mission.xml`).
//! Its HUD panel (in `assets/ui/hud.html`, shown while `lab.active`) lists
//! every model: built-in rigs, glTF models from modules, and characters
//! dropped into `assets/characters/`. Click one to put it on the test subject,
//! then click any slot to play it. **Walk** toggles the walk loop so you can
//! check the cycle in place; right-drag to orbit around it.
//!
//! UI commands (from `data-command`): `model:<id>`, `anim:<slot>`, `lab_walk`.

use bevy::prelude::*;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::animation::AnimCommand;
use crate::characters::RefreshUnitVisuals;
use crate::hud::LogMessages;
use crate::missions::StartBattle;
use crate::state::{BattleState, Selection};
use crate::units::UnitEntities;

pub struct LabPlugin;

impl Plugin for LabPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LabCommand>()
            .init_resource::<LabState>()
            .add_systems(Update, (lab_hotkey, run_lab_commands).chain().in_set(FrameSet::Logic));
    }
}

/// Mission id of the lab.
pub const LAB_MISSION: &str = "lab";

/// Requests from the lab panel.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum LabCommand {
    /// Show this model on the selected unit.
    SetModel(String),
    /// Play this slot on the selected unit.
    Play(String),
    /// Toggle the walk loop on the selected unit.
    ToggleWalk,
}

#[derive(Resource, Default)]
pub struct LabState {
    pub walking: bool,
}

fn lab_hotkey(keys: Res<ButtonInput<KeyCode>>, mut start: MessageWriter<StartBattle>) {
    if keys.just_pressed(KeyCode::F7) {
        start.write(StartBattle::Mission(LAB_MISSION.into()));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_lab_commands(
    mut commands_in: MessageReader<LabCommand>,
    mut state: ResMut<BattleState>,
    selection: Res<Selection>,
    mut lab: ResMut<LabState>,
    mut anim: MessageWriter<AnimCommand>,
    mut refresh: MessageWriter<RefreshUnitVisuals>,
    mut log: ResMut<LogMessages>,
    entities: Res<UnitEntities>,
) {
    for c in commands_in.read() {
        let Some(unit) = selection.unit.or_else(|| state.battle.living(Team::Player).next().map(|u| u.id)) else {
            log.push("Lab: no unit to show".into());
            continue;
        };
        match c {
            LabCommand::SetModel(id) => {
                state.battle.set_unit_model(unit, id.clone());
                state.revision += 1;
                lab.walking = false;
                refresh.write(RefreshUnitVisuals);
                log.push(format!("Lab: showing model '{id}'"));
            }
            LabCommand::Play(slot) => {
                if entities.0.contains_key(&unit) {
                    anim.write(AnimCommand::OneShot { unit, slot: slot.clone() });
                }
            }
            LabCommand::ToggleWalk => {
                lab.walking = !lab.walking;
                anim.write(AnimCommand::Walking { unit, on: lab.walking });
            }
        }
    }
}

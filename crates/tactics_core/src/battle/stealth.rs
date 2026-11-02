//! Mission mechanics beyond plain combat. Each one is small, data-driven
//! (switched on by mission XML attributes) and scriptable:
//!
//! | Mechanic | Mission XML | Scripts |
//! |---|---|---|
//! | **Dormant pods** — enemies hold position until their pod is seen, one of them is hurt or the alarm sounds | `<unit dormant="true" pod="a">` | `wake(unit)`, `wake_all()`, `unit_dormant(unit)` |
//! | **Patrols** — dormant units walk waypoints (one AP per turn) | `<unit patrol="x,y x,y">` | |
//! | **Concealment** — unalerted enemies only notice soldiers within `detection` tiles; being noticed or attacking raises the alarm | `<mission concealment="true" detection="5">` | `concealed()`, `alarm()`, `raise_alarm()`, `on="alarm"` hook |
//! | **Doors** — block movement and sight; open when a unit steps next to them unless locked | `<door tag="d" x y width height locked="true">` | `open_door`, `close_door`, `lock_door`, `on="opened"` hook |
//! | **Fog** — the player only sees enemies in sight (a presentation flag) | `<mission fog="true">` | |
//! | **AI goals** — AI units head somewhere while no enemy is in sight | | `ai_goal(team, x, y)`, `clear_ai_goal(team)` |
//! | **Objective** — HUD text | | `objective(text)` |
//!
//! Everything here emits ordinary [`GameEvent`]s so the frontend can animate it.

use super::Battle;
use super::world::{Prop, PropId, Trigger};
use crate::event::GameEvent;
use crate::grid::{CoverLevel, GridPos, Terrain};
use crate::mission::{HookEvent, MissionDef};
use crate::unit::{Team, UnitId};
use crate::visibility;

/// Concealment / alarm state of a battle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stealth {
    /// The squad hasn't been noticed yet (only when the mission enables it).
    pub concealed: bool,
    /// The alarm has been raised (concealment broken; every enemy active).
    pub alarm: bool,
    /// How close (tiles) an unalerted enemy notices a concealed soldier.
    pub detection: f32,
}

impl Default for Stealth {
    fn default() -> Self {
        Self { concealed: false, alarm: false, detection: 5.0 }
    }
}

/// A zone drawn on the board (evac pad, objective, danger area).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkedZone {
    pub tag: String,
    /// Inclusive corners (grid coordinates).
    pub min: GridPos,
    pub max: GridPos,
    /// `evac`, `objective`, `danger`, or a custom word.
    pub kind: String,
    pub label: String,
    pub visible: bool,
}

impl MarkedZone {
    pub fn contains(&self, p: GridPos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn center(&self) -> (f32, f32) {
        ((self.min.x + self.max.x) as f32 / 2.0, (self.min.y + self.max.y) as f32 / 2.0)
    }
}

/// State of a door panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorState {
    pub open: bool,
    pub locked: bool,
}

pub(super) fn team_index(team: Team) -> usize {
    match team {
        Team::Player => 0,
        Team::Enemy => 1,
    }
}

impl Battle {
    /// Applies the mission's mechanic settings and creates its doors. Called
    /// by `from_mission` before units are placed.
    pub(super) fn setup_mechanics(&mut self, mission: &MissionDef) {
        self.fog = mission.fog;
        self.zones = mission
            .zones
            .iter()
            .filter_map(|z| {
                let kind = z.marker.clone()?;
                let label = z.label.clone().unwrap_or_else(|| match kind.as_str() {
                    "evac" => "EVAC".into(),
                    "danger" => "DANGER".into(),
                    "objective" => "OBJECTIVE".into(),
                    other => other.to_uppercase(),
                });
                Some(MarkedZone { tag: z.tag.clone(), min: z.min, max: z.max, kind, label, visible: !z.hidden })
            })
            .collect();
        self.stealth = Stealth { concealed: mission.concealment, alarm: false, detection: mission.detection };
        for door in &mission.doors {
            let first = PropId(self.props.len() as u32);
            for &pos in &door.tiles {
                let id = PropId(self.props.len() as u32);
                self.map.set_terrain(pos, Terrain::HighCover);
                self.props.push(Prop {
                    id,
                    tag: door.tag.clone(),
                    kind: "door".into(),
                    pos,
                    cover: CoverLevel::Full,
                    hp: None,
                    model: None,
                    alive: true,
                    door: Some(DoorState { open: false, locked: door.locked }),
                });
            }
            if !door.tiles.is_empty() {
                self.add_hooks(super::world::HookOwner::Prop(first), &door.hooks);
            }
        }
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn stealth(&self) -> Stealth {
        self.stealth
    }

    /// Zones drawn on the board (evac pads, objectives...).
    pub fn zones(&self) -> &[MarkedZone] {
        &self.zones
    }

    /// Shows/hides or relabels the marked zone `tag` (scripts).
    pub(super) fn set_zone(&mut self, tag: &str, show: Option<bool>, label: Option<String>, events: &mut Vec<GameEvent>) {
        let Some(z) = self.zones.iter_mut().find(|z| z.tag == tag) else {
            events.push(GameEvent::ScriptError { message: format!("no marked zone '{tag}' (add marker=\"...\" to it)") });
            return;
        };
        if let Some(show) = show {
            z.visible = show;
        }
        if let Some(label) = label {
            z.label = label;
        }
        events.push(GameEvent::ZoneChanged { tag: tag.to_string() });
    }

    /// Does the mission hide enemies the squad can't see?
    pub fn fog(&self) -> bool {
        self.fog
    }

    /// Current objective text (empty = none).
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// Where `team`'s AI is told to go, if anywhere.
    pub fn ai_goal(&self, team: Team) -> Option<GridPos> {
        self.ai_goals[team_index(team)]
    }

    /// Should the player be shown this unit? (Always, without fog.)
    pub fn is_revealed(&self, id: UnitId) -> bool {
        let Some(u) = self.unit(id) else { return false };
        !self.fog || u.team == Team::Player || !u.is_alive() || self.team_can_see(Team::Player, u.pos)
    }

    /// The door panel on `pos`, if any.
    pub fn door_at(&self, pos: GridPos) -> Option<&Prop> {
        self.props.iter().find(|p| p.pos == pos && p.door.is_some())
    }

    // ------------------------------------------------------------------
    // Alarm, pods, detection
    // ------------------------------------------------------------------

    pub(super) fn set_objective(&mut self, text: String, events: &mut Vec<GameEvent>) {
        if self.objective != text {
            self.objective = text.clone();
            events.push(GameEvent::ObjectiveChanged { text });
        }
    }

    /// Breaks concealment, wakes every enemy and runs `alarm` hooks. Only the
    /// first call does anything.
    pub(super) fn raise_alarm(&mut self, by: Option<UnitId>, events: &mut Vec<GameEvent>) {
        if self.stealth.alarm {
            return;
        }
        self.stealth.alarm = true;
        self.stealth.concealed = false;
        events.push(GameEvent::Alarm { by });
        let sleepers: Vec<UnitId> = self.units.iter().filter(|u| u.is_alive() && u.ai.dormant).map(|u| u.id).collect();
        for id in sleepers {
            self.wake_one(id, events);
        }
        self.fire(Trigger { unit: by, ..Trigger::new(HookEvent::Alarm) }, events);
    }

    /// Wakes a dormant unit and the rest of its pod.
    pub(super) fn wake(&mut self, id: UnitId, events: &mut Vec<GameEvent>) {
        let Some(u) = self.unit(id).filter(|u| u.ai.dormant && u.is_alive()) else { return };
        match u.ai.pod.clone() {
            Some(pod) => {
                let pod: Vec<UnitId> =
                    self.units.iter().filter(|o| o.is_alive() && o.ai.dormant && o.ai.pod.as_ref() == Some(&pod)).map(|o| o.id).collect();
                for o in pod {
                    self.wake_one(o, events);
                }
            }
            None => self.wake_one(id, events),
        }
    }

    fn wake_one(&mut self, id: UnitId, events: &mut Vec<GameEvent>) {
        let u = &mut self.units[id.0 as usize];
        if u.ai.dormant {
            u.ai.dormant = false;
            events.push(GameEvent::UnitAlerted { unit: id });
        }
    }

    /// Runs after anything moves or appears:
    /// * concealed squad: an enemy within `detection` tiles with line of
    ///   sight to a soldier raises the alarm;
    /// * otherwise: dormant pods the squad can see (or that see the squad) wake.
    pub(super) fn check_awareness(&mut self, events: &mut Vec<GameEvent>) {
        if self.is_over() {
            return;
        }
        if self.stealth.concealed {
            let detection = self.stealth.detection;
            let spotter = self.living(Team::Enemy).find(|e| {
                self.living(Team::Player).any(|p| {
                    e.pos.distance(p.pos) <= detection.min(e.stats.sight) && visibility::has_line_of_sight(&self.map, e.pos, p.pos)
                })
            });
            if let Some(e) = spotter.map(|e| e.id) {
                self.raise_alarm(Some(e), events);
            }
            return;
        }
        let woken: Vec<UnitId> = self
            .living(Team::Enemy)
            .filter(|e| e.ai.dormant)
            .filter(|e| self.team_can_see(Team::Player, e.pos) || self.living(Team::Player).any(|p| self.can_unit_see(e, p.pos)))
            .map(|e| e.id)
            .collect();
        for id in woken {
            self.wake(id, events);
        }
    }

    /// Dormant units walk at most one AP per turn (patrol pace) and advance
    /// to their next waypoint on arrival.
    pub(super) fn after_dormant_move(&mut self, id: UnitId, events: &mut Vec<GameEvent>) {
        let u = &mut self.units[id.0 as usize];
        if !u.is_alive() || !u.ai.dormant {
            return;
        }
        if let Some(&wp) = u.ai.patrol.get(u.ai.patrol_index)
            && u.pos.chebyshev(wp) <= 1
        {
            u.ai.patrol_index = (u.ai.patrol_index + 1) % u.ai.patrol.len();
        }
        if u.action_points > 0 {
            u.action_points = 0;
            events.push(GameEvent::ActionPointsChanged { unit: id, remaining: 0 });
        }
    }

    // ------------------------------------------------------------------
    // Doors
    // ------------------------------------------------------------------

    fn door_ids(&self, tag: &str) -> Vec<PropId> {
        self.props.iter().filter(|p| p.door.is_some() && p.tag == tag).map(|p| p.id).collect()
    }

    /// Opens (or closes) every panel of door `tag`. Closing fails silently on
    /// panels with a unit in the way.
    pub(super) fn set_door_open(&mut self, tag: &str, open: bool, events: &mut Vec<GameEvent>) {
        let ids = self.door_ids(tag);
        if ids.is_empty() {
            events.push(GameEvent::ScriptError { message: format!("no door tagged '{tag}'") });
            return;
        }
        let mut changed = false;
        for id in ids.iter().copied() {
            let p = &self.props[id.0 as usize];
            let pos = p.pos;
            let Some(state) = p.door else { continue };
            if state.open == open || (!open && self.unit_at(pos).is_some()) {
                continue;
            }
            self.props[id.0 as usize].door = Some(DoorState { open, ..state });
            self.props[id.0 as usize].cover = if open { CoverLevel::None } else { CoverLevel::Full };
            self.map.set_terrain(pos, if open { Terrain::Floor } else { Terrain::HighCover });
            events.push(GameEvent::DoorChanged { prop: id, open });
            events.push(GameEvent::TerrainChanged { pos });
            changed = true;
        }
        if changed && open {
            let first = ids[0];
            let pos = self.props[first.0 as usize].pos;
            self.fire(Trigger { prop: Some(first), pos: Some(pos), ..Trigger::new(HookEvent::Opened) }, events);
        }
    }

    pub(super) fn lock_door(&mut self, tag: &str, locked: bool) {
        for id in self.door_ids(tag) {
            if let Some(d) = self.props[id.0 as usize].door.as_mut() {
                d.locked = locked;
            }
        }
    }

    /// A unit standing on `pos` opens adjacent closed, unlocked doors.
    pub(super) fn open_doors_near(&mut self, pos: GridPos, events: &mut Vec<GameEvent>) {
        let mut tags: Vec<String> = self
            .props
            .iter()
            .filter(|p| p.door.is_some_and(|d| !d.open && !d.locked) && p.pos.chebyshev(pos) == 1)
            .map(|p| p.tag.clone())
            .collect();
        tags.dedup();
        for tag in tags {
            self.set_door_open(&tag, true, events);
        }
    }
}

#[cfg(all(test, feature = "missions", feature = "scripting"))]
mod tests {
    use crate::action::{Action, Target};
    use crate::ai::{self, AiConfig};
    use crate::battle::Battle;
    use crate::content::Ruleset;
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Terrain};
    use crate::mission::parse_mission;
    use crate::unit::{Team, UnitId};

    fn battle(xml: &str) -> (Battle, Vec<GameEvent>) {
        let mission = parse_mission(xml).unwrap_or_else(|e| panic!("{e}"));
        let (mut b, warnings) = Battle::from_mission(Ruleset::standard(), &mission, 1);
        assert!(warnings.is_empty(), "{warnings:?}");
        let events = b.begin();
        (b, events)
    }

    fn has(events: &[GameEvent], f: impl Fn(&GameEvent) -> bool) -> bool {
        events.iter().any(f)
    }

    #[test]
    fn concealment_detection_raises_the_alarm_and_wakes_everyone() {
        let (mut b, events) = battle(
            r#"<mission id="m" concealment="true" detection="3">
                 <layout>
                   ....................
                   ....................
                 </layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="Guard" x="12" y="0" dormant="true"/>
                 <unit team="enemy" class="sectoid" name="Far" x="19" y="1" dormant="true" pod="b"/>
                 <script on="alarm">message("alarm!"); objective("Fight your way out");</script>
               </mission>"#,
        );
        assert!(!has(&events, |e| matches!(e, GameEvent::Alarm { .. })));
        assert!(b.stealth().concealed);
        let a = UnitId(0);
        // In plain view but beyond 3 tiles: still hidden; the guard stays dormant.
        b.apply(Action::Move { unit: a, to: GridPos::new(5, 1) }).unwrap();
        assert!(b.stealth().concealed && b.unit(UnitId(1)).unwrap().ai.dormant);
        let events = b.apply(Action::Move { unit: a, to: GridPos::new(9, 1) }).unwrap();
        assert!(has(&events, |e| matches!(e, GameEvent::Alarm { by: Some(UnitId(1)) })), "{events:?}");
        assert!(has(&events, |e| matches!(e, GameEvent::Message { text } if text == "alarm!")));
        assert_eq!(b.objective(), "Fight your way out");
        assert!(!b.stealth().concealed && b.stealth().alarm);
        assert!(b.units().iter().all(|u| !u.ai.dormant));
    }

    #[test]
    fn attacking_breaks_concealment() {
        let (mut b, _) = battle(
            r#"<mission id="m" concealment="true" detection="2">
                 <layout>..........</layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="E" x="6" y="0" dormant="true"/>
               </mission>"#,
        );
        let events = b.apply(Action::ability(UnitId(0), "shoot", Target::Unit(UnitId(1)))).unwrap();
        assert!(has(&events, |e| matches!(e, GameEvent::Alarm { by: None })));
        assert!(!b.unit(UnitId(1)).unwrap().ai.dormant);
    }

    #[test]
    fn pods_wake_together_when_seen_and_dormant_units_hold() {
        let (mut b, events) = battle(
            r#"<mission id="m">
                 <layout>
                   ..........H.....
                   ..........H.....
                 </layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="0"/>
                 <unit team="enemy" class="sectoid" name="P1" x="13" y="0" dormant="true" pod="a"/>
                 <unit team="enemy" class="sectoid" name="P2" x="15" y="0" dormant="true" pod="a"/>
                 <unit team="enemy" class="sectoid" name="Lone" x="15" y="1" dormant="true"/>
               </mission>"#,
        );
        assert!(!has(&events, |e| matches!(e, GameEvent::UnitAlerted { .. })), "behind the wall");
        // Dormant units just wait.
        b.apply(Action::EndTurn).unwrap();
        while b.active_team() == Team::Enemy {
            let action = ai::next_team_action(&b, &AiConfig::default());
            assert!(matches!(action, Action::Wait { .. } | Action::EndTurn), "{action:?}");
            b.apply(action).unwrap();
        }
        assert_eq!(b.unit(UnitId(1)).unwrap().pos, GridPos::new(13, 1));
        // Hurt one member of pod "a": the whole pod wakes, the loner doesn't.
        let mut events = Vec::new();
        b.apply_script_command(crate::script::ScriptCommand::Damage { unit: 1, amount: 1 }, &mut events);
        assert!(!b.unit(UnitId(1)).unwrap().ai.dormant && !b.unit(UnitId(2)).unwrap().ai.dormant);
        assert!(b.unit(UnitId(3)).unwrap().ai.dormant);
    }

    #[test]
    fn doors_open_when_approached_unless_locked() {
        let (mut b, _) = battle(
            r#"<mission id="m">
                 <layout>
                   ....H....
                   ....H....
                   .........
                 </layout>
                 <door tag="front" x="4" y="2" width="1" height="1"/>
                 <door tag="vault" x="8" y="0" width="1" height="1" locked="true">
                   <script on="opened">message("vault open");</script>
                 </door>
                 <unit team="player" class="rifleman" name="A" x="0" y="2"/>
                 <unit team="enemy" class="sectoid" name="E" x="6" y="0"/>
               </mission>"#,
        );
        let front = GridPos::new(4, 0);
        assert_eq!(b.map().terrain(front), Terrain::HighCover, "closed doors block");
        assert!(!b.team_can_see(Team::Player, GridPos::new(6, 2)), "and hide what's behind");
        let events = b.apply(Action::Move { unit: UnitId(0), to: GridPos::new(3, 0) }).unwrap();
        assert!(has(&events, |e| matches!(e, GameEvent::DoorChanged { open: true, .. })));
        assert_eq!(b.map().terrain(front), Terrain::Floor);
        // The locked vault only opens by script.
        let vault = GridPos::new(8, 2);
        assert!(!b.door_at(vault).unwrap().door.unwrap().open);
        let mut events = Vec::new();
        b.apply_script_command(crate::script::ScriptCommand::OpenDoor { tag: "vault".into() }, &mut events);
        assert!(has(&events, |e| matches!(e, GameEvent::Message { text } if text == "vault open")));
        assert_eq!(b.map().terrain(vault), Terrain::Floor);
    }

    #[test]
    fn patrols_walk_waypoints_and_ai_goals_pull_units() {
        let (mut b, _) = battle(
            r#"<mission id="m" win="script">
                 <layout>
                   ......................
                   ......................
                   HHHHHHHHHHHHHHHHHHHHHH
                   ......................
                 </layout>
                 <unit team="player" class="rifleman" name="A" x="0" y="3"/>
                 <unit team="enemy" class="sectoid" name="Patrol" x="2" y="0" patrol="10,0 2,0"/>
                 <unit team="enemy" class="sectoid" name="Hunter" x="20" y="1"/>
                 <script on="start">ai_goal("enemy", 12, 1);</script>
               </mission>"#,
        );
        assert_eq!(b.ai_goal(Team::Enemy), Some(GridPos::new(12, 2)));
        b.apply(Action::EndTurn).unwrap();
        while b.active_team() == Team::Enemy {
            b.apply(ai::next_team_action(&b, &AiConfig::default())).unwrap();
        }
        let patrol = b.unit(UnitId(1)).unwrap();
        assert!(patrol.ai.dormant);
        assert_eq!(patrol.pos, GridPos::new(8, 3), "one AP (6 tiles) towards the first waypoint");
        let hunter = b.unit(UnitId(2)).unwrap();
        assert!(hunter.pos.distance(GridPos::new(12, 2)) < 3.0, "the goal pulls the hunter: {}", hunter.pos);
    }
}

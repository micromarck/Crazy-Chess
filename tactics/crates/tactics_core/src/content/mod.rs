//! Game content and the module system.
//!
//! All tunable game content lives in a [`Ruleset`]: abilities ("moves"),
//! weapons, unit classes, squads, maps, AI profiles and global combat rules.
//! The battle engine only reads the ruleset, so changing content never means
//! changing rules code.
//!
//! Content is added by **modules**. A module is anything implementing
//! [`GameModule`]: a Rust type (see [`crate::modules`]) or, with the `data`
//! feature, a RON file loaded through [`data::DataModule`]. Modules are applied
//! in order and **later registrations replace earlier ones with the same id**,
//! so a module can both add new content and rebalance existing content.
//!
//! ```
//! use tactics_core::content::*;
//! use tactics_core::modules;
//!
//! struct MyModule;
//! impl GameModule for MyModule {
//!     fn info(&self) -> ModuleInfo {
//!         ModuleInfo::new("my_mod", "My Mod")
//!     }
//!     fn register(&self, rules: &mut Ruleset) {
//!         rules.add_ability(
//!             AbilityDef::new("snap_shot", "Snap Shot")
//!                 .with_cost(ApCost::Points(1))
//!                 .with_ammo_cost(1)
//!                 .with_targeting(Targeting::Enemy, AbilityRange::Weapon)
//!                 .with_effect(AbilityEffect::Attack(AttackProfile {
//!                     accuracy_bonus: -10,
//!                     half_cover_penalty: 25,
//!                     full_cover_penalty: 45,
//!                     ..Default::default()
//!                 })),
//!         );
//!         rules.grant_ability("rifleman", "snap_shot");
//!     }
//! }
//!
//! let mut rules = Ruleset::new();
//! rules.add_module(&modules::BaseModule);
//! rules.add_module(&MyModule);
//! assert!(rules.ability(&"snap_shot".into()).is_some());
//! assert!(rules.validate().is_empty());
//! ```
//!
//! ## Module types
//!
//! | kind | type | status |
//! |---|---|---|
//! | Abilities / moves | [`AbilityDef`] | ✅ |
//! | Weapons | [`Weapon`] | ✅ |
//! | Unit classes | [`UnitClass`] | ✅ |
//! | Squads (who deploys) | [`SquadDef`] | ✅ |
//! | Maps | [`MapDef`] | ✅ |
//! | AI profiles | [`AiProfile`] | ✅ |
//! | Global combat rules | [`CombatRules`] | ✅ |
//! | 3D models + animation clips | [`ModelDef`] | ✅ |
//! | Ability visuals (animation, camera, projectile) | [`AbilityPresentation`] | ✅ |
//! | Camera scripts | [`CameraScriptDef`] | ✅ |
//! | HTML/CSS UI layouts | [`UiLayoutDef`] | ✅ |
//! | Status effects, terrain types, items, objectives, campaign… | — | planned, see `MODULES.md` |

pub mod ability;
#[cfg(feature = "data")]
pub mod data;
pub mod presentation;

use std::collections::BTreeMap;

pub use ability::*;
pub use presentation::*;

use crate::ai::AiConfig;
pub use crate::unit::{UnitClass, UnitStats, Weapon};

/// Identity and metadata of a module.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct ModuleInfo {
    /// Unique, stable id (`"base"`, `"grenadier"`...).
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
}

impl ModuleInfo {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: "1.0".into(),
            ..Default::default()
        }
    }
}

/// A bundle of content. Implement this to extend the game.
pub trait GameModule {
    /// Who this module is (shown in logs and the editor).
    fn info(&self) -> ModuleInfo;
    /// Adds or replaces content in the ruleset.
    fn register(&self, rules: &mut Ruleset);
}

/// Global numbers that apply to every attack and turn.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct CombatRules {
    /// Lowest possible hit chance for attacks that can miss.
    pub min_hit: i32,
    /// Highest possible hit chance for attacks that can miss.
    pub max_hit: i32,
    /// Accuracy lost per tile beyond a weapon's optimal range.
    pub falloff_per_tile: f32,
    /// Accuracy gained per tile closer than optimal range...
    pub close_bonus_per_tile: f32,
    /// ...capped at this value.
    pub max_close_bonus: i32,
    /// Action points every unit receives each turn.
    pub action_points: u8,
}

impl Default for CombatRules {
    fn default() -> Self {
        Self {
            min_hit: 1,
            max_hit: 99,
            falloff_per_tile: 3.0,
            close_bonus_per_tile: 2.0,
            max_close_bonus: 15,
            action_points: 2,
        }
    }
}

/// A hand-made map in the ASCII format of [`crate::grid::Map::from_ascii`].
/// `A` marks player spawns, `E` enemy spawns.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct MapDef {
    pub id: String,
    pub name: String,
    pub ascii: String,
}

/// One soldier/alien in a squad.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SquadMember {
    /// Unit class id.
    pub class: String,
    /// Display name of this individual.
    pub name: String,
}

/// An ordered list of units deployed together. The scenarios use the squads
/// with ids `"player"` and `"enemy"`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SquadDef {
    pub id: String,
    pub members: Vec<SquadMember>,
}

impl SquadDef {
    /// Builds a squad from `(class, name)` pairs.
    pub fn new(id: impl Into<String>, members: &[(&str, &str)]) -> Self {
        Self {
            id: id.into(),
            members: members
                .iter()
                .map(|(class, name)| SquadMember { class: class.to_string(), name: name.to_string() })
                .collect(),
        }
    }
}

/// Named AI tuning (e.g. "aggressive", "cautious", per difficulty or species).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct AiProfile {
    pub id: String,
    pub config: AiConfig,
}

/// The complete set of content a battle is played with.
///
/// Collections are `BTreeMap`s keyed by id so iteration order (and therefore
/// the battle) is deterministic.
#[derive(Debug, Clone, Default)]
pub struct Ruleset {
    pub combat: CombatRules,
    abilities: BTreeMap<AbilityId, AbilityDef>,
    weapons: BTreeMap<String, Weapon>,
    classes: BTreeMap<String, UnitClass>,
    squads: BTreeMap<String, SquadDef>,
    maps: BTreeMap<String, MapDef>,
    ai_profiles: BTreeMap<String, AiProfile>,
    // Presentation (read only by frontends).
    pub presentation: PresentationSettings,
    models: BTreeMap<String, ModelDef>,
    ability_presentations: BTreeMap<AbilityId, AbilityPresentation>,
    camera_scripts: BTreeMap<String, CameraScriptDef>,
    ui_layouts: BTreeMap<String, UiLayoutDef>,
    modules: Vec<ModuleInfo>,
    // FUTURE module types slot in here as additional maps, e.g.
    //   status_effects: BTreeMap<String, StatusEffectDef>,
    //   terrain: BTreeMap<String, TerrainDef>,
    //   items: BTreeMap<String, ItemDef>,
    //   objectives: BTreeMap<String, ObjectiveDef>,
    // See MODULES.md for the planned shapes.
}

impl Ruleset {
    /// An empty ruleset with default combat rules. Add modules to fill it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in content only.
    pub fn standard() -> Self {
        let mut rules = Self::new();
        rules.add_module(&crate::modules::BaseModule);
        rules
    }

    /// Applies a module. Later modules override earlier content with equal ids.
    pub fn add_module(&mut self, module: &dyn GameModule) {
        module.register(self);
        self.modules.push(module.info());
    }

    /// Modules applied so far, in load order.
    pub fn modules(&self) -> &[ModuleInfo] {
        &self.modules
    }

    // --- registration -----------------------------------------------------

    pub fn add_ability(&mut self, def: AbilityDef) {
        self.abilities.insert(def.id.clone(), def);
    }

    pub fn add_weapon(&mut self, weapon: Weapon) {
        self.weapons.insert(weapon.id.clone(), weapon);
    }

    pub fn add_class(&mut self, class: UnitClass) {
        self.classes.insert(class.id.clone(), class);
    }

    pub fn add_squad(&mut self, squad: SquadDef) {
        self.squads.insert(squad.id.clone(), squad);
    }

    pub fn add_map(&mut self, map: MapDef) {
        self.maps.insert(map.id.clone(), map);
    }

    pub fn add_ai_profile(&mut self, profile: AiProfile) {
        self.ai_profiles.insert(profile.id.clone(), profile);
    }

    pub fn add_model(&mut self, model: ModelDef) {
        self.models.insert(model.id.clone(), model);
    }

    pub fn add_presentation(&mut self, p: AbilityPresentation) {
        self.ability_presentations.insert(p.ability.clone(), p);
    }

    pub fn add_camera_script(&mut self, script: CameraScriptDef) {
        self.camera_scripts.insert(script.id.clone(), script);
    }

    pub fn add_ui_layout(&mut self, layout: UiLayoutDef) {
        self.ui_layouts.insert(layout.id.clone(), layout);
    }

    /// Gives an existing class an extra ability (no-op if already known or the
    /// class doesn't exist). Lets a module add a move to classes it didn't define.
    pub fn grant_ability(&mut self, class: &str, ability: &str) {
        if let Some(c) = self.classes.get_mut(class) {
            let id = AbilityId::new(ability);
            if !c.abilities.contains(&id) {
                c.abilities.push(id);
            }
        }
    }

    // --- lookup -------------------------------------------------------------

    pub fn ability(&self, id: &AbilityId) -> Option<&AbilityDef> {
        self.abilities.get(id)
    }

    /// Mutable access, used by the in-game editor for live tuning.
    pub fn ability_mut(&mut self, id: &AbilityId) -> Option<&mut AbilityDef> {
        self.abilities.get_mut(id)
    }

    pub fn abilities(&self) -> impl Iterator<Item = &AbilityDef> {
        self.abilities.values()
    }

    pub fn weapon(&self, id: &str) -> Option<&Weapon> {
        self.weapons.get(id)
    }

    pub fn weapons(&self) -> impl Iterator<Item = &Weapon> {
        self.weapons.values()
    }

    pub fn class(&self, id: &str) -> Option<&UnitClass> {
        self.classes.get(id)
    }

    pub fn classes(&self) -> impl Iterator<Item = &UnitClass> {
        self.classes.values()
    }

    pub fn squad(&self, id: &str) -> Option<&SquadDef> {
        self.squads.get(id)
    }

    pub fn squads(&self) -> impl Iterator<Item = &SquadDef> {
        self.squads.values()
    }

    pub fn map(&self, id: &str) -> Option<&MapDef> {
        self.maps.get(id)
    }

    pub fn maps(&self) -> impl Iterator<Item = &MapDef> {
        self.maps.values()
    }

    pub fn ai_profile(&self, id: &str) -> Option<&AiProfile> {
        self.ai_profiles.get(id)
    }

    pub fn ai_profiles(&self) -> impl Iterator<Item = &AiProfile> {
        self.ai_profiles.values()
    }

    pub fn model(&self, id: &str) -> Option<&ModelDef> {
        self.models.get(id)
    }

    pub fn models(&self) -> impl Iterator<Item = &ModelDef> {
        self.models.values()
    }

    /// Visuals of an ability (`None` = frontend defaults).
    pub fn presentation_of(&self, ability: &AbilityId) -> Option<&AbilityPresentation> {
        self.ability_presentations.get(ability)
    }

    pub fn presentations(&self) -> impl Iterator<Item = &AbilityPresentation> {
        self.ability_presentations.values()
    }

    pub fn camera_script(&self, id: &str) -> Option<&CameraScriptDef> {
        self.camera_scripts.get(id)
    }

    pub fn camera_scripts(&self) -> impl Iterator<Item = &CameraScriptDef> {
        self.camera_scripts.values()
    }

    /// An HTML UI layout by id (`"hud"`...).
    pub fn ui_layout(&self, id: &str) -> Option<&UiLayoutDef> {
        self.ui_layouts.get(id)
    }

    /// Checks cross-references (classes → weapons/abilities, squads → classes).
    /// Returns human-readable problems; empty means consistent.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for class in self.classes.values() {
            if !self.weapons.contains_key(&class.weapon) {
                problems.push(format!("class '{}' uses unknown weapon '{}'", class.id, class.weapon));
            }
            for a in &class.abilities {
                if !self.abilities.contains_key(a) {
                    problems.push(format!("class '{}' lists unknown ability '{a}'", class.id));
                }
            }
        }
        for class in self.classes.values() {
            if !class.model.is_empty() && !self.models.contains_key(&class.model) {
                problems.push(format!("class '{}' uses unknown model '{}'", class.id, class.model));
            }
        }
        for p in self.ability_presentations.values() {
            if !self.abilities.contains_key(&p.ability) {
                problems.push(format!("presentation for unknown ability '{}'", p.ability));
            }
            if let Some(cam) = &p.camera
                && !self.camera_scripts.contains_key(cam) {
                    problems.push(format!("ability '{}' uses unknown camera script '{cam}'", p.ability));
                }
        }
        if let Some(cam) = &self.presentation.kill_camera
            && !self.camera_scripts.contains_key(cam) {
                problems.push(format!("kill camera uses unknown camera script '{cam}'"));
            }
        for squad in self.squads.values() {
            for m in &squad.members {
                if !self.classes.contains_key(&m.class) {
                    problems.push(format!("squad '{}' uses unknown class '{}'", squad.id, m.class));
                }
            }
        }
        for ability in self.abilities.values() {
            if let Some(p) = ability.effect.attack_profile()
                && p.half_cover_penalty > p.full_cover_penalty {
                    problems.push(format!(
                        "ability '{}': half cover penalty ({}) exceeds full cover penalty ({})",
                        ability.id, p.half_cover_penalty, p.full_cover_penalty
                    ));
                }
        }
        problems
    }
}

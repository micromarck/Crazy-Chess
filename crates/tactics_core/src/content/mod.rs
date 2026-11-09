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
//! | Skeletons (bone rigs, can extend each other) | [`SkeletonDef`] | ✅ |
//! | Keyframe animations per skeleton + slot | [`AnimationDef`] | ✅ |
//! | Items (armor, utility) | [`ItemDef`] | ✅ |
//! | Loadout presets | [`LoadoutDef`] | ✅ |
//! | Morale / fear rules | [`MoraleRules`] (in [`CombatRules`]) | ✅ |
//! | Status effects, terrain types, objectives, campaign… | — | planned, see `MODULES.md` |

pub mod ability;
pub mod appearance;
#[cfg(feature = "data")]
pub mod data;
pub mod gear;
pub mod morale;
pub mod presentation;
pub mod skeleton;

use std::collections::BTreeMap;

pub use ability::*;
pub use appearance::{Appearance, Build, Hair, Headgear};
pub use gear::*;
pub use morale::*;
pub use presentation::*;
pub use skeleton::*;

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
    /// Aim bonus when the attack comes from higher ground than the target.
    pub height_advantage_bonus: i32,
    /// Aim penalty while suppressed.
    pub suppression_aim: i32,
    /// Fear added to a unit that gets suppressed.
    pub suppression_fear: i32,
    /// Scales every attack's aim (before cover): 1.3 = 30% more accurate.
    pub accuracy_multiplier: f32,
    /// Fear and its effects.
    pub morale: MoraleRules,
    /// Damage to a unit that starts its turn standing in fire.
    pub fire_damage: i32,
    /// Damage for each burning tile a unit walks into.
    pub fire_step_damage: i32,
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
            height_advantage_bonus: 15,
            suppression_aim: 25,
            suppression_fear: 15,
            accuracy_multiplier: 1.3,
            morale: MoraleRules::default(),
            fire_damage: 2,
            fire_step_damage: 1,
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
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SquadMember {
    /// Unit class id.
    pub class: String,
    /// Display name of this individual.
    pub name: String,
    /// Loadout preset id (see [`LoadoutDef`]).
    pub loadout: Option<String>,
    /// Individual overrides on top of the preset.
    pub gear: Loadout,
    /// Looks (`None` = a stable random look from the name).
    pub appearance: Option<Appearance>,
}

/// An ordered list of units deployed together. The scenarios use the squads
/// with ids `"player"` and `"enemy"`.
#[derive(Debug, Clone, PartialEq, Default)]
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
                .map(|(class, name)| SquadMember { class: class.to_string(), name: name.to_string(), ..Default::default() })
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
    items: BTreeMap<String, ItemDef>,
    loadouts: BTreeMap<String, LoadoutDef>,
    // Presentation (read only by frontends).
    pub presentation: PresentationSettings,
    models: BTreeMap<String, ModelDef>,
    ability_presentations: BTreeMap<AbilityId, AbilityPresentation>,
    camera_scripts: BTreeMap<String, CameraScriptDef>,
    ui_layouts: BTreeMap<String, UiLayoutDef>,
    skeletons: BTreeMap<String, SkeletonDef>,
    /// Keyed by (skeleton, slot).
    animations: BTreeMap<(String, String), AnimationDef>,
    /// Shared script functions from modules (`script_library`), available
    /// to every script (class scripts and missions alike).
    pub script_library: String,
    modules: Vec<ModuleInfo>,
    // FUTURE module types slot in here as additional maps, e.g.
    //   status_effects: BTreeMap<String, StatusEffectDef>,
    //   terrain: BTreeMap<String, TerrainDef>,
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

    pub fn add_item(&mut self, item: ItemDef) {
        self.items.insert(item.id.clone(), item);
    }

    pub fn add_loadout(&mut self, loadout: LoadoutDef) {
        self.loadouts.insert(loadout.id.clone(), loadout);
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

    /// Adds or replaces a skeleton. To change only some bones of an existing
    /// skeleton, define a new one that `extends` it.
    pub fn add_skeleton(&mut self, skeleton: SkeletonDef) {
        self.skeletons.insert(skeleton.id.clone(), skeleton);
    }

    /// Adds or replaces the animation for `(anim.skeleton, anim.slot)`.
    pub fn add_animation(&mut self, anim: AnimationDef) {
        self.animations.insert((anim.skeleton.clone(), anim.slot.clone()), anim);
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

    pub fn skeleton(&self, id: &str) -> Option<&SkeletonDef> {
        self.skeletons.get(id)
    }

    pub fn skeletons(&self) -> impl Iterator<Item = &SkeletonDef> {
        self.skeletons.values()
    }

    /// Animations defined directly for `skeleton` (not inherited ones; see
    /// [`Ruleset::skeleton_animations`]).
    pub fn animations_of<'a>(&'a self, skeleton: &'a str) -> impl Iterator<Item = &'a AnimationDef> + 'a {
        self.animations.iter().filter(move |((sk, _), _)| sk == skeleton).map(|(_, a)| a)
    }

    /// An HTML UI layout by id (`"hud"`...).
    pub fn item(&self, id: &str) -> Option<&ItemDef> {
        self.items.get(id)
    }

    pub fn items(&self) -> impl Iterator<Item = &ItemDef> {
        self.items.values()
    }

    pub fn loadout(&self, id: &str) -> Option<&LoadoutDef> {
        self.loadouts.get(id)
    }

    pub fn loadouts(&self) -> impl Iterator<Item = &LoadoutDef> {
        self.loadouts.values()
    }

    /// Mutable squad access (frontends edit the player's loadouts).
    pub fn squad_mut(&mut self, id: &str) -> Option<&mut SquadDef> {
        self.squads.get_mut(id)
    }

    /// The gear a unit of `class` ends up with: the class's preset, then
    /// `preset`, then `overrides` (see [`gear`]). Unknown ids are kept here
    /// and reported by [`Ruleset::check_gear`] / at spawn.
    pub fn gear_for(&self, class: &str, preset: Option<&str>, overrides: &Loadout) -> Loadout {
        let class = self.classes.get(class);
        let mut gear = Loadout { weapon: class.map(|c| c.weapon.clone()), ..Default::default() };
        if let Some(p) = class.and_then(|c| c.loadout.as_deref()).and_then(|id| self.loadouts.get(id)) {
            gear = gear.overridden_by(&p.gear);
        }
        if let Some(p) = preset.and_then(|id| self.loadouts.get(id)) {
            gear = gear.overridden_by(&p.gear);
        }
        let mut gear = gear.overridden_by(overrides);
        // "" means "explicitly nothing": armor "" = no armor, items [""] = no items.
        if gear.armor.as_deref() == Some("") {
            gear.armor = None;
        }
        gear.items.retain(|i| !i.is_empty());
        gear.augments.retain(|i| !i.is_empty());
        gear
    }

    /// Is this gear legal for `class`? Returns the problems.
    pub fn check_gear(&self, class: &str, gear: &Loadout) -> Vec<String> {
        let mut problems = Vec::new();
        // Restricted classes (robot frames) only take gear made for them.
        let restricted = self.class(class).is_some_and(|c| c.restricted_gear);
        // Augments can make a soldier count as another class for gear (a heat
        // frame conversion carries Strider weapons).
        let gear_as: Vec<&String> = gear.augments.iter().filter_map(|a| self.items.get(a)).flat_map(|i| i.gear_as.iter()).collect();
        let allowed = |list: &[String]| (!restricted && list.is_empty()) || list.iter().any(|c| c == class || gear_as.contains(&c));
        if let Some(w) = &gear.weapon {
            match self.weapons.get(w) {
                None => problems.push(format!("unknown weapon '{w}'")),
                Some(w) if !allowed(&w.classes) => problems.push(format!("{} can't use the {}", class, w.name)),
                _ => {}
            }
        }
        let mut check_item = |id: &str, slot: ItemSlot| match self.items.get(id) {
            None => problems.push(format!("unknown item '{id}'")),
            Some(i) if i.slot != slot => problems.push(format!("'{id}' doesn't go in the {slot:?} slot")),
            Some(i) if !allowed(&i.classes) => problems.push(format!("{class} can't use {}", i.name)),
            _ => {}
        };
        if let Some(a) = &gear.armor {
            check_item(a, ItemSlot::Armor);
        }
        for i in &gear.items {
            check_item(i, ItemSlot::Utility);
        }
        for a in &gear.augments {
            check_item(a, ItemSlot::Augment);
        }
        if gear.items.len() > UTILITY_SLOTS {
            problems.push(format!("{} utility items (max {UTILITY_SLOTS})", gear.items.len()));
        }
        if gear.augments.len() > AUGMENT_SLOTS {
            problems.push(format!("{} augments (max {AUGMENT_SLOTS})", gear.augments.len()));
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(dup) = gear.augments.iter().find(|a| !seen.insert(*a)) {
            problems.push(format!("the same augment twice ('{dup}')"));
        }
        problems
    }

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
            for h in &class.hooks {
                if crate::mission::HookEvent::parse(&h.on).is_none() {
                    problems.push(format!("class '{}' has a script on unknown event '{}'", class.id, h.on));
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
        for model in self.models.values() {
            if let Some(sk) = &model.skeleton
                && let Err(e) = self.resolve_skeleton(sk) {
                    problems.push(format!("model '{}': {e}", model.id));
                }
        }
        for sk in self.skeletons.keys() {
            match self.resolve_skeleton(sk) {
                Err(e) => problems.push(e),
                Ok(resolved) => {
                    for anim in self.animations_of(sk) {
                        let bones = anim.rotations.keys().chain(anim.translations.keys()).chain(anim.mask.iter().flatten());
                        for b in bones {
                            if resolved.index_of(b).is_none() {
                                problems.push(format!("animation '{sk}/{}' uses unknown bone '{b}'", anim.slot));
                            }
                        }
                    }
                }
            }
        }
        for (sk, slot) in self.animations.keys() {
            if !self.skeletons.contains_key(sk) {
                problems.push(format!("animation '{sk}/{slot}' is for unknown skeleton '{sk}'"));
            }
        }
        for squad in self.squads.values() {
            for m in &squad.members {
                if !self.classes.contains_key(&m.class) {
                    problems.push(format!("squad '{}' uses unknown class '{}'", squad.id, m.class));
                }
                if let Some(p) = &m.loadout
                    && !self.loadouts.contains_key(p)
                {
                    problems.push(format!("squad '{}': {} uses unknown loadout '{p}'", squad.id, m.name));
                }
                let gear = self.gear_for(&m.class, m.loadout.as_deref(), &m.gear);
                for p in self.check_gear(&m.class, &gear) {
                    problems.push(format!("squad '{}': {}: {p}", squad.id, m.name));
                }
            }
        }
        for item in self.items.values() {
            for a in &item.abilities {
                if !self.abilities.contains_key(a) {
                    problems.push(format!("item '{}' grants unknown ability '{a}'", item.id));
                }
            }
        }
        for l in self.loadouts.values() {
            let class = l.classes.first().map_or("", String::as_str);
            for p in self.check_gear(class, &l.gear) {
                if !p.contains("can't use") {
                    problems.push(format!("loadout '{}': {p}", l.id));
                }
            }
        }
        for class in self.classes.values() {
            if let Some(p) = &class.loadout
                && !self.loadouts.contains_key(p)
            {
                problems.push(format!("class '{}' uses unknown loadout '{p}'", class.id));
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

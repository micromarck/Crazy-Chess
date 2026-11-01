//! Data-driven modules: content written in [RON](https://github.com/ron-rs/ron)
//! instead of Rust (enabled by the `data` feature).
//!
//! A data module is a single RON document with any of these sections; every
//! section and every field is optional and falls back to its default:
//!
//! ```ron
//! (
//!     info: (id: "marksman", name: "Marksman Pack", version: "1.0"),
//!     abilities: [
//!         (
//!             id: "aimed_shot",
//!             name: "Aimed Shot",
//!             cost: Points(2),
//!             ammo_cost: 1,
//!             targeting: Enemy,
//!             effect: Attack((accuracy_bonus: 20, half_cover_penalty: 15, full_cover_penalty: 30)),
//!         ),
//!     ],
//!     grants: [(class: "sniper", ability: "aimed_shot")],
//! )
//! ```
//!
//! Load with [`DataModule::from_ron`] and apply with
//! [`Ruleset::add_module`](super::Ruleset::add_module). The in-game editor uses
//! [`DataModule::to_ron`] to save tuned values back to disk.

use super::{
    AbilityDef, AiProfile, CombatRules, GameModule, MapDef, ModuleInfo, Ruleset, SquadDef, UnitClass,
    Weapon,
};

/// "Give this existing class this ability" — lets data modules extend classes
/// they don't own without redefining them.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AbilityGrant {
    pub class: String,
    pub ability: String,
}

/// A module described entirely by data.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DataModule {
    pub info: ModuleInfo,
    /// Replaces the global combat rules when present.
    pub combat: Option<CombatRules>,
    pub abilities: Vec<AbilityDef>,
    pub weapons: Vec<Weapon>,
    pub classes: Vec<UnitClass>,
    pub squads: Vec<SquadDef>,
    pub maps: Vec<MapDef>,
    pub ai_profiles: Vec<AiProfile>,
    pub grants: Vec<AbilityGrant>,
}

/// RON extensions we enable for every file, so authors can skip the
/// `#![enable(...)]` header:
/// * `implicit_some` — write `charges: 2` instead of `charges: Some(2)`;
/// * `unwrap_variant_newtypes` — write `Attack(accuracy_bonus: 10)` instead of
///   `Attack((accuracy_bonus: 10))` (the double form still works).
fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(
        ron::extensions::Extensions::IMPLICIT_SOME | ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES,
    )
}

impl DataModule {
    /// Parses a module from RON text. The error message includes line/column.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron_options().from_str(text).map_err(|e| e.to_string())
    }

    /// Serialises the module as pretty RON (round-trips through `from_ron`).
    pub fn to_ron(&self) -> String {
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(6)
            .struct_names(false)
            .extensions(
                ron::extensions::Extensions::IMPLICIT_SOME
                    | ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES,
            );
        ron_options()
            .to_string_pretty(self, pretty)
            .unwrap_or_else(|e| format!("// serialisation failed: {e}"))
    }

    /// Snapshot of every ability in a ruleset as a module — what the in-game
    /// editor saves. Loading it later re-applies the tuned values (same ids
    /// replace the originals).
    pub fn abilities_from(rules: &Ruleset, info: ModuleInfo) -> Self {
        Self {
            info,
            combat: Some(rules.combat.clone()),
            abilities: rules.abilities().cloned().collect(),
            ..Default::default()
        }
    }
}

impl GameModule for DataModule {
    fn info(&self) -> ModuleInfo {
        self.info.clone()
    }

    fn register(&self, rules: &mut Ruleset) {
        if let Some(combat) = &self.combat {
            rules.combat = combat.clone();
        }
        for a in &self.abilities {
            rules.add_ability(a.clone());
        }
        for w in &self.weapons {
            rules.add_weapon(w.clone());
        }
        for c in &self.classes {
            rules.add_class(c.clone());
        }
        for s in &self.squads {
            rules.add_squad(s.clone());
        }
        for m in &self.maps {
            rules.add_map(m.clone());
        }
        for p in &self.ai_profiles {
            rules.add_ai_profile(p.clone());
        }
        for g in &self.grants {
            rules.grant_ability(&g.class, &g.ability);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{AbilityEffect, ApCost, Targeting};

    const SAMPLE: &str = r#"
        (
            info: (id: "test", name: "Test"),
            abilities: [
                (
                    id: "aimed_shot",
                    name: "Aimed Shot",
                    cost: Points(2),
                    ammo_cost: 1,
                    targeting: Enemy,
                    charges: 2,
                    effect: Attack(accuracy_bonus: 20, half_cover_penalty: 15),
                ),
            ],
            grants: [(class: "rifleman", ability: "aimed_shot")],
        )
    "#;

    #[test]
    fn parses_with_defaults_and_shorthand() {
        let module = DataModule::from_ron(SAMPLE).expect("valid RON");
        let a = &module.abilities[0];
        assert_eq!(a.cost, ApCost::Points(2));
        assert_eq!(a.targeting, Targeting::Enemy);
        assert_eq!(a.charges, Some(2));
        let AbilityEffect::Attack(p) = &a.effect else { panic!("expected attack") };
        assert_eq!(p.accuracy_bonus, 20);
        assert_eq!(p.half_cover_penalty, 15);
        assert_eq!(p.full_cover_penalty, 40, "unspecified fields keep defaults");
    }

    #[test]
    fn applies_and_grants() {
        let mut rules = Ruleset::standard();
        rules.add_module(&DataModule::from_ron(SAMPLE).unwrap());
        assert!(rules.class("rifleman").unwrap().abilities.contains(&"aimed_shot".into()));
        assert!(rules.validate().is_empty(), "{:?}", rules.validate());
    }

    #[test]
    fn round_trip() {
        let rules = Ruleset::standard();
        let module = DataModule::abilities_from(&rules, ModuleInfo::new("x", "X"));
        let text = module.to_ron();
        let back = DataModule::from_ron(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(module, back);
    }
}

//! The base game content, expressed as a module.
//!
//! Everything here could equally be a RON file; it's Rust only so the game has
//! content even when the `data` feature is disabled.

use crate::ai::AiConfig;
use crate::content::*;
use crate::scenario::DEMO_MAP;

/// The core content every game starts from.
pub struct BaseModule;

/// Ids of the base abilities, for code that wants to refer to them.
pub mod ids {
    pub const SHOOT: &str = "shoot";
    pub const OVERWATCH: &str = "overwatch";
    pub const HUNKER: &str = "hunker";
    pub const RELOAD: &str = "reload";
}

impl GameModule for BaseModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            description: "Core abilities, weapons, classes, squads and the demo map.".into(),
            author: "Tactics".into(),
            ..ModuleInfo::new("base", "Base Game")
        }
    }

    fn register(&self, rules: &mut Ruleset) {
        rules.combat = CombatRules::default();
        register_abilities(rules);
        register_weapons(rules);
        register_classes(rules);
        register_squads_maps_ai(rules);
    }
}

fn register_abilities(rules: &mut Ruleset) {
    // Standard shot: unit aim, -20 half cover, -40 full cover, range falloff.
    rules.add_ability(
        AbilityDef::new(ids::SHOOT, "Shoot")
            .with_description("Fire your weapon at a visible enemy. Ends the turn.")
            .with_cost(ApCost::EndsTurn)
            .with_ammo_cost(1)
            .with_targeting(Targeting::Enemy, AbilityRange::Weapon)
            .with_effect(AbilityEffect::Attack(AttackProfile::default())),
    );

    // Reaction shot: same cover rules, but -15 accuracy.
    rules.add_ability(
        AbilityDef::new(ids::OVERWATCH, "Overwatch")
            .with_description("Shoot the first enemy that moves in sight (-15 aim).")
            .with_cost(ApCost::EndsTurn)
            .with_targeting(Targeting::SelfOnly, AbilityRange::Weapon)
            .with_effect(AbilityEffect::Overwatch(AttackProfile {
                accuracy_bonus: -15,
                ..Default::default()
            })),
    );

    rules.add_ability(
        AbilityDef::new(ids::HUNKER, "Hunker Down")
            .with_description("+30 defence until your next turn.")
            .with_cost(ApCost::EndsTurn)
            .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
            .with_los(false)
            .with_effect(AbilityEffect::Defend { defense_bonus: 30 }),
    );

    rules.add_ability(
        AbilityDef::new(ids::RELOAD, "Reload")
            .with_description("Refill the magazine. Costs 1 AP.")
            .with_cost(ApCost::Points(1))
            .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
            .with_los(false)
            .with_effect(AbilityEffect::Reload),
    );
}

fn register_weapons(rules: &mut Ruleset) {
    let w = |id: &str, name: &str| Weapon { id: id.into(), name: name.into(), ..Default::default() };
    rules.add_weapon(Weapon { damage_min: 3, damage_max: 5, crit_chance: 10, clip_size: 4, ..w("rifle", "Assault Rifle") });
    rules.add_weapon(Weapon {
        damage_min: 4,
        damage_max: 7,
        crit_chance: 20,
        crit_bonus: 3,
        optimal_range: 2.0,
        max_range: 10.0,
        clip_size: 3,
        ..w("shotgun", "Shotgun")
    });
    rules.add_weapon(Weapon {
        damage_min: 4,
        damage_max: 6,
        crit_chance: 25,
        crit_bonus: 3,
        optimal_range: 14.0,
        max_range: 25.0,
        clip_size: 3,
        ..w("sniper_rifle", "Sniper Rifle")
    });
    rules.add_weapon(Weapon {
        damage_min: 2,
        damage_max: 4,
        crit_chance: 5,
        optimal_range: 5.0,
        max_range: 14.0,
        clip_size: 3,
        ..w("plasma_pistol", "Plasma Pistol")
    });
}

fn register_classes(rules: &mut Ruleset) {
    let basic = || {
        [ids::SHOOT, ids::OVERWATCH, ids::HUNKER, ids::RELOAD]
            .map(AbilityId::new)
            .to_vec()
    };
    rules.add_class(UnitClass {
        id: "rifleman".into(),
        name: "Rifleman".into(),
        stats: UnitStats::default(),
        weapon: "rifle".into(),
        abilities: basic(),
        model: "soldier".into(),
    });
    rules.add_class(UnitClass {
        id: "assault".into(),
        name: "Assault".into(),
        stats: UnitStats { max_hp: 10, mobility: 14, aim: 65, ..Default::default() },
        weapon: "shotgun".into(),
        abilities: basic(),
        model: "soldier".into(),
    });
    rules.add_class(UnitClass {
        id: "sniper".into(),
        name: "Sniper".into(),
        stats: UnitStats { max_hp: 6, mobility: 10, aim: 80, sight: 18.0, ..Default::default() },
        weapon: "sniper_rifle".into(),
        abilities: basic(),
        model: "soldier".into(),
    });
    rules.add_class(UnitClass {
        id: "sectoid".into(),
        name: "Sectoid".into(),
        stats: UnitStats { max_hp: 6, mobility: 12, aim: 65, defense: 5, sight: 13.0 },
        weapon: "plasma_pistol".into(),
        abilities: basic(),
        model: "alien".into(),
    });
}

fn register_squads_maps_ai(rules: &mut Ruleset) {
    rules.add_squad(SquadDef::new(
        "player",
        &[
            ("rifleman", "Sgt. Vance"),
            ("assault", "Cpl. Okoye"),
            ("sniper", "Spc. Moreau"),
            ("rifleman", "Pvt. Kowalski"),
        ],
    ));
    rules.add_squad(SquadDef::new(
        "enemy",
        &[
            ("sectoid", "Sectoid 1"),
            ("sectoid", "Sectoid 2"),
            ("sectoid", "Sectoid 3"),
            ("sectoid", "Sectoid 4"),
            ("sectoid", "Sectoid 5"),
        ],
    ));
    rules.add_map(MapDef { id: "demo".into(), name: "Supply Depot".into(), ascii: DEMO_MAP.into() });
    rules.add_ai_profile(AiProfile { id: "default".into(), config: AiConfig::default() });
    rules.add_ai_profile(AiProfile {
        id: "aggressive".into(),
        config: AiConfig { shoot_threshold: 35, cover_weight: 0.5, distance_weight: 2.5, ..AiConfig::default() },
    });
}

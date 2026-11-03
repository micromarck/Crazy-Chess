//! The base game content, expressed as a module.
//!
//! Everything here could equally be a RON file; it's Rust only so the game has
//! content even when the `data` feature is disabled.

use crate::ai::AiConfig;
use crate::content::*;
use crate::unit::WeaponLook;
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
        register_presentation(rules);
    }
}

/// Models, camera scripts and ability visuals. Pure data for the frontend.
fn register_presentation(rules: &mut Ruleset) {
    use CameraAnchor::*;
    use CameraStep::*;

    // The standard skeleton and its basic movements (idle, walk, shoot...).
    super::humanoid::register(rules);

    // Models: animated rigs on the standard skeleton (no assets needed).
    // Modules can override these ids with glTF scenes or custom skeletons
    // (see assets/modules/creatures.ron).
    rules.add_model(ModelDef::rigged("soldier", super::humanoid::HUMANOID, PrimitiveShape::Soldier));
    rules.add_model(ModelDef::rigged("alien", super::humanoid::HUMANOID, PrimitiveShape::Alien));

    // XCOM-style over-the-shoulder shot.
    rules.add_camera_script(CameraScriptDef {
        id: "over_shoulder".into(),
        steps: vec![
            Together(vec![
                Focus { on: Midpoint, duration: 0.45 },
                FaceAlong { from: Actor, to: Target, offset_degrees: 25.0, duration: 0.45 },
                Zoom { distance: 9.0, duration: 0.45 },
                Pitch { degrees: 25.0, duration: 0.45 },
            ]),
            Wait { duration: 1.1 },
            Restore { duration: 0.5 },
        ],
    });

    // Slow-motion orbit around the victim.
    rules.add_camera_script(CameraScriptDef {
        id: "kill_cam".into(),
        steps: vec![
            TimeScale { scale: 0.35 },
            Together(vec![
                Focus { on: Target, duration: 0.35 },
                Zoom { distance: 6.5, duration: 0.35 },
                Pitch { degrees: 30.0, duration: 0.35 },
            ]),
            Orbit { degrees: 40.0, duration: 1.0 },
            Restore { duration: 0.5 },
        ],
    });

    // Pull back and shake for explosions.
    rules.add_camera_script(CameraScriptDef {
        id: "explosion_cam".into(),
        steps: vec![
            Together(vec![
                Focus { on: Target, duration: 0.4 },
                Zoom { distance: 13.0, duration: 0.4 },
                Pitch { degrees: 60.0, duration: 0.4 },
            ]),
            Wait { duration: 0.35 },
            Shake { strength: 0.35, duration: 0.5 },
            Restore { duration: 0.6 },
        ],
    });

    // A short look at the unit (stances, buffs).
    rules.add_camera_script(CameraScriptDef {
        id: "focus_actor".into(),
        steps: vec![
            Together(vec![Focus { on: Actor, duration: 0.35 }, Zoom { distance: 10.0, duration: 0.35 }]),
            Wait { duration: 0.4 },
            Restore { duration: 0.4 },
        ],
    });

    let present = |ability: &str, animation: &str| AbilityPresentation {
        ability: AbilityId::new(ability),
        animation: Some(animation.into()),
        ..Default::default()
    };
    rules.add_presentation(AbilityPresentation {
        camera: Some("over_shoulder".into()),
        camera_chance: 50,
        ..present(ids::SHOOT, "shoot")
    });
    rules.add_presentation(AbilityPresentation {
        projectile: ProjectileStyle::None,
        ..present(ids::OVERWATCH, "overwatch")
    });
    rules.add_presentation(AbilityPresentation {
        projectile: ProjectileStyle::None,
        ..present(ids::HUNKER, "hunker")
    });
    rules.add_presentation(AbilityPresentation {
        projectile: ProjectileStyle::None,
        windup: 0.6,
        ..present(ids::RELOAD, "reload")
    });

    rules.presentation = PresentationSettings { kill_camera: Some("kill_cam".into()), kill_camera_chance: 100 };
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

/// Weapons. Damage is **per round**: actions fire bursts (`burst` rounds,
/// shotguns `pellets` per round), each traced as a bullet (see
/// `battle::ballistics`). `spread` (degrees) is the gun's own scatter.
fn register_weapons(rules: &mut Ruleset) {
    let w = |id: &str, name: &str| Weapon { id: id.into(), name: name.into(), ..Default::default() };
    rules.add_weapon(Weapon {
        damage_min: 1,
        damage_max: 2,
        crit_chance: 10,
        crit_bonus: 1,
        burst: 4,
        clip_size: 24,
        magazines: 3,
        spread: 0.8,
        ..w("rifle", "Assault Rifle")
    });
    rules.add_weapon(Weapon {
        damage_min: 1,
        damage_max: 1,
        crit_chance: 15,
        crit_bonus: 1,
        look: WeaponLook::Shotgun,
        // A tight choke: deadly up close, still dangerous at mid range.
        optimal_range: 4.0,
        max_range: 10.0,
        burst: 1,
        pellets: 8,
        spread: 1.8,
        // The blast wraps around cover: pellets can't be blocked by it.
        ignores_cover: true,
        clip_size: 6,
        magazines: 3,
        ..w("shotgun", "Shotgun")
    });
    rules.add_weapon(Weapon {
        damage_min: 4,
        damage_max: 6,
        crit_chance: 25,
        crit_bonus: 3,
        optimal_range: 14.0,
        max_range: 25.0,
        burst: 1,
        spread: 0.1,
        // The scope is useless up close: -10 aim per tile inside 5 tiles
        // (an adjacent target: -40).
        min_range: 5.0,
        close_penalty: 10,
        clip_size: 5,
        magazines: 3,
        classes: vec!["sniper".into()],
        look: WeaponLook::Sniper,
        ..w("sniper_rifle", "Sniper Rifle")
    });
    rules.add_weapon(Weapon {
        damage_min: 1,
        damage_max: 2,
        crit_chance: 5,
        optimal_range: 5.0,
        max_range: 14.0,
        burst: 2,
        spread: 1.0,
        clip_size: 12,
        magazines: 3,
        look: WeaponLook::Plasma,
        ..w("plasma_pistol", "Plasma Pistol")
    });
    rules.add_weapon(Weapon {
        damage_min: 1,
        damage_max: 1,
        crit_chance: 15,
        crit_bonus: 1,
        optimal_range: 4.0,
        max_range: 14.0,
        burst: 5,
        spread: 1.5,
        clip_size: 30,
        magazines: 3,
        look: WeaponLook::Smg,
        ..w("smg", "SMG")
    });
}

/// Armor and stat items. Grenades and medkits come with the Grenadier Pack.
fn register_items(rules: &mut Ruleset) {
    let armor = |id: &str, name: &str, desc: &str, stats: StatModifiers| ItemDef {
        id: id.into(),
        name: name.into(),
        description: desc.into(),
        slot: ItemSlot::Armor,
        stats,
        ..Default::default()
    };
    rules.add_item(armor("kevlar", "Kevlar Vest", "+2 HP.", StatModifiers { max_hp: 2, ..Default::default() }));
    rules.add_item(armor(
        "plated_armor",
        "Plated Armor",
        "+4 HP, +5 defense, -2 mobility, +10 bravery.",
        StatModifiers { max_hp: 4, defense: 5, mobility: -2, bravery: 10, ..Default::default() },
    ));
    rules.add_item(armor(
        "ghost_suit",
        "Ghost Suit",
        "+1 HP, +2 mobility, +10 defense.",
        StatModifiers { max_hp: 1, mobility: 2, defense: 10, ..Default::default() },
    ));
    let util = |id: &str, name: &str, desc: &str, stats: StatModifiers| ItemDef {
        id: id.into(),
        name: name.into(),
        description: desc.into(),
        slot: ItemSlot::Utility,
        stats,
        ..Default::default()
    };
    rules.add_item(util("scope", "Scope", "+10 aim.", StatModifiers { aim: 10, ..Default::default() }));
    rules.add_item(util("stims", "Combat Stims", "+2 mobility.", StatModifiers { mobility: 2, ..Default::default() }));
    rules.add_item(util("ammo_pouch", "Ammo Pouch", "+2 spare magazines.", StatModifiers { magazines: 2, ..Default::default() }));
    rules.add_item(util("dog_tags", "Lucky Dog Tags", "+25 bravery: shrugs off fear.", StatModifiers { bravery: 25, ..Default::default() }));
}

fn register_classes(rules: &mut Ruleset) {
    register_items(rules);
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
        ..Default::default()
    });
    rules.add_class(UnitClass {
        id: "assault".into(),
        name: "Assault".into(),
        stats: UnitStats { max_hp: 10, mobility: 14, aim: 65, bravery: 60, ..Default::default() },
        weapon: "shotgun".into(),
        abilities: basic(),
        model: "soldier".into(),
        ..Default::default()
    });
    rules.add_class(UnitClass {
        id: "sniper".into(),
        name: "Sniper".into(),
        stats: UnitStats { max_hp: 6, mobility: 10, aim: 80, sight: 18.0, bravery: 55, ..Default::default() },
        weapon: "sniper_rifle".into(),
        abilities: basic(),
        model: "soldier".into(),
        ..Default::default()
    });
    rules.add_class(UnitClass {
        id: "sectoid".into(),
        name: "Sectoid".into(),
        stats: UnitStats { max_hp: 6, mobility: 12, aim: 65, defense: 5, sight: 13.0, bravery: 40, ..Default::default() },
        weapon: "plasma_pistol".into(),
        abilities: basic(),
        model: "alien".into(),
        ..Default::default()
    });
    // A stationary, harmless target for training missions: no abilities,
    // can't move, never afraid.
    rules.add_class(UnitClass {
        id: "training_dummy".into(),
        name: "Training Dummy".into(),
        stats: UnitStats { max_hp: 4, mobility: 0, aim: 0, defense: 0, sight: 1.0, ..Default::default() },
        weapon: "plasma_pistol".into(),
        abilities: Vec::new(),
        model: "alien".into(),
        fearless: true,
        ..Default::default()
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

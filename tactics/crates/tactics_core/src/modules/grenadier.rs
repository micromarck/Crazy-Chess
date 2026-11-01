//! Example add-on module written in Rust.
//!
//! Shows the three things a module typically does:
//! 1. add **new abilities** (an area grenade, a medkit),
//! 2. add a **new class** and weapon that use them,
//! 3. **modify existing content** (grant the medkit to riflemen, swap a squad
//!    member for the new class).
//!
//! Use it as a template: copy the file, rename the struct, change the numbers,
//! and add your module to the list in `tactics_game/src/content.rs`.

use crate::content::*;

pub struct GrenadierModule;

impl GameModule for GrenadierModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            description: "Adds the Grenadier class, frag grenades and medkits.".into(),
            author: "Tactics".into(),
            ..ModuleInfo::new("grenadier", "Grenadier Pack")
        }
    }

    fn register(&self, rules: &mut Ruleset) {
        // 1. New abilities ---------------------------------------------------
        rules.add_ability(
            AbilityDef::new("frag_grenade", "Frag Grenade")
                .with_description("Throw at a tile: 3-4 damage to everyone within 1.5 tiles. Never misses, ignores cover. 1 per battle.")
                .with_cost(ApCost::Points(1))
                .with_charges(1)
                .with_targeting(Targeting::Tile, AbilityRange::Fixed(10.0))
                .with_effect(AbilityEffect::Attack(AttackProfile {
                    can_miss: false,
                    half_cover_penalty: 0,
                    full_cover_penalty: 0,
                    range_falloff: false,
                    flank_crit_bonus: 0,
                    damage: DamageSource::Fixed { min: 3, max: 4 },
                    area_radius: 1.5,
                    ..Default::default()
                })),
        );

        rules.add_ability(
            AbilityDef::new("medkit", "Medkit")
                .with_description("Heal an adjacent ally (or yourself) for 4 HP. 2 per battle.")
                .with_cost(ApCost::Points(1))
                .with_charges(2)
                .with_targeting(Targeting::Ally, AbilityRange::Fixed(1.5))
                .with_los(false)
                .with_effect(AbilityEffect::Heal { amount: 4 }),
        );

        // 2. New weapon + class ----------------------------------------------
        rules.add_weapon(Weapon {
            id: "lmg".into(),
            name: "Light Machine Gun".into(),
            damage_min: 3,
            damage_max: 6,
            crit_chance: 5,
            crit_bonus: 2,
            optimal_range: 7.0,
            max_range: 16.0,
            clip_size: 5,
            ammo: 0,
        });
        rules.add_class(UnitClass {
            id: "grenadier".into(),
            name: "Grenadier".into(),
            stats: UnitStats { max_hp: 9, mobility: 11, aim: 65, ..Default::default() },
            weapon: "lmg".into(),
            abilities: ["shoot", "frag_grenade", "overwatch", "hunker", "reload"]
                .map(AbilityId::new)
                .to_vec(),
            model: "soldier".into(),
        });

        // Visuals: throw animation, lobbed projectile, explosion camera.
        rules.add_presentation(AbilityPresentation {
            ability: "frag_grenade".into(),
            animation: Some("throw".into()),
            camera: Some("explosion_cam".into()),
            projectile: ProjectileStyle::Arc,
            windup: 0.45,
            ..Default::default()
        });
        rules.add_presentation(AbilityPresentation {
            ability: "medkit".into(),
            animation: Some("heal".into()),
            camera: Some("focus_actor".into()),
            projectile: ProjectileStyle::None,
            ..Default::default()
        });

        // 3. Modify existing content -----------------------------------------
        rules.grant_ability("rifleman", "medkit");
        rules.add_squad(SquadDef::new(
            "player",
            &[
                ("rifleman", "Sgt. Vance"),
                ("assault", "Cpl. Okoye"),
                ("sniper", "Spc. Moreau"),
                ("grenadier", "Pvt. Kowalski"),
            ],
        ));
    }
}

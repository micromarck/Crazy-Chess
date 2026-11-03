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
use crate::unit::WeaponLook;

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
                .with_description("Throw at a tile: 3-4 damage to everyone within 1.5 tiles and to walls and props there. Never misses, ignores cover. 16 fragments fly further out (1 damage each, friend or foe; walls stop them). 1 per battle.")
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
                    shrapnel: 16,
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
            damage_min: 1,
            damage_max: 2,
            crit_chance: 5,
            crit_bonus: 1,
            optimal_range: 7.0,
            max_range: 16.0,
            burst: 6,
            spread: 1.6,
            clip_size: 50,
            magazines: 2,
            look: WeaponLook::Lmg,
            ..Default::default()
        });
        // Suppressive fire: a long, loose burst that pins the target.
        rules.add_ability(
            AbilityDef::new("suppress", "Suppress")
                .with_description("Hose the target with a 12-round burst (-20 aim; bullets hit anything in the way). It is pinned: -25 aim, frightened, and if it moves you fire again. Ends the turn.")
                .with_cost(ApCost::EndsTurn)
                .with_ammo_cost(1)
                .with_targeting(Targeting::Enemy, AbilityRange::Weapon)
                .with_effect(AbilityEffect::Suppress(AttackProfile { accuracy_bonus: -20, rounds: Some(12), ..Default::default() })),
        );
        rules.add_class(UnitClass {
            id: "grenadier".into(),
            name: "Grenadier".into(),
            stats: UnitStats { max_hp: 9, mobility: 11, aim: 65, ..Default::default() },
            weapon: "lmg".into(),
            abilities: ["shoot", "suppress", "frag_grenade", "overwatch", "hunker", "reload"]
                .map(AbilityId::new)
                .to_vec(),
            model: "soldier".into(),
            ..Default::default()
        });

        // Items that carry the new abilities, and loadout presets using them.
        rules.add_item(ItemDef {
            id: "frag".into(),
            name: "Frag Grenade".into(),
            description: "Grants Frag Grenade (one throw per grenade carried).".into(),
            slot: ItemSlot::Utility,
            abilities: vec!["frag_grenade".into()],
            charges: Some(1),
            ..Default::default()
        });
        rules.add_item(ItemDef {
            id: "medkit_item".into(),
            name: "Medkit".into(),
            description: "Grants Medkit (two uses per kit).".into(),
            slot: ItemSlot::Utility,
            abilities: vec!["medkit".into()],
            charges: Some(2),
            ..Default::default()
        });
        let preset = |id: &str, name: &str, desc: &str, weapon: &str, armor: Option<&str>, items: &[&str]| LoadoutDef {
            id: id.into(),
            name: name.into(),
            description: desc.into(),
            classes: Vec::new(),
            gear: Loadout {
                weapon: Some(weapon.into()),
                armor: armor.map(String::from),
                items: items.iter().map(|s| s.to_string()).collect(),
            },
        };
        rules.add_loadout(preset("standard", "Standard", "Rifle, kevlar, one frag.", "rifle", Some("kevlar"), &["frag"]));
        rules.add_loadout(preset("breacher", "Breacher", "Shotgun, plated armor, frag and stims: room clearing.", "shotgun", Some("plated_armor"), &["frag", "stims"]));
        rules.add_loadout(preset("recon", "Recon", "SMG, ghost suit, scope: fast and quiet.", "smg", Some("ghost_suit"), &["scope"]));
        rules.add_loadout(preset("medic", "Field Medic", "Rifle, kevlar, two medkits.", "rifle", Some("kevlar"), &["medkit_item", "medkit_item"]));
        rules.add_loadout(preset("heavy", "Heavy", "LMG, plated armor, two frags.", "lmg", Some("plated_armor"), &["frag", "frag"]));
        rules.add_loadout(LoadoutDef {
            classes: vec!["sniper".into()],
            ..preset("marksman_kit", "Marksman", "Sniper rifle, scope, dog tags.", "sniper_rifle", Some("ghost_suit"), &["scope", "dog_tags"])
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
            ability: "suppress".into(),
            animation: Some("overwatch".into()),
            projectile: ProjectileStyle::Tracer,
            windup: 0.2,
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
        let mut squad = SquadDef::new(
            "player",
            &[
                ("rifleman", "Sgt. Vance"),
                ("assault", "Cpl. Okoye"),
                ("sniper", "Spc. Moreau"),
                ("grenadier", "Pvt. Kowalski"),
            ],
        );
        // Default loadouts (players change them in the Loadout panel).
        for (member, preset) in squad.members.iter_mut().zip(["standard", "breacher", "marksman_kit", "heavy"]) {
            member.loadout = Some(preset.into());
        }
        rules.add_squad(squad);
    }
}

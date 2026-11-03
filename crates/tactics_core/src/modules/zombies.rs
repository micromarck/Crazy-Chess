//! Zombies: slow, tough, mindless and many.
//!
//! * `zombie` class — half a soldier's speed, 2 HP (one good hit), a claw strike in reach,
//!   no fear. It uses the **horde** brain (closes in and strikes; cheap
//!   enough for hundreds of units) and **hears** gunfire within 14 tiles
//!   while dormant, so every shot draws the crowd in.
//! * `bite` — melee: rolled, not traced, ignores cover, 2-3 damage.
//! * `zombie_horde` squad — a pack of them, for missions.
//!
//! The look (a hunched skeleton with its own shamble) is data:
//! `tactics_game/assets/modules/zombies.ron` adds the `zombie` model.

use crate::content::*;
use crate::unit::{UnitClass, UnitStats, Weapon, WeaponLook};

pub struct ZombieModule;

impl GameModule for ZombieModule {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            description: "Adds slow, fragile, mindless zombies that swarm in hordes and come running at gunfire.".into(),
            author: "Tactics".into(),
            ..ModuleInfo::new("zombies", "Zombie Pack")
        }
    }

    fn register(&self, rules: &mut Ruleset) {
        rules.add_weapon(Weapon {
            id: "claws".into(),
            name: "Claws".into(),
            damage_min: 2,
            damage_max: 3,
            crit_chance: 10,
            crit_bonus: 1,
            optimal_range: 1.0,
            max_range: 1.5,
            clip_size: 0,
            magazines: 0,
            look: WeaponLook::None,
            ..Default::default()
        });
        rules.add_ability(
            AbilityDef::new("bite", "Bite")
                .with_description("Claw and bite an enemy in reach: 2-3 damage. Ignores cover.")
                .with_cost(ApCost::EndsTurn)
                .with_targeting(Targeting::Enemy, AbilityRange::Fixed(1.5))
                .with_effect(AbilityEffect::Attack(AttackProfile {
                    base_accuracy: Some(75),
                    half_cover_penalty: 0,
                    full_cover_penalty: 0,
                    range_falloff: false,
                    flank_crit_bonus: 0,
                    melee: true,
                    ..Default::default()
                })),
        );
        rules.add_class(UnitClass {
            id: "zombie".into(),
            name: "Zombie".into(),
            // mobility 6 = 3 tiles per action point (soldiers: 6).
            stats: UnitStats { max_hp: 2, mobility: 6, aim: 60, defense: 0, sight: 10.0, climb: 1, bravery: 100 },
            weapon: "claws".into(),
            abilities: vec![AbilityId::new("bite")],
            model: "zombie".into(),
            fearless: true,
            horde: true,
            hearing: 14.0,
            ..Default::default()
        });
        rules.add_squad(SquadDef {
            id: "zombie_horde".into(),
            members: (1..=8).map(|i| SquadMember { class: "zombie".into(), name: format!("Zombie {i}"), ..Default::default() }).collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{Action, Target};
    use crate::ai::{self, AiConfig};
    use crate::battle::Battle;
    use crate::content::Ruleset;
    use crate::event::GameEvent;
    use crate::grid::{GridPos, Map};
    use crate::unit::Team;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&super::ZombieModule);
        r
    }

    #[test]
    fn zombies_shamble_in_and_bite() {
        let mut b = Battle::new(Map::from_ascii(&"..............\n".repeat(6)), rules(), 3);
        let s = b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 2)).unwrap();
        let z = b.spawn("zombie", "Z", Team::Enemy, GridPos::new(10, 2)).unwrap();
        b.apply(Action::EndTurn).unwrap();
        // Slow: two moves of 3 tiles still leave it short of the soldier.
        let mut bites = 0;
        for _ in 0..40 {
            if b.active_team() != Team::Enemy {
                b.apply(Action::EndTurn).unwrap();
                continue;
            }
            let a = ai::next_team_action(&b, &AiConfig::default());
            if matches!(&a, Action::UseAbility { ability, .. } if ability.as_str() == "bite") {
                bites += 1;
            }
            b.apply(a).unwrap();
            if b.is_over() {
                break;
            }
        }
        assert!(b.unit(z).unwrap().pos.chebyshev(b.unit(s).map_or(GridPos::new(0, 2), |u| u.pos)) <= 1);
        assert!(bites > 0, "it reached the soldier and bit");
    }

    #[test]
    fn bites_are_melee_not_bullets() {
        let mut b = Battle::new(Map::from_ascii("....."), rules(), 1);
        let s = b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 0)).unwrap();
        let z = b.spawn("zombie", "Z", Team::Enemy, GridPos::new(1, 0)).unwrap();
        b.apply(Action::EndTurn).unwrap();
        let ev = b.apply(Action::ability(z, "bite", Target::Unit(s))).unwrap();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::AttackResolved { .. })));
        assert!(!ev.iter().any(|e| matches!(e, GameEvent::Volley { .. })));
    }

    #[test]
    fn gunfire_wakes_zombies_that_hear_it() {
        let mut b = Battle::new(Map::from_ascii(&"..............................\n".repeat(3)), rules(), 1);
        let s = b.spawn("rifleman", "S", Team::Player, GridPos::new(0, 1)).unwrap();
        let target = b.spawn("zombie", "T", Team::Enemy, GridPos::new(5, 1)).unwrap();
        let near = b.spawn("zombie", "Near", Team::Enemy, GridPos::new(12, 2)).unwrap();
        let far = b.spawn("zombie", "Far", Team::Enemy, GridPos::new(29, 0)).unwrap();
        for id in [near, far] {
            b.set_dormant(id, true);
        }
        b.apply(Action::ability(s, "shoot", Target::Unit(target))).unwrap();
        assert!(!b.unit(near).unwrap().ai.dormant, "heard the shot");
        assert!(b.unit(far).unwrap().ai.dormant, "too far away to hear");
    }
}

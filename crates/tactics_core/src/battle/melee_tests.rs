//! Tests for shock weapons (tasers vs machines) and slams (big robots).

use crate::action::{Action, Target};
use crate::battle::Battle;
use crate::content::{AbilityDef, AbilityEffect, AbilityId, AbilityRange, ApCost, AttackProfile, DamageSource, Ruleset, Targeting};
use crate::grid::{GridPos, Map};
use crate::unit::{Machine, Team};

fn rules() -> Ruleset {
    let mut r = Ruleset::standard();
    r.add_ability(
        AbilityDef::new("tase", "Tase")
            .with_cost(ApCost::Points(1))
            .with_targeting(Targeting::Enemy, AbilityRange::Fixed(3.0))
            .with_effect(AbilityEffect::Attack(AttackProfile {
                can_miss: false,
                range_falloff: false,
                damage: DamageSource::Fixed { min: 1, max: 1 },
                melee: true,
                shock: Some(9),
                stun: 1,
                ..Default::default()
            })),
    );
    r.add_ability(
        AbilityDef::new("slam", "Slam")
            .with_cost(ApCost::EndsTurn)
            .with_targeting(Targeting::Enemy, AbilityRange::Fixed(1.5))
            .with_effect(AbilityEffect::Attack(AttackProfile {
                can_miss: false,
                half_cover_penalty: 0,
                full_cover_penalty: 0,
                range_falloff: false,
                damage: DamageSource::Fixed { min: 5, max: 5 },
                area_radius: 1.5,
                melee: true,
                slam: true,
                ..Default::default()
            })),
    );
    let mut trooper = r.class("rifleman").unwrap().clone();
    trooper.abilities.push(AbilityId::new("tase"));
    r.add_class(trooper);
    for (id, machine, hp) in [("bot_light", Machine::Light, 8), ("bot_medium", Machine::Medium, 14), ("bot_heavy", Machine::Heavy, 20)] {
        let mut c = r.class("rifleman").unwrap().clone();
        c.id = id.into();
        c.machine = machine;
        c.stats.max_hp = hp;
        c.abilities.push(AbilityId::new("slam"));
        r.add_class(c);
    }
    r
}

fn tase(b: &mut Battle, who: crate::unit::UnitId, at: crate::unit::UnitId) {
    b.apply(Action::ability(who, "tase", Target::Unit(at))).unwrap();
}

#[test]
fn a_taser_shorts_out_small_robots_and_hurts_big_ones() {
    let mut b = Battle::new(Map::from_ascii(&"........\n".repeat(5)), rules(), 1);
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(2, 2)).unwrap();
    let c = b.spawn("rifleman", "C", Team::Player, GridPos::new(2, 0)).unwrap();
    let light = b.spawn("bot_light", "Light", Team::Enemy, GridPos::new(4, 2)).unwrap();
    let medium = b.spawn("bot_medium", "Medium", Team::Enemy, GridPos::new(4, 3)).unwrap();
    let heavy = b.spawn("bot_heavy", "Heavy", Team::Enemy, GridPos::new(4, 1)).unwrap();
    let preview = b.preview(a, &AbilityId::new("tase"), Target::Unit(medium)).unwrap();
    assert_eq!(preview[0].1.damage, (14, 14), "the preview shows it shorting out");
    tase(&mut b, a, light);
    tase(&mut b, a, medium);
    assert!(!b.unit(light).unwrap().is_alive(), "light robot shorted out");
    assert!(!b.unit(medium).unwrap().is_alive(), "medium robot shorted out");
    tase(&mut b, c, heavy);
    let h = b.unit(heavy).unwrap();
    assert!(h.is_alive());
    assert_eq!(h.hp, 20 - 9, "a heavy robot takes the shock damage");
    // Machines aren't stunned: the heavy one acts on its turn.
    b.apply(Action::EndTurn).unwrap();
    assert!(b.unit(heavy).unwrap().action_points > 0);
}

#[test]
fn a_taser_stuns_people() {
    let mut b = Battle::new(Map::from_ascii(&"........\n".repeat(5)), rules(), 1);
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(2, 2)).unwrap();
    let e = b.spawn("rifleman", "E", Team::Enemy, GridPos::new(4, 2)).unwrap();
    let hp = b.unit(e).unwrap().hp;
    tase(&mut b, a, e);
    assert_eq!(b.unit(e).unwrap().hp, hp - 1);
    b.apply(Action::EndTurn).unwrap();
    assert_eq!(b.unit(e).unwrap().action_points, 0, "stunned: loses its turn");
    b.apply(Action::EndTurn).unwrap();
    b.apply(Action::EndTurn).unwrap();
    assert!(b.unit(e).unwrap().action_points > 0, "and is back the turn after");
}

#[test]
fn a_slam_hits_everyone_around_the_user_but_the_user() {
    let mut b = Battle::new(Map::from_ascii(&"........\n".repeat(5)), rules(), 1);
    let bot = b.spawn("bot_heavy", "Bastion", Team::Player, GridPos::new(3, 2)).unwrap();
    let ally = b.spawn("rifleman", "Too close", Team::Player, GridPos::new(2, 2)).unwrap();
    let foe = b.spawn("rifleman", "Foe", Team::Enemy, GridPos::new(4, 3)).unwrap();
    let far = b.spawn("rifleman", "Far", Team::Enemy, GridPos::new(6, 2)).unwrap();
    let hp = |b: &Battle, id| b.unit(id).unwrap().hp;
    let (bot0, ally0, foe0, far0) = (hp(&b, bot), hp(&b, ally), hp(&b, foe), hp(&b, far));
    b.apply(Action::ability(bot, "slam", Target::Unit(foe))).unwrap();
    assert_eq!(hp(&b, bot), bot0, "the slammer is spared");
    assert_eq!(hp(&b, foe), foe0 - 5);
    assert_eq!(hp(&b, ally), ally0 - 5, "friend or foe");
    assert_eq!(hp(&b, far), far0, "only those next to it");
}

#[test]
fn an_adrenaline_shot_trades_half_your_health_for_two_action_points() {
    let mut r = rules();
    r.add_ability(
        AbilityDef::new("adrenaline", "Adrenaline")
            .with_cost(ApCost::Points(0))
            .with_targeting(Targeting::SelfOnly, AbilityRange::Fixed(0.0))
            .with_effect(AbilityEffect::Adrenaline { action_points: 2 }),
    );
    let mut c = r.class("rifleman").unwrap().clone();
    c.abilities.push(AbilityId::new("adrenaline"));
    r.add_class(c);
    let mut b = Battle::new(Map::from_ascii(&"........\n".repeat(3)), r, 1);
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(1, 1)).unwrap();
    b.spawn("rifleman", "E", Team::Enemy, GridPos::new(7, 1)).unwrap();
    let hp = b.unit(a).unwrap().hp;
    let ap = b.unit(a).unwrap().action_points;
    b.apply(Action::ability(a, "adrenaline", Target::None)).unwrap();
    let u = b.unit(a).unwrap();
    assert_eq!(u.hp, hp - hp / 2, "half the remaining health, rounded down");
    assert_eq!(u.action_points, ap + 2);
    // At 1 HP it costs nothing.
    b.units[a.0 as usize].hp = 1;
    b.apply(Action::ability(a, "adrenaline", Target::None)).unwrap();
    let u = b.unit(a).unwrap();
    assert_eq!((u.hp, u.action_points), (1, ap + 4));
}

#[test]
fn blind_fire_shoots_at_a_tile_beyond_sight_without_giving_anything_away() {
    use crate::event::GameEvent;
    let mut b = Battle::new(Map::from_ascii(&"....................\n".repeat(3)), Ruleset::standard(), 1);
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 1)).unwrap();
    let e = b.spawn("rifleman", "E", Team::Enemy, GridPos::new(16, 1)).unwrap();
    assert!(!b.team_sees_unit(Team::Player, b.unit(e).unwrap()), "16 tiles: past a rifleman's sight (14)");
    // A normal shot can't reach what you can't see; blind fire can (rifle: 18).
    assert!(b.validate(&Action::ability(a, "shoot", Target::Unit(e))).is_err());
    let blind = Action::ability(a, "blind_fire", Target::Tile(GridPos::new(16, 1)));
    assert!(b.validate(&blind).is_ok());
    // The preview doesn't say whether anyone's there.
    assert!(b.preview(a, &AbilityId::new("blind_fire"), Target::Tile(GridPos::new(16, 1))).unwrap().is_empty());
    let ammo = b.unit(a).unwrap().weapon.ammo;
    let ev = b.apply(blind).unwrap();
    assert!(ev.iter().any(|x| matches!(x, GameEvent::Volley { target, .. } if *target == e)), "the burst goes at whoever is there");
    assert!(b.unit(a).unwrap().weapon.ammo < ammo);
}

#[test]
fn blind_fire_into_nothing_spends_the_burst() {
    let mut b = Battle::new(Map::from_ascii(&"....................\n".repeat(3)), Ruleset::standard(), 1);
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 1)).unwrap();
    b.spawn("rifleman", "E", Team::Enemy, GridPos::new(19, 2)).unwrap();
    let ammo = b.unit(a).unwrap().weapon.ammo;
    b.apply(Action::ability(a, "blind_fire", Target::Tile(GridPos::new(12, 0)))).unwrap();
    assert_eq!(b.unit(a).unwrap().weapon.ammo, ammo - 4, "a rifle burst");
    assert_eq!(b.unit(a).unwrap().action_points, 0, "ends the turn");
}

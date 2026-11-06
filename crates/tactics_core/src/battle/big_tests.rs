//! Tests for big (multi-tile) units: a 3x3 "brute" (the zombie's bite).

use crate::action::{Action, Target};
use crate::battle::Battle;
use crate::content::Ruleset;
use crate::grid::{GridPos, Map};
use crate::modules::{GrenadierModule, ZombieModule};
use crate::unit::Team;

fn rules() -> Ruleset {
    let mut r = Ruleset::standard();
    r.add_module(&GrenadierModule);
    r.add_module(&ZombieModule);
    let mut brute = r.class("zombie").unwrap().clone();
    brute.id = "brute".into();
    brute.size = 3;
    brute.stats.max_hp = 30;
    brute.stats.mobility = 12;
    r.add_class(brute);
    r
}

#[test]
fn a_big_unit_takes_its_whole_footprint() {
    let mut b = Battle::new(Map::from_ascii(&"..........\n".repeat(8)), rules(), 1);
    let brute = b.spawn("brute", "Brute", Team::Enemy, GridPos::new(4, 3)).unwrap();
    for (x, y) in [(4, 3), (6, 3), (4, 5), (6, 5), (5, 4)] {
        assert_eq!(b.unit_at(GridPos::new(x, y)).map(|u| u.id), Some(brute), "({x},{y})");
    }
    assert!(b.unit_at(GridPos::new(7, 3)).is_none());
    assert!(b.spawn("rifleman", "In the way", Team::Player, GridPos::new(6, 5)).is_err(), "can't stand inside it");
    assert!(b.spawn("brute", "Overlap", Team::Enemy, GridPos::new(6, 5)).is_err(), "footprints can't overlap");
    // Walls: no room for 3x3 next to the edge of a 2-wide space.
    let mut b = Battle::new(Map::from_ascii("..H.....\n..H.....\n..H....."), rules(), 1);
    assert!(b.spawn("brute", "Squeezed", Team::Enemy, GridPos::new(0, 0)).is_err());
    assert!(b.spawn("brute", "Fits", Team::Enemy, GridPos::new(3, 0)).is_ok());
}

#[test]
fn a_big_unit_cannot_squeeze_through_a_gap() {
    let art = "
        .....H.....
        .....H.....
        ...........
        .....H.....
        .....H.....
    ";
    let mut b = Battle::new(Map::from_ascii(art), rules(), 1);
    let brute = b.spawn("brute", "Brute", Team::Enemy, GridPos::new(0, 1)).unwrap();
    b.spawn("rifleman", "A", Team::Player, GridPos::new(10, 4)).unwrap();
    b.apply(Action::EndTurn).unwrap();
    let range = b.movement_range(brute);
    assert!(range.contains(GridPos::new(2, 1)), "open ground on its side");
    assert!(!range.contains(GridPos::new(6, 1)), "the 1-tile gap is too narrow for 3x3");
}

#[test]
fn big_units_are_shot_seen_and_blasted_on_any_tile_and_bite_from_any_edge() {
    let art = "
        ............
        ............
        ............
        ............
        ............
    ";
    let mut b = Battle::new(Map::from_ascii(art), rules(), 2);
    let brute = b.spawn("brute", "Brute", Team::Enemy, GridPos::new(5, 1)).unwrap();
    let a = b.spawn("rifleman", "A", Team::Player, GridPos::new(0, 0)).unwrap();
    let g = b.spawn("grenadier", "G", Team::Player, GridPos::new(11, 4)).unwrap();
    assert_eq!(b.visible_enemies(a), vec![brute]);
    // Shots hit it (a big target).
    let hp = b.unit(brute).unwrap().hp;
    for _ in 0..3 {
        if b.active_team() != Team::Player {
            b.apply(Action::EndTurn).unwrap();
            b.apply(Action::EndTurn).unwrap();
        }
        let _ = b.apply(Action::ability(a, "shoot", Target::Unit(brute)));
    }
    assert!(b.unit(brute).unwrap().hp < hp, "hit");
    // A grenade landing next to its corner catches it.
    if b.active_team() != Team::Player {
        b.apply(Action::EndTurn).unwrap();
        b.apply(Action::EndTurn).unwrap();
    }
    let before = b.unit(brute).unwrap().hp;
    b.apply(Action::ability(g, "frag_grenade", Target::Tile(GridPos::new(8, 4)))).unwrap();
    assert!(b.unit(brute).unwrap().hp < before, "blast touches its corner tile (7,3)");
    // Its bite reaches a soldier next to any tile of the footprint.
    let mut b = Battle::new(Map::from_ascii(art), rules(), 2);
    let brute = b.spawn("brute", "Brute", Team::Enemy, GridPos::new(5, 1)).unwrap();
    let s = b.spawn("rifleman", "S", Team::Player, GridPos::new(8, 3)).unwrap(); // diagonal to (7,3)
    b.apply(Action::EndTurn).unwrap();
    assert!(b.validate(&Action::ability(brute, "bite", Target::Unit(s))).is_ok());
}

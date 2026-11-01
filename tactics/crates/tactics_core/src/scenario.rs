//! Ready-made battles: a hand-authored demo map and a procedural skirmish.
//!
//! A real game would load maps from asset files (RON/JSON or a level editor);
//! the ASCII format from [`Map::from_ascii`] doubles as a simple text format.

use crate::battle::Battle;
use crate::grid::{GridPos, Map, Terrain};
use crate::pathfinding;
use crate::rng::GameRng;
use crate::unit::{Team, UnitTemplate};

/// Hand-made demo map. `A` = player spawn, `E` = enemy spawn
/// (`h` half cover, `H` full cover, `#` void). Top row is north.
pub const DEMO_MAP: &str = "
    ..................EE..
    ..HH....h.....hh..E...
    ..HH....h..........E..
    ..........HHH.........
    .hh.......H.....HH....
    ..........H.....HH..h.
    ....h.............. h.
    ....h....##...........
    .........##.....h.....
    ..HHH...........h.....
    ......h....HH.........
    ......h....HH.....hh..
    ..A...................
    .AA.....hhh.....H.....
    ..A.............H.....
    ......................
";

/// The default player squad.
fn player_squad() -> Vec<UnitTemplate> {
    vec![
        UnitTemplate::rifleman("Sgt. Vance"),
        UnitTemplate::assault("Cpl. Okoye"),
        UnitTemplate::sniper("Spc. Moreau"),
        UnitTemplate::rifleman("Pvt. Kowalski"),
    ]
}

/// Alien pod names.
fn enemy_squad(count: usize) -> Vec<UnitTemplate> {
    (0..count)
        .map(|i| UnitTemplate::alien_grunt(format!("Sectoid {}", i + 1)))
        .collect()
}

/// The hand-authored demo battle.
pub fn demo(seed: u64) -> Battle {
    let map = Map::from_ascii(DEMO_MAP);
    let mut battle = Battle::new(map, seed);
    for (template, pos) in player_squad().into_iter().zip(Map::find_markers(DEMO_MAP, 'A')) {
        battle.spawn(template, Team::Player, pos);
    }
    let enemy_spawns = Map::find_markers(DEMO_MAP, 'E');
    for (template, pos) in enemy_squad(enemy_spawns.len()).into_iter().zip(enemy_spawns) {
        battle.spawn(template, Team::Enemy, pos);
    }
    battle
}

/// Procedurally generates a map with scattered cover clusters.
///
/// Spawn zones (south-west and north-east corners) are kept clear and the map
/// is re-rolled until the two zones are connected, so every result is playable.
pub fn generate_map(width: i32, height: i32, seed: u64) -> Map {
    let mut rng = GameRng::new(seed);
    let player_zone = GridPos::new(2, 2);
    let enemy_zone = GridPos::new(width - 3, height - 3);

    loop {
        let mut map = Map::new(width, height);
        let clusters = (width * height) / 18;
        for _ in 0..clusters {
            let cx = rng.range_inclusive(0, width - 1);
            let cy = rng.range_inclusive(0, height - 1);
            let terrain = if rng.roll_percent(35) { Terrain::HighCover } else { Terrain::LowCover };
            // Clusters are short horizontal or vertical runs, like walls or barricades.
            let len = rng.range_inclusive(1, 4);
            let horizontal = rng.roll_percent(50);
            for i in 0..len {
                let p = if horizontal { GridPos::new(cx + i, cy) } else { GridPos::new(cx, cy + i) };
                map.set_terrain(p, terrain);
            }
        }
        // Clear spawn zones.
        for zone in [player_zone, enemy_zone] {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    map.set_terrain(GridPos::new(zone.x + dx, zone.y + dy), Terrain::Floor);
                }
            }
        }
        let connected = pathfinding::find_path(&map, player_zone, enemy_zone, None, |_| true).is_some();
        if connected {
            return map;
        }
    }
}

/// A procedurally generated 24×20 skirmish: 4 soldiers vs 5 aliens.
pub fn skirmish(seed: u64) -> Battle {
    let (w, h) = (24, 20);
    let map = generate_map(w, h, seed);
    let mut battle = Battle::new(map, seed.wrapping_mul(31).wrapping_add(7));
    let player_spots = [(1, 1), (2, 1), (1, 2), (3, 2)];
    for (t, (x, y)) in player_squad().into_iter().zip(player_spots) {
        battle.spawn(t, Team::Player, GridPos::new(x, y));
    }
    let enemy_spots = [(w - 2, h - 2), (w - 3, h - 2), (w - 2, h - 3), (w - 4, h - 3), (w - 3, h - 4)];
    for (t, (x, y)) in enemy_squad(enemy_spots.len()).into_iter().zip(enemy_spots) {
        battle.spawn(t, Team::Enemy, GridPos::new(x, y));
    }
    battle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_has_both_squads() {
        let b = demo(0);
        assert_eq!(b.living(Team::Player).count(), 4);
        assert!(b.living(Team::Enemy).count() >= 3);
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(generate_map(20, 16, 9), generate_map(20, 16, 9));
    }
}

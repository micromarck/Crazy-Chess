//! Ready-made battles: a hand-authored demo map and a procedural skirmish.
//!
//! A real game would load maps from asset files (RON/JSON or a level editor);
//! the ASCII format from [`Map::from_ascii`] doubles as a simple text format.

use crate::battle::Battle;
use crate::content::Ruleset;
use crate::grid::{GridPos, Map, Terrain};
use crate::pathfinding;
use crate::rng::GameRng;
use crate::unit::Team;

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

/// Spawns the ruleset's squad `squad_id` for `team` on the given tiles (extra
/// members without a tile are left behind). Problems are logged to stderr and
/// skipped so a broken data module can't crash the game.
fn deploy(battle: &mut Battle, squad_id: &str, team: Team, spots: &[GridPos]) {
    let members = battle.rules().squad(squad_id).map(|s| s.members.clone()).unwrap_or_default();
    for (member, pos) in members.iter().zip(spots) {
        if let Err(e) = battle.spawn(&member.class, member.name.clone(), team, *pos) {
            eprintln!("scenario: could not deploy {}: {e}", member.name);
        }
    }
}

/// The hand-authored demo battle (map id `"demo"`, squads `"player"` / `"enemy"`).
pub fn demo(rules: &Ruleset, seed: u64) -> Battle {
    let ascii = rules.map("demo").map_or(DEMO_MAP, |m| m.ascii.as_str()).to_string();
    let mut battle = Battle::new(Map::from_ascii(&ascii), rules.clone(), seed);
    deploy(&mut battle, "player", Team::Player, &Map::find_markers(&ascii, 'A'));
    deploy(&mut battle, "enemy", Team::Enemy, &Map::find_markers(&ascii, 'E'));
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
        let connected = pathfinding::find_path(&map, player_zone, enemy_zone, None, crate::grid::Climb::default(), |_| true).is_some();
        if connected {
            return map;
        }
    }
}

/// A procedurally generated 24×20 skirmish with the `"player"` and `"enemy"` squads.
pub fn skirmish(rules: &Ruleset, seed: u64) -> Battle {
    let (w, h) = (24, 20);
    let map = generate_map(w, h, seed);
    let mut battle = Battle::new(map, rules.clone(), seed.wrapping_mul(31).wrapping_add(7));
    let spots = |list: &[(i32, i32)]| list.iter().map(|&(x, y)| GridPos::new(x, y)).collect::<Vec<_>>();
    // Up to 6 per side; spawn zones are cleared by `generate_map`.
    let player = spots(&[(1, 1), (2, 1), (1, 2), (3, 2), (2, 3), (3, 1)]);
    let enemy = spots(&[(w - 2, h - 2), (w - 3, h - 2), (w - 2, h - 3), (w - 4, h - 3), (w - 3, h - 4), (w - 4, h - 2)]);
    deploy(&mut battle, "player", Team::Player, &player);
    deploy(&mut battle, "enemy", Team::Enemy, &enemy);
    battle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_has_both_squads() {
        let b = demo(&Ruleset::standard(), 0);
        assert_eq!(b.living(Team::Player).count(), 4);
        assert!(b.living(Team::Enemy).count() >= 3);
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(generate_map(20, 16, 9), generate_map(20, 16, 9));
    }
}

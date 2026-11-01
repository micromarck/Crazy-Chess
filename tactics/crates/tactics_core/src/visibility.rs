//! Line of sight (LOS).
//!
//! LOS is traced on the grid with a symmetric "supercover" style line: we walk
//! every tile the segment between two tile centres passes through. If any
//! intermediate tile [`blocks sight`](crate::grid::Terrain::blocks_sight), the line
//! is blocked. The start and end tiles themselves never block.
//!
//! When the segment passes *exactly* through a corner shared by two tiles we only
//! treat it as blocked if **both** of those tiles block. This is a deliberately
//! permissive rule that mimics "leaning around corners": a soldier standing next
//! to a wall corner can still see diagonally past it.
//!
//! The system is symmetric (`A sees B ⇔ B sees A`), which keeps combat fair and
//! avoids "they can shoot me but I can't shoot them" frustration.

use crate::grid::{GridPos, Map};

/// Returns `true` if there is an unobstructed line between the centres of `a` and `b`.
pub fn has_line_of_sight(map: &Map, a: GridPos, b: GridPos) -> bool {
    if a == b {
        return true;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let nx = dx.abs();
    let ny = dy.abs();
    let sign_x = dx.signum();
    let sign_y = dy.signum();

    let mut x = a.x;
    let mut y = a.y;
    let (mut ix, mut iy) = (0, 0);

    // Walk the grid one tile at a time (Amanatides–Woo style, integer arithmetic).
    // Comparing (1 + 2ix) * ny with (1 + 2iy) * nx tells us whether the line next
    // crosses a vertical edge, a horizontal edge, or a corner.
    while ix < nx || iy < ny {
        let decision = (1 + 2 * ix) * ny - (1 + 2 * iy) * nx;
        if decision == 0 {
            // Exactly through a corner: check the two side tiles.
            let side_a = GridPos::new(x + sign_x, y);
            let side_b = GridPos::new(x, y + sign_y);
            // (The side tiles can never be the endpoints: the line passes
            // diagonally between them.)
            if map.terrain(side_a).blocks_sight()
                && map.terrain(side_b).blocks_sight()
            {
                return false;
            }
            x += sign_x;
            y += sign_y;
            ix += 1;
            iy += 1;
        } else if decision < 0 {
            x += sign_x;
            ix += 1;
        } else {
            y += sign_y;
            iy += 1;
        }
        let pos = GridPos::new(x, y);
        if pos != b && map.terrain(pos).blocks_sight() {
            return false;
        }
    }
    true
}

/// Can `from` see `to` given a maximum sight range (in tiles, Euclidean)?
pub fn can_see(map: &Map, from: GridPos, to: GridPos, sight_range: f32) -> bool {
    from.distance(to) <= sight_range && has_line_of_sight(map, from, to)
}

/// All tiles visible from `from` within `sight_range`. Useful for fog of war.
pub fn visible_tiles(map: &Map, from: GridPos, sight_range: f32) -> Vec<GridPos> {
    map.positions()
        .filter(|p| can_see(map, from, *p, sight_range))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_blocks() {
        let map = Map::from_ascii(
            "
            .....
            ..H..
            .....
            ",
        );
        assert!(!has_line_of_sight(&map, GridPos::new(0, 1), GridPos::new(4, 1)));
        assert!(has_line_of_sight(&map, GridPos::new(0, 0), GridPos::new(4, 0)));
    }

    #[test]
    fn low_cover_does_not_block() {
        let map = Map::from_ascii(".h.");
        assert!(has_line_of_sight(&map, GridPos::new(0, 0), GridPos::new(2, 0)));
    }

    #[test]
    fn symmetric() {
        let map = Map::from_ascii(
            "
            ..H...
            ...H..
            H.....
            ....H.
            ",
        );
        for a in map.positions() {
            for b in map.positions() {
                assert_eq!(
                    has_line_of_sight(&map, a, b),
                    has_line_of_sight(&map, b, a),
                    "asymmetric LOS between {a} and {b}"
                );
            }
        }
    }

    #[test]
    fn diagonal_corner_peek() {
        // One wall on a corner: diagonal view is allowed.
        let map = Map::from_ascii(
            "
            ..
            H.
            ",
        );
        assert!(has_line_of_sight(&map, GridPos::new(1, 0), GridPos::new(0, 1)));
    }
}

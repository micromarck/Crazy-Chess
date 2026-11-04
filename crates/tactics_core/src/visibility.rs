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
//!
//! **Corner peeking:** a unit at the end of a wall can lean one tile to the
//! side, past the corner ([`lean_tiles`]); sight between units counts both
//! units' lean tiles ([`sight_line`], still symmetric) and shots are fired
//! from (and at) the lean tile.
//!
//! **Height:** the sight line runs between the two units' eyes
//! (ground level + [`EYE_HEIGHT`]) and an intermediate tile blocks it when its
//! top (ground level, plus [`HIGH_COVER_HEIGHT`] for full-cover terrain) rises
//! above the line there. So a 1-level step doesn't block sight, a 2-level
//! cliff does, and units on high ground see over obstacles.

use crate::grid::{GridPos, Map};

/// Eye height above the ground, in levels.
pub const EYE_HEIGHT: f32 = 1.5;
/// Height of full-cover terrain (walls) above its ground, in levels.
pub const HIGH_COVER_HEIGHT: f32 = 3.0;

/// Top of whatever stands on `pos`, in levels.
fn tile_top(map: &Map, pos: GridPos) -> f32 {
    map.elevation(pos) as f32 + if map.terrain(pos).blocks_sight() { HIGH_COVER_HEIGHT } else { 0.0 }
}

/// Returns `true` if there is an unobstructed line between the centres of `a` and `b`.
pub fn has_line_of_sight(map: &Map, a: GridPos, b: GridPos) -> bool {
    if a == b {
        return true;
    }
    // Height of the sight line above tile `p`: interpolate the eye heights by
    // projecting the tile centre onto the segment (symmetric in a and b).
    let ha = map.elevation(a) as f32 + EYE_HEIGHT;
    let hb = map.elevation(b) as f32 + EYE_HEIGHT;
    let (abx, aby) = ((b.x - a.x) as f32, (b.y - a.y) as f32);
    let len2 = abx * abx + aby * aby;
    let blocks = |p: GridPos| {
        let t = (((p.x - a.x) as f32 * abx + (p.y - a.y) as f32 * aby) / len2).clamp(0.0, 1.0);
        tile_top(map, p) > ha + (hb - ha) * t + 1e-4
    };
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
            if blocks(side_a) && blocks(side_b) {
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
        if pos != b && blocks(pos) {
            return false;
        }
    }
    true
}

/// **Corner peeking.** Where a unit standing on `pos` can lean out to look
/// (and shoot) past a wall corner: a wall (full cover) is directly in front
/// of it in some direction, and one tile to the side the wall ends. The
/// lean tile is that side tile (it must be open, and at about the same
/// height). In a corridor, or in the middle of a long wall, there's nothing
/// to lean around.
///
/// ```text
///   . . . .        A stands at the end of the wall (#) and can lean to L
///   # # A L        to look north past the corner.
/// ```
pub fn lean_tiles(map: &Map, pos: GridPos) -> Vec<GridPos> {
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let open = |p: GridPos| map.in_bounds(p) && !map.terrain(p).blocks_sight() && map.terrain(p) != crate::grid::Terrain::Void;
    let mut out = Vec::new();
    for (fx, fy) in DIRS {
        let front = GridPos::new(pos.x + fx, pos.y + fy);
        if !map.in_bounds(front) || !map.terrain(front).blocks_sight() {
            continue;
        }
        for (sx, sy) in [(fy, fx), (-fy, -fx)] {
            let side = GridPos::new(pos.x + sx, pos.y + sy);
            let past = GridPos::new(side.x + fx, side.y + fy);
            if open(side) && open(past) && (map.elevation(side) - map.elevation(pos)).abs() <= 1 && !out.contains(&side) {
                out.push(side);
            }
        }
    }
    out
}

/// Line of sight with **corner peeking**: the pair of points (where `a`
/// looks from, where it sees `b`) - the tiles themselves when the direct
/// line is clear, else lean tiles ([`lean_tiles`]) of either side. `None`
/// = no sight. Symmetric: if `a` sees `b`, `b` sees `a`. Of several leans,
/// `a` leaning out is preferred (so `b` is seen where it stands).
pub fn sight_line(map: &Map, a: GridPos, b: GridPos) -> Option<(GridPos, GridPos)> {
    if has_line_of_sight(map, a, b) {
        return Some((a, b));
    }
    sight_line_leaning(map, a, &lean_tiles(map, a), b)
}

/// [`sight_line`] past the direct line, with `a`'s lean tiles given.
fn sight_line_leaning(map: &Map, a: GridPos, la: &[GridPos], b: GridPos) -> Option<(GridPos, GridPos)> {
    if let Some(pa) = la.iter().copied().find(|pa| *pa == b || has_line_of_sight(map, *pa, b)) {
        return Some((pa, b));
    }
    let lb = lean_tiles(map, b);
    for pb in &lb {
        if *pb == a || has_line_of_sight(map, a, *pb) {
            return Some((a, *pb));
        }
    }
    for pa in la {
        for pb in &lb {
            if has_line_of_sight(map, *pa, *pb) {
                return Some((*pa, *pb));
            }
        }
    }
    None
}

/// Line of sight including corner peeking (see [`sight_line`]).
pub fn has_sight(map: &Map, a: GridPos, b: GridPos) -> bool {
    sight_line(map, a, b).is_some()
}

/// Can `from` see `to` given a maximum sight range (in tiles, Euclidean)?
/// Includes corner peeking ([`sight_line`]).
pub fn can_see(map: &Map, from: GridPos, to: GridPos, sight_range: f32) -> bool {
    from.distance(to) <= sight_range && has_sight(map, from, to)
}

/// All tiles visible from `from` within `sight_range`. Useful for fog of war.
/// Only the square around `from` is searched, so it is cheap on huge maps.
pub fn visible_tiles(map: &Map, from: GridPos, sight_range: f32) -> Vec<GridPos> {
    let r = sight_range.max(0.0).ceil() as i32;
    let lean = lean_tiles(map, from);
    let mut out = Vec::new();
    for y in (from.y - r).max(0)..=(from.y + r).min(map.height() - 1) {
        for x in (from.x - r).max(0)..=(from.x + r).min(map.width() - 1) {
            let p = GridPos::new(x, y);
            if from.distance(p) <= sight_range && (has_line_of_sight(map, from, p) || sight_line_leaning(map, from, &lean, p).is_some()) {
                out.push(p);
            }
        }
    }
    out
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
    fn height_blocks_and_reveals() {
        let mut map = Map::new(7, 1);
        let (a, b) = (GridPos::new(0, 0), GridPos::new(6, 0));
        map.set_elevation(GridPos::new(3, 0), 1);
        assert!(has_line_of_sight(&map, a, b), "a 1-level step doesn't block");
        map.set_elevation(GridPos::new(3, 0), 2);
        assert!(!has_line_of_sight(&map, a, b), "a 2-level ridge does");
        map.set_elevation(a, 2);
        map.set_elevation(b, 2);
        assert!(has_line_of_sight(&map, a, b), "both on high ground: they see over it");
        // High ground sees over a wall that blocks at ground level.
        let mut map = Map::from_ascii(".....H.");
        assert!(!has_line_of_sight(&map, GridPos::new(0, 0), GridPos::new(6, 0)));
        map.set_elevation(GridPos::new(0, 0), 4);
        map.set_elevation(GridPos::new(6, 0), 4);
        assert!(has_line_of_sight(&map, GridPos::new(0, 0), GridPos::new(6, 0)));
    }

    #[test]
    fn symmetric_with_heights() {
        let mut map = Map::from_ascii(
            "
            ..H...
            ...H..
            H.....
            ....H.
            ",
        );
        for (i, p) in map.positions().collect::<Vec<_>>().into_iter().enumerate() {
            map.set_elevation(p, ((i * 7) % 4) as i8);
        }
        for a in map.positions() {
            for b in map.positions() {
                assert_eq!(has_line_of_sight(&map, a, b), has_line_of_sight(&map, b, a), "{a} {b}");
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

    /// Standing at the end of a wall, a unit leans past the corner: it sees
    /// (and is seen by) what's behind the wall on the other side; in the
    /// middle of the wall it doesn't.
    #[test]
    fn corner_peeking() {
        // y grows upwards in GridPos (row 0 of the art is the top line).
        let map = Map::from_ascii(
            "
            ......
            ......
            HHHH..
            ......
            ",
        );
        let h = map.height();
        let at = |x: i32, row: i32| GridPos::new(x, h - 1 - row);
        let corner = at(3, 3); // just south of the wall's east end
        let behind = at(3, 0); // north of the wall
        assert!(!has_line_of_sight(&map, corner, behind), "the wall blocks the straight line");
        assert_eq!(lean_tiles(&map, corner), vec![at(4, 3)]);
        assert_eq!(sight_line(&map, corner, behind), Some((at(4, 3), behind)));
        assert!(has_sight(&map, behind, corner), "symmetric");
        let middle = at(1, 3);
        assert!(lean_tiles(&map, middle).is_empty(), "nothing to lean around mid-wall");
        assert!(!has_sight(&map, middle, at(1, 0)));
        assert!(can_see(&map, corner, behind, 10.0));
        assert!(!can_see(&map, corner, behind, 2.0), "range still counts from the unit");
    }

    /// Peeking stays symmetric on a cluttered map.
    #[test]
    fn peeking_is_symmetric() {
        let map = Map::from_ascii(
            "
            ..H...H..
            .HH.H....
            ....H.HH.
            HH.....H.
            ..H.HH...
            ",
        );
        for y in 0..map.height() {
            for x in 0..map.width() {
                for y2 in 0..map.height() {
                    for x2 in 0..map.width() {
                        let (a, b) = (GridPos::new(x, y), GridPos::new(x2, y2));
                        assert_eq!(has_sight(&map, a, b), has_sight(&map, b, a), "{a} {b}");
                    }
                }
            }
        }
    }
}

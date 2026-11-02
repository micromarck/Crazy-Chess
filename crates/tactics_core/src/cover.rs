//! Cover and flanking.
//!
//! A unit is "in cover" against an attacker when an obstacle sits on a tile
//! **orthogonally adjacent** to the unit, on a side that faces the attacker.
//!
//! For each cardinal direction we check whether the attacker lies on that side
//! of the defender (strictly positive offset along the axis). Each such side
//! whose neighbouring tile is cover contributes; the best one wins. If the
//! attacker is on a side with no cover at all, the defender is **flanked**.
//!
//! ```text
//!   . . . A      A is north-east of D. D's north and east neighbours are checked.
//!   . h . .      Neither is cover (the `h` is north-WEST), so D is flanked.
//!   . . D .
//! ```
//!
//! This is the classic XCOM model: cover is directional, and repositioning to
//! hit a target from an uncovered side is the core tactical decision.
//!
//! Terrain height counts as cover too: a neighbouring tile 1 level higher than
//! the defender is half cover, 2+ levels is full cover (see
//! [`Map::cover_from`]).

use crate::grid::{CoverLevel, Direction, GridPos, Map};

/// The outcome of evaluating cover between an attacker and a defender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverResult {
    /// Best cover the defender has against this attacker.
    pub level: CoverLevel,
    /// `true` when the defender has **no** cover facing the attacker.
    pub flanked: bool,
}

/// Cover provided by each cardinal neighbour of `pos`, in
/// [`Direction::CARDINAL`] order (N, E, S, W). Handy for drawing shield icons.
pub fn cover_around(map: &Map, pos: GridPos) -> [CoverLevel; 4] {
    Direction::CARDINAL.map(|d| map.cover_from(pos.step(d), pos))
}

/// Best cover the tile `pos` offers in any direction (ignores attacker position).
/// Used by the AI to value positions before knowing who will shoot at them.
pub fn best_cover_at(map: &Map, pos: GridPos) -> CoverLevel {
    cover_around(map, pos).into_iter().max().unwrap_or_default()
}

/// Directional cover of a defender at `defender` against an attacker at `attacker`.
pub fn cover_against(map: &Map, defender: GridPos, attacker: GridPos) -> CoverResult {
    let dx = attacker.x - defender.x;
    let dy = attacker.y - defender.y;

    let mut best = CoverLevel::None;
    for dir in Direction::CARDINAL {
        let (cx, cy) = dir.delta();
        // Is the attacker on this side of the defender?
        let facing = (cx != 0 && dx.signum() == cx) || (cy != 0 && dy.signum() == cy);
        if facing {
            best = best.max(map.cover_from(defender.step(dir), defender));
        }
    }

    CoverResult {
        level: best,
        flanked: best == CoverLevel::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ART: &str = "
        .......
        .......
        ...h...
        ...D...
        .......
        .......
    ";

    #[test]
    fn cover_from_the_front_flanked_from_the_side() {
        let map = Map::from_ascii(ART);
        let d = Map::find_markers(ART, 'D')[0];

        // Attacker straight north: the half-cover tile is between them.
        let north = cover_against(&map, d, GridPos::new(d.x, d.y + 3));
        assert_eq!(north.level, CoverLevel::Half);
        assert!(!north.flanked);

        // Attacker diagonally north-east still faces the northern cover.
        let ne = cover_against(&map, d, GridPos::new(d.x + 3, d.y + 2));
        assert_eq!(ne.level, CoverLevel::Half);

        // Attacker due east: no cover on the east side.
        let east = cover_against(&map, d, GridPos::new(d.x + 3, d.y));
        assert!(east.flanked);

        // Attacker south: cover is behind the defender, useless.
        let south = cover_against(&map, d, GridPos::new(d.x, d.y - 2));
        assert!(south.flanked);
    }
}

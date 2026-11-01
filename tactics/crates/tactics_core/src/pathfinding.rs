//! Movement range and path search.
//!
//! Movement is measured in *movement points* (MP). An orthogonal step costs
//! [`ORTHOGONAL_COST`] and a diagonal step [`DIAGONAL_COST`], approximating the
//! √2 ratio with integers so results are exact and deterministic.
//!
//! Two queries are provided:
//! * [`reachable`] — a Dijkstra flood fill returning every tile reachable within a
//!   budget, plus back-pointers so any of those tiles can be turned into a path
//!   instantly. This drives the blue/yellow movement overlay in the UI.
//! * [`find_path`] — A* between two points with an optional budget, for AI and
//!   for one-off queries.
//!
//! Both take a `passable` predicate so the caller decides what blocks movement
//! (other units, hazards, doors...). The map's own walkability is always enforced.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::grid::{GridPos, Map};

/// Cost of a north/east/south/west step.
pub const ORTHOGONAL_COST: u32 = 2;
/// Cost of a diagonal step (≈ 1.5 × orthogonal).
pub const DIAGONAL_COST: u32 = 3;

/// Cost of stepping in a direction.
fn step_cost(diagonal: bool) -> u32 {
    if diagonal {
        DIAGONAL_COST
    } else {
        ORTHOGONAL_COST
    }
}

/// Result of a [`reachable`] flood fill.
#[derive(Debug, Clone, Default)]
pub struct ReachableSet {
    origin: GridPos,
    /// For each reachable tile: (total cost from origin, previous tile).
    nodes: HashMap<GridPos, (u32, GridPos)>,
}

impl ReachableSet {
    /// Total movement cost to reach `pos`, if reachable.
    pub fn cost(&self, pos: GridPos) -> Option<u32> {
        self.nodes.get(&pos).map(|(c, _)| *c)
    }

    /// Is `pos` reachable (the origin itself counts)?
    pub fn contains(&self, pos: GridPos) -> bool {
        self.nodes.contains_key(&pos)
    }

    /// Iterates over every reachable tile and its cost (origin excluded).
    pub fn iter(&self) -> impl Iterator<Item = (GridPos, u32)> + '_ {
        self.nodes
            .iter()
            .filter(move |(p, _)| **p != self.origin)
            .map(|(p, (c, _))| (*p, *c))
    }

    /// Reconstructs the path from the origin to `target`, **excluding** the
    /// origin and **including** the target. Returns `None` if unreachable.
    pub fn path_to(&self, target: GridPos) -> Option<Vec<GridPos>> {
        if !self.contains(target) {
            return None;
        }
        let mut path = Vec::new();
        let mut current = target;
        while current != self.origin {
            path.push(current);
            current = self.nodes[&current].1;
        }
        path.reverse();
        Some(path)
    }
}

/// Dijkstra flood fill from `origin`, visiting tiles whose total cost is `<= budget`.
///
/// `passable(pos)` is consulted for every tile except the origin (the moving unit
/// stands there, so it's obviously allowed).
pub fn reachable(
    map: &Map,
    origin: GridPos,
    budget: u32,
    passable: impl Fn(GridPos) -> bool,
) -> ReachableSet {
    let mut nodes: HashMap<GridPos, (u32, GridPos)> = HashMap::new();
    nodes.insert(origin, (0, origin));
    // Reverse turns the max-heap into a min-heap. Position is a deterministic tiebreak.
    let mut frontier = BinaryHeap::new();
    frontier.push(Reverse((0u32, origin)));

    while let Some(Reverse((cost, pos))) = frontier.pop() {
        if cost > nodes[&pos].0 {
            continue; // Stale queue entry: a cheaper route was already found.
        }
        for (next, dir) in map.walkable_neighbors(pos) {
            if !passable(next) {
                continue;
            }
            let new_cost = cost + step_cost(dir.is_diagonal());
            if new_cost > budget {
                continue;
            }
            let better = nodes.get(&next).is_none_or(|(old, _)| new_cost < *old);
            if better {
                nodes.insert(next, (new_cost, pos));
                frontier.push(Reverse((new_cost, next)));
            }
        }
    }

    ReachableSet { origin, nodes }
}

/// Admissible A* heuristic for 8-way movement with the costs above ("octile").
fn octile(a: GridPos, b: GridPos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    let (lo, hi) = if dx < dy { (dx, dy) } else { (dy, dx) };
    DIAGONAL_COST * lo + ORTHOGONAL_COST * (hi - lo)
}

/// A* search from `start` to `goal`.
///
/// Returns the path (excluding `start`, including `goal`) and its total cost, or
/// `None` when the goal is unreachable or would cost more than `max_cost`.
pub fn find_path(
    map: &Map,
    start: GridPos,
    goal: GridPos,
    max_cost: Option<u32>,
    passable: impl Fn(GridPos) -> bool,
) -> Option<(Vec<GridPos>, u32)> {
    if !map.is_walkable(goal) || !passable(goal) {
        return None;
    }
    let mut came_from: HashMap<GridPos, GridPos> = HashMap::new();
    let mut g_score: HashMap<GridPos, u32> = HashMap::new();
    g_score.insert(start, 0);
    let mut open = BinaryHeap::new();
    open.push(Reverse((octile(start, goal), 0u32, start)));

    while let Some(Reverse((_, g, pos))) = open.pop() {
        if pos == goal {
            let mut path = vec![goal];
            let mut cur = goal;
            while let Some(prev) = came_from.get(&cur) {
                if *prev == start {
                    break;
                }
                path.push(*prev);
                cur = *prev;
            }
            path.reverse();
            return Some((path, g));
        }
        if g > g_score[&pos] {
            continue;
        }
        for (next, dir) in map.walkable_neighbors(pos) {
            if !passable(next) {
                continue;
            }
            let tentative = g + step_cost(dir.is_diagonal());
            if max_cost.is_some_and(|m| tentative > m) {
                continue;
            }
            if g_score.get(&next).is_none_or(|old| tentative < *old) {
                g_score.insert(next, tentative);
                came_from.insert(next, pos);
                open.push(Reverse((tentative + octile(next, goal), tentative, next)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachable_respects_budget() {
        let map = Map::new(10, 10);
        let set = reachable(&map, GridPos::new(5, 5), 4, |_| true);
        assert_eq!(set.cost(GridPos::new(7, 5)), Some(4)); // two orthogonal steps
        assert!(!set.contains(GridPos::new(8, 5))); // three steps = 6 > 4
        assert_eq!(set.cost(GridPos::new(6, 6)), Some(3)); // one diagonal
    }

    #[test]
    fn path_goes_around_walls() {
        let map = Map::from_ascii(
            "
            .....
            .HHH.
            .....
            ",
        );
        let (path, cost) =
            find_path(&map, GridPos::new(2, 0), GridPos::new(2, 2), None, |_| true).unwrap();
        assert_eq!(*path.last().unwrap(), GridPos::new(2, 2));
        assert!(path.iter().all(|p| map.is_walkable(*p)));
        // Must go around: at least 4 steps.
        assert!(path.len() >= 4);
        assert!(cost >= 10);
    }

    #[test]
    fn reachable_path_matches_cost() {
        let map = Map::new(8, 8);
        let set = reachable(&map, GridPos::new(0, 0), 20, |_| true);
        let path = set.path_to(GridPos::new(3, 1)).unwrap();
        assert_eq!(path.last(), Some(&GridPos::new(3, 1)));
        assert_eq!(set.cost(GridPos::new(3, 1)), Some(DIAGONAL_COST + 2 * ORTHOGONAL_COST));
    }

    #[test]
    fn blocked_by_predicate() {
        let map = Map::new(3, 1);
        let blocker = GridPos::new(1, 0);
        assert!(find_path(&map, GridPos::new(0, 0), GridPos::new(2, 0), None, |p| p != blocker).is_none());
    }
}

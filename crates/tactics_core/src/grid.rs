//! The tactical map: a rectangular grid of tiles.
//!
//! Coordinates are integer `(x, y)` pairs where `x` grows to the "east" and `y`
//! grows to the "north". The renderer maps this onto its own world space (in the
//! Bevy frontend: `x -> X`, `y -> -Z`, height -> `Y`), so the core never needs to know
//! about 3D at all.
//!
//! Each tile has a [`Terrain`] which decides three independent properties:
//! * whether units can stand on / walk through it,
//! * whether it blocks line of sight,
//! * how much [`CoverLevel`] it grants to a unit standing *next* to it.
//!
//! Keeping those as derived properties of a single enum keeps map data compact
//! and makes it easy to add new terrain (e.g. destructible cars, smoke) later.

use std::fmt;

/// An integer tile coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

impl GridPos {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Returns this position shifted by a direction.
    pub fn step(self, dir: Direction) -> Self {
        let (dx, dy) = dir.delta();
        Self::new(self.x + dx, self.y + dy)
    }

    /// Chebyshev distance: the number of king moves between two tiles.
    pub fn chebyshev(self, other: Self) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }

    /// Straight-line (Euclidean) distance in tiles. Used for weapon/sight range.
    pub fn distance(self, other: Self) -> f32 {
        let dx = (self.x - other.x) as f32;
        let dy = (self.y - other.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
}

impl fmt::Display for GridPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// The eight compass directions used for movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Direction {
    /// All eight directions, cardinal ones first.
    pub const ALL: [Direction; 8] = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
        Direction::NorthEast,
        Direction::SouthEast,
        Direction::SouthWest,
        Direction::NorthWest,
    ];

    /// The four cardinal directions. Cover is always evaluated along these.
    pub const CARDINAL: [Direction; 4] = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ];

    /// The `(dx, dy)` offset of one step in this direction.
    pub const fn delta(self) -> (i32, i32) {
        match self {
            Direction::North => (0, 1),
            Direction::NorthEast => (1, 1),
            Direction::East => (1, 0),
            Direction::SouthEast => (1, -1),
            Direction::South => (0, -1),
            Direction::SouthWest => (-1, -1),
            Direction::West => (-1, 0),
            Direction::NorthWest => (-1, 1),
        }
    }

    /// `true` for the four diagonal directions.
    pub const fn is_diagonal(self) -> bool {
        let (dx, dy) = self.delta();
        dx != 0 && dy != 0
    }
}

/// How much protection a piece of terrain offers to an adjacent unit.
///
/// The ordering matters: `None < Half < Full`, so `max()` picks the best cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum CoverLevel {
    #[default]
    None,
    /// Low obstacles (crates, sandbags, car hoods). Units can shoot over them.
    Half,
    /// Tall obstacles (walls, pillars). Block line of sight entirely.
    Full,
}

/// What occupies a tile, independent of units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Terrain {
    /// Open ground. Walkable, transparent, grants no cover.
    #[default]
    Floor,
    /// Low obstacle: not walkable, does not block sight, grants half cover.
    LowCover,
    /// Tall obstacle: not walkable, blocks sight, grants full cover.
    HighCover,
    /// Out-of-bounds / chasm: not walkable, does not block sight, no cover.
    Void,
}

impl Terrain {
    /// Can a unit stand on or path through this tile?
    pub const fn is_walkable(self) -> bool {
        matches!(self, Terrain::Floor)
    }

    /// Does this tile stop line of sight (and therefore shots)?
    pub const fn blocks_sight(self) -> bool {
        matches!(self, Terrain::HighCover)
    }

    /// The cover this tile gives to a unit standing directly beside it.
    pub const fn cover(self) -> CoverLevel {
        match self {
            Terrain::LowCover => CoverLevel::Half,
            Terrain::HighCover => CoverLevel::Full,
            Terrain::Floor | Terrain::Void => CoverLevel::None,
        }
    }
}

/// How a tile looks (purely visual; rules only read [`Terrain`]). Floor
/// styles colour the ground, wall styles shape cover blocks.
///
/// | char | surface | | char | surface |
/// |---|---|---|---|---|
/// | `.` | `Default` | | `B` | `Brick` (building wall) |
/// | `c` | `Concrete` | | `C` | `Container` |
/// | `a` | `Asphalt` (road) | | `G` | `Hedge` |
/// | `g` | `Grass` | | `T` | `Tree` |
/// | `d` | `Dirt` | | `R` | `Rock` |
/// | `w` | `Interior` (wooden floor) | | `S` | `Sandbags` |
/// | `r` | `Roof` | | `F` | `Fence` |
/// | `m` | `Metal` (gratings, walkways) | | `W` | `Water` (for cut-outs) |
/// | `l` | `Lines` (road markings) | | | |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Surface {
    #[default]
    Default,
    Concrete,
    Asphalt,
    Lines,
    Grass,
    Dirt,
    Interior,
    Roof,
    Metal,
    Brick,
    Container,
    Hedge,
    Tree,
    Rock,
    Sandbags,
    Fence,
    Water,
    /// Broken wall: a heap of debris (half cover).
    Rubble,
}

impl Surface {
    pub const ALL: [Surface; 18] = [
        Surface::Default,
        Surface::Concrete,
        Surface::Asphalt,
        Surface::Lines,
        Surface::Grass,
        Surface::Dirt,
        Surface::Interior,
        Surface::Roof,
        Surface::Metal,
        Surface::Brick,
        Surface::Container,
        Surface::Hedge,
        Surface::Tree,
        Surface::Rock,
        Surface::Sandbags,
        Surface::Fence,
        Surface::Water,
        Surface::Rubble,
    ];

    /// The character used in mission `<surfaces>` grids.
    pub fn to_char(self) -> char {
        match self {
            Surface::Default => '.',
            Surface::Concrete => 'c',
            Surface::Asphalt => 'a',
            Surface::Lines => 'l',
            Surface::Grass => 'g',
            Surface::Dirt => 'd',
            Surface::Interior => 'w',
            Surface::Roof => 'r',
            Surface::Metal => 'm',
            Surface::Brick => 'B',
            Surface::Container => 'C',
            Surface::Hedge => 'G',
            Surface::Tree => 'T',
            Surface::Rock => 'R',
            Surface::Sandbags => 'S',
            Surface::Fence => 'F',
            Surface::Water => 'W',
            Surface::Rubble => 'u',
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.to_char() == c)
    }
}

/// A single map cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tile {
    pub terrain: Terrain,
    /// Height level of the ground (0 = base level). Units can step up at most
    /// their `climb` stat (1 by default) in one move; dropping down is free.
    pub elevation: i8,
    /// A ladder/ramp: any height difference may be climbed into or out of it.
    pub ladder: bool,
    /// Visual style (ground material or wall type).
    pub surface: Surface,
}

/// How a unit may change height while moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Climb {
    /// Maximum levels a unit can step **up** in one step (ladders ignore this).
    pub max_up: i8,
}

impl Default for Climb {
    fn default() -> Self {
        Self { max_up: 1 }
    }
}

/// A rectangular tile map stored in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    width: i32,
    height: i32,
    tiles: Vec<Tile>,
}

impl Map {
    /// Creates a map filled with open floor.
    ///
    /// # Panics
    /// Panics if either dimension is not positive.
    pub fn new(width: i32, height: i32) -> Self {
        assert!(width > 0 && height > 0, "map dimensions must be positive");
        Self {
            width,
            height,
            tiles: vec![Tile::default(); (width * height) as usize],
        }
    }

    /// Builds a map from ASCII art, which is handy for tests and hand-made levels.
    ///
    /// The **first** line is the **northern-most** row (highest `y`), so the text
    /// looks like the map seen from above. Legend:
    /// `.` floor, `h` low (half) cover, `H` high (full) cover, `#` void.
    /// Any other character is treated as floor, letting callers annotate spawn
    /// points (e.g. `A`, `E`) and read them back via [`Map::find_markers`].
    pub fn from_ascii(art: &str) -> Self {
        let rows: Vec<&str> = art
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let height = rows.len() as i32;
        let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as i32;
        let mut map = Map::new(width.max(1), height.max(1));
        for (row_index, row) in rows.iter().enumerate() {
            let y = height - 1 - row_index as i32;
            for (x, ch) in row.chars().enumerate() {
                let terrain = match ch {
                    'h' => Terrain::LowCover,
                    'H' => Terrain::HighCover,
                    '#' => Terrain::Void,
                    _ => Terrain::Floor,
                };
                map.set_terrain(GridPos::new(x as i32, y), terrain);
            }
        }
        map
    }

    /// Returns every position in `art` (same layout as [`Map::from_ascii`]) whose
    /// character equals `marker`.
    pub fn find_markers(art: &str, marker: char) -> Vec<GridPos> {
        let rows: Vec<&str> = art
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let height = rows.len() as i32;
        let mut out = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                if ch == marker {
                    out.push(GridPos::new(x as i32, height - 1 - row_index as i32));
                }
            }
        }
        out
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// Is the position inside the map rectangle?
    pub fn in_bounds(&self, pos: GridPos) -> bool {
        pos.x >= 0 && pos.y >= 0 && pos.x < self.width && pos.y < self.height
    }

    fn index(&self, pos: GridPos) -> Option<usize> {
        self.in_bounds(pos)
            .then(|| (pos.y * self.width + pos.x) as usize)
    }

    /// Returns the tile at `pos`, or `None` when out of bounds.
    pub fn tile(&self, pos: GridPos) -> Option<&Tile> {
        self.index(pos).map(|i| &self.tiles[i])
    }

    /// Terrain at `pos`. Out-of-bounds positions read as [`Terrain::Void`], which
    /// lets algorithms treat the map edge like any other non-walkable tile.
    pub fn terrain(&self, pos: GridPos) -> Terrain {
        self.tile(pos).map_or(Terrain::Void, |t| t.terrain)
    }

    /// Changes the terrain at `pos`. Silently ignores out-of-bounds positions.
    pub fn set_terrain(&mut self, pos: GridPos, terrain: Terrain) {
        if let Some(i) = self.index(pos) {
            self.tiles[i].terrain = terrain;
        }
    }

    /// Convenience: is `pos` inside the map and walkable terrain?
    /// (Unit occupancy is checked separately by the battle state.)
    pub fn is_walkable(&self, pos: GridPos) -> bool {
        self.terrain(pos).is_walkable()
    }

    /// Ground height level at `pos` (0 outside the map).
    pub fn elevation(&self, pos: GridPos) -> i8 {
        self.tile(pos).map_or(0, |t| t.elevation)
    }

    pub fn set_elevation(&mut self, pos: GridPos, level: i8) {
        if let Some(i) = self.index(pos) {
            self.tiles[i].elevation = level;
        }
    }

    /// Is there a ladder/ramp on `pos`?
    pub fn is_ladder(&self, pos: GridPos) -> bool {
        self.tile(pos).is_some_and(|t| t.ladder)
    }

    pub fn set_ladder(&mut self, pos: GridPos, ladder: bool) {
        if let Some(i) = self.index(pos) {
            self.tiles[i].ladder = ladder;
        }
    }

    /// Visual style at `pos`.
    pub fn surface(&self, pos: GridPos) -> Surface {
        self.tile(pos).map_or(Surface::Default, |t| t.surface)
    }

    pub fn set_surface(&mut self, pos: GridPos, surface: Surface) {
        if let Some(i) = self.index(pos) {
            self.tiles[i].surface = surface;
        }
    }

    /// Gives an unstyled map a plausible look (missions without a
    /// `<surfaces>` layer): concrete ground, roofs on raised tiles, brick for
    /// runs of full cover (walls), containers for lone tall blocks, sandbags
    /// for low cover. Tiles that already have a surface are left alone.
    pub fn auto_surfaces(&mut self) {
        let snapshot = self.clone();
        for p in snapshot.positions() {
            if snapshot.surface(p) != Surface::Default {
                continue;
            }
            let terrain = snapshot.terrain(p);
            let neighbours = |t: Terrain| {
                [Direction::North, Direction::East, Direction::South, Direction::West].iter().filter(|d| snapshot.terrain(p.step(**d)) == t).count()
            };
            let s = match terrain {
                Terrain::HighCover if neighbours(Terrain::HighCover) >= 1 => Surface::Brick,
                Terrain::HighCover => Surface::Container,
                Terrain::LowCover => Surface::Sandbags,
                Terrain::Floor if snapshot.elevation(p) > 0 => Surface::Roof,
                Terrain::Floor => Surface::Concrete,
                Terrain::Void => continue,
            };
            self.set_surface(p, s);
        }
    }

    /// Highest elevation on the map.
    pub fn max_elevation(&self) -> i8 {
        self.tiles.iter().map(|t| t.elevation).max().unwrap_or(0)
    }

    /// May a unit with `climb` step from `from` to the adjacent `to` height-wise?
    /// Stepping **up** more than `climb.max_up` levels needs a ladder on either
    /// tile; stepping down is always allowed.
    pub fn can_step(&self, from: GridPos, to: GridPos, climb: Climb) -> bool {
        let rise = self.elevation(to) - self.elevation(from);
        rise <= climb.max_up || self.is_ladder(from) || self.is_ladder(to)
    }

    /// Levels climbed going from `from` to `to` (0 when level or descending).
    pub fn rise(&self, from: GridPos, to: GridPos) -> u32 {
        (self.elevation(to) - self.elevation(from)).max(0) as u32
    }

    /// Cover that the tile `neighbor` gives to a unit standing at `defender`:
    /// its terrain's cover, or the ground itself when it rises above the
    /// defender (1 level = half cover, 2+ = full cover).
    pub fn cover_from(&self, neighbor: GridPos, defender: GridPos) -> CoverLevel {
        let terrain = self.terrain(neighbor).cover();
        let wall = match self.elevation(neighbor) - self.elevation(defender) {
            d if d >= 2 => CoverLevel::Full,
            1 => CoverLevel::Half,
            _ => CoverLevel::None,
        };
        terrain.max(wall)
    }

    /// Iterates over every position on the map, row by row.
    pub fn positions(&self) -> impl Iterator<Item = GridPos> + '_ {
        (0..self.height).flat_map(move |y| (0..self.width).map(move |x| GridPos::new(x, y)))
    }

    /// Walkable neighbours of `pos` together with the direction taken.
    ///
    /// Diagonal steps are only allowed when **both** adjacent orthogonal tiles are
    /// walkable, which prevents units from slipping between two touching walls
    /// ("corner cutting").
    ///
    /// Height is ignored here; see [`Map::climbable_neighbors`].
    pub fn walkable_neighbors(&self, pos: GridPos) -> impl Iterator<Item = (GridPos, Direction)> + '_ {
        self.climbable_neighbors(pos, Climb { max_up: i8::MAX })
    }

    /// Like [`Map::walkable_neighbors`] but also enforcing the climbing rule.
    /// A diagonal step must be possible via both orthogonal side tiles, so
    /// units can't slip diagonally around a ledge.
    pub fn climbable_neighbors(&self, pos: GridPos, climb: Climb) -> impl Iterator<Item = (GridPos, Direction)> + '_ {
        Direction::ALL.into_iter().filter_map(move |dir| {
            let next = pos.step(dir);
            if !self.is_walkable(next) || !self.can_step(pos, next, climb) {
                return None;
            }
            if dir.is_diagonal() {
                let (dx, dy) = dir.delta();
                let side_a = GridPos::new(pos.x + dx, pos.y);
                let side_b = GridPos::new(pos.x, pos.y + dy);
                for side in [side_a, side_b] {
                    if !self.is_walkable(side) || !self.can_step(pos, side, climb) || !self.can_step(side, next, climb) {
                        return None;
                    }
                }
            }
            Some((next, dir))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trip_orientation() {
        // Top line is the highest y.
        let map = Map::from_ascii(
            "
            H..
            ...
            ..h
            ",
        );
        assert_eq!(map.width(), 3);
        assert_eq!(map.height(), 3);
        assert_eq!(map.terrain(GridPos::new(0, 2)), Terrain::HighCover);
        assert_eq!(map.terrain(GridPos::new(2, 0)), Terrain::LowCover);
        assert_eq!(map.terrain(GridPos::new(-1, 0)), Terrain::Void);
    }

    #[test]
    fn climbing_limits_and_ladders() {
        let mut map = Map::new(4, 1);
        map.set_elevation(GridPos::new(1, 0), 1);
        map.set_elevation(GridPos::new(2, 0), 3);
        let climb = Climb::default();
        let (a, b, c) = (GridPos::new(0, 0), GridPos::new(1, 0), GridPos::new(2, 0));
        assert!(map.can_step(a, b, climb), "one level up is fine");
        assert!(!map.can_step(b, c, climb), "two levels up is not");
        assert!(map.can_step(c, b, climb), "dropping down is always fine");
        assert!(map.can_step(b, c, Climb { max_up: 2 }), "unless the unit can climb more");
        map.set_ladder(c, true);
        assert!(map.can_step(b, c, climb), "or there is a ladder");
        assert_eq!(map.cover_from(c, b), CoverLevel::Full, "a 2-level wall is full cover");
        assert_eq!(map.cover_from(b, a), CoverLevel::Half);
    }

    #[test]
    fn no_corner_cutting() {
        let map = Map::from_ascii(
            "
            .H
            H.
            ",
        );
        let from = GridPos::new(0, 0);
        assert!(map
            .walkable_neighbors(from)
            .all(|(p, _)| p != GridPos::new(1, 1)));
    }
}

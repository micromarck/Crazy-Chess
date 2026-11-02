//! Procedural missions: themed maps with objectives.
//!
//! [`generate`] builds a complete [`MissionDef`] (map, surfaces, heights,
//! doors, props, marked zones, enemy pods and patrols, objective scripts), so
//! generated levels use every mechanic hand-made missions do and play exactly
//! like them. Same [`GenParams`] (including the seed) = same mission.
//!
//! **Themes**
//! * [`Theme::Urban`]: a street grid; blocks hold buildings (brick walls,
//!   windows, doors, wooden floors), flat roofs with parapets and ladders,
//!   plazas with trees and hedges; cars, barrels and crates on the streets.
//! * [`Theme::Industrial`]: an asphalt yard with warehouses, rows of
//!   shipping containers (some stacked high, with ladders), fences and fuel
//!   barrels.
//! * [`Theme::Wilds`]: grassland with hills (gentle steps), a river with
//!   bridges, woods, rocks, hedgerows, dirt tracks and a cabin.
//!
//! **Objectives**: [`Objective::Eliminate`] (kill everything),
//! [`Objective::Extract`] (get the whole squad to the evac point; killing
//! everything also wins), [`Objective::Hold`] (keep enemies off a beacon for
//! six turns against waves), [`Objective::Recover`] (grab the data from a
//! terminal, then extract with it).
//!
//! **Playability**: the squad deploys at the south edge; everything the
//! mission needs (objectives, enemies, evac) is placed only on tiles
//! reachable from there (doors count as open, climbing obeys the 1-level
//! rule and ladders).

use std::collections::BTreeSet;

use crate::grid::{Climb, CoverLevel, GridPos, Map, Surface, Terrain};
use crate::mission::{DeployDef, DoorDef, HookEvent, MissionDef, PropDef, ScriptHook, UnitPlacement, ZoneDef};
use crate::pathfinding::{self, ReachableSet};
use crate::rng::GameRng;
use crate::unit::Team;

/// Visual and structural style of a generated map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    Urban,
    Industrial,
    Wilds,
}

/// What the squad must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Objective {
    Eliminate,
    Extract,
    Hold,
    Recover,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Urban, Theme::Industrial, Theme::Wilds];
    pub fn name(self) -> &'static str {
        match self {
            Theme::Urban => "Urban",
            Theme::Industrial => "Industrial",
            Theme::Wilds => "Wilds",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }
}

impl Objective {
    pub const ALL: [Objective; 4] = [Objective::Eliminate, Objective::Extract, Objective::Hold, Objective::Recover];
    pub fn name(self) -> &'static str {
        match self {
            Objective::Eliminate => "Eliminate",
            Objective::Extract => "Extract",
            Objective::Hold => "Hold",
            Objective::Recover => "Recover",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Settings for [`generate`].
#[derive(Debug, Clone, PartialEq)]
pub struct GenParams {
    pub theme: Theme,
    pub objective: Objective,
    pub width: i32,
    pub height: i32,
    /// Enemy classes to use, most common first (e.g. `["sectoid", "drone"]`).
    pub enemy_classes: Vec<String>,
    /// Hide enemies outside the squad's sight.
    pub fog: bool,
    pub seed: u64,
}

impl GenParams {
    /// Sensible defaults for a theme and objective.
    pub fn new(theme: Theme, objective: Objective, seed: u64) -> Self {
        Self { theme, objective, width: 32, height: 28, enemy_classes: vec!["sectoid".into()], fog: theme != Theme::Wilds, seed }
    }

    /// A random theme and objective from `seed`.
    pub fn random(seed: u64) -> Self {
        let mut rng = GameRng::new(seed ^ 0x7E3A_11CE);
        let theme = Theme::ALL[rng.range_inclusive(0, 2) as usize];
        let objective = Objective::ALL[rng.range_inclusive(0, 3) as usize];
        Self::new(theme, objective, seed)
    }
}

/// Inclusive rectangle in grid coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Rect {
    fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    fn w(&self) -> i32 {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> i32 {
        self.y1 - self.y0 + 1
    }
    fn tiles(&self) -> impl Iterator<Item = GridPos> + '_ {
        (self.y0..=self.y1).flat_map(move |y| (self.x0..=self.x1).map(move |x| GridPos::new(x, y)))
    }
    fn is_edge(&self, p: GridPos) -> bool {
        p.x == self.x0 || p.x == self.x1 || p.y == self.y0 || p.y == self.y1
    }
    fn is_corner(&self, p: GridPos) -> bool {
        (p.x == self.x0 || p.x == self.x1) && (p.y == self.y0 || p.y == self.y1)
    }
    fn shrink(&self, n: i32) -> Self {
        Rect::new(self.x0 + n, self.y0 + n, self.x1 - n, self.y1 - n)
    }
}

/// Map being built plus everything placed on it.
struct Gen {
    p: GenParams,
    map: Map,
    rng: GameRng,
    /// Tiles nothing may be built on (spawn area, roads, objective spots).
    keep: BTreeSet<GridPos>,
    doors: Vec<DoorDef>,
    props: Vec<PropDef>,
    zones: Vec<ZoneDef>,
    units: Vec<UnitPlacement>,
    hooks: Vec<ScriptHook>,
    library: String,
    spawn: GridPos,
}

fn hook(on: HookEvent, team: Option<Team>, code: impl Into<String>) -> ScriptHook {
    ScriptHook { on, team, once: false, code: code.into() }
}

impl Gen {
    fn new(p: GenParams) -> Self {
        let (w, h) = (p.width.max(16), p.height.max(14));
        let rng = GameRng::new(p.seed);
        let spawn = GridPos::new(w / 2, 1);
        Self {
            map: Map::new(w, h),
            rng,
            keep: BTreeSet::new(),
            doors: Vec::new(),
            props: Vec::new(),
            zones: Vec::new(),
            units: Vec::new(),
            hooks: Vec::new(),
            library: String::new(),
            spawn,
            p: GenParams { width: w, height: h, ..p },
        }
    }

    fn w(&self) -> i32 {
        self.p.width
    }
    fn h(&self) -> i32 {
        self.p.height
    }
    fn roll(&mut self, pct: i32) -> bool {
        self.rng.roll_percent(pct)
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        self.rng.range_inclusive(lo, hi.max(lo))
    }
    fn inside(&self, p: GridPos) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < self.w() && p.y < self.h()
    }
    /// Open floor nothing is reserved on.
    fn free(&self, p: GridPos) -> bool {
        self.inside(p) && self.map.terrain(p) == Terrain::Floor && !self.keep.contains(&p) && !self.props.iter().any(|q| q.pos == p)
    }
    fn fill(&mut self, r: Rect, surface: Surface) {
        for t in r.tiles() {
            self.map.set_surface(t, surface);
        }
    }
    fn block(&mut self, p: GridPos, terrain: Terrain, surface: Surface) {
        if self.free(p) {
            self.map.set_terrain(p, terrain);
            self.map.set_surface(p, surface);
        }
    }
    /// Top-down y for scripts.
    fn td(&self, y: i32) -> i32 {
        self.h() - 1 - y
    }

    fn prop(&mut self, tag: String, kind: &str, p: GridPos, cover: CoverLevel, hp: Option<i32>, scripts: Vec<ScriptHook>) {
        if !self.free(p) {
            return;
        }
        let terrain = match cover {
            CoverLevel::Full => Terrain::HighCover,
            CoverLevel::Half => Terrain::LowCover,
            CoverLevel::None => Terrain::Floor,
        };
        self.map.set_terrain(p, terrain);
        self.props.push(PropDef { tag, kind: kind.into(), pos: p, cover, hp, model: None, hooks: scripts });
    }

    fn barrel(&mut self, p: GridPos) {
        let tag = format!("barrel_{}_{}", p.x, p.y);
        let boom = hook(HookEvent::Destroyed, None, "message(\"A barrel explodes!\"); explode(x, y, 1.6, 4);");
        self.prop(tag, "barrel", p, CoverLevel::Half, Some(2), vec![boom]);
    }

    fn crate_at(&mut self, p: GridPos) {
        let tag = format!("crate_{}_{}", p.x, p.y);
        self.prop(tag, "crate", p, CoverLevel::Half, Some(4), Vec::new());
    }

    // ------------------------------------------------------------------
    // Structures
    // ------------------------------------------------------------------

    /// A building: walls (with windows), a floor, and doors on sides that
    /// open onto free ground.
    fn building(&mut self, r: Rect, wall: Surface, floor: Surface) {
        if r.w() < 4 || r.h() < 4 || r.tiles().any(|t| !self.free(t)) {
            return;
        }
        let window_every = self.range(3, 4);
        for t in r.tiles().collect::<Vec<_>>() {
            if r.is_edge(t) {
                let along = if t.y == r.y0 || t.y == r.y1 { t.x - r.x0 } else { t.y - r.y0 };
                let window = !r.is_corner(t) && along % window_every == 0;
                let terrain = if window { Terrain::LowCover } else { Terrain::HighCover };
                self.map.set_terrain(t, terrain);
                self.map.set_surface(t, wall);
            } else {
                self.map.set_surface(t, floor);
            }
        }
        // Doors: middle-ish of up to two sides whose outside is open.
        let mut sides: Vec<(GridPos, GridPos)> = Vec::new();
        let mx = self.range(r.x0 + 1, r.x1 - 1);
        let my = self.range(r.y0 + 1, r.y1 - 1);
        sides.push((GridPos::new(mx, r.y0), GridPos::new(mx, r.y0 - 1)));
        sides.push((GridPos::new(mx, r.y1), GridPos::new(mx, r.y1 + 1)));
        sides.push((GridPos::new(r.x0, my), GridPos::new(r.x0 - 1, my)));
        sides.push((GridPos::new(r.x1, my), GridPos::new(r.x1 + 1, my)));
        let mut made = 0;
        for (door, outside) in sides {
            if made >= 2 || !self.inside(outside) || !self.map.is_walkable(outside) {
                continue;
            }
            if made == 1 && self.roll(40) {
                continue;
            }
            let tag = format!("door_{}_{}", door.x, door.y);
            self.map.set_terrain(door, Terrain::HighCover);
            self.doors.push(DoorDef { tag, tiles: vec![door], locked: false, hooks: Vec::new() });
            made += 1;
        }
        if made == 0 {
            // No open side: knock a doorway into the south wall.
            let p = GridPos::new(mx, r.y0);
            self.map.set_terrain(p, Terrain::Floor);
        }
        // Furniture.
        let inner = r.shrink(1);
        for _ in 0..self.range(1, 3) {
            let p = GridPos::new(self.range(inner.x0, inner.x1), self.range(inner.y0, inner.y1));
            let near_door = self.doors.iter().any(|d| d.tiles[0].chebyshev(p) <= 1);
            if !near_door && inner.w() > 2 && inner.h() > 2 {
                self.crate_at(p);
            }
        }
    }

    /// A flat roof `level` high with a parapet and a ladder on one side.
    fn rooftop(&mut self, r: Rect, level: i8, surface: Surface, rim: Surface) {
        if r.w() < 4 || r.h() < 4 || r.tiles().any(|t| !self.free(t)) {
            return;
        }
        // Ladder: outside the south (or north) edge, on open ground.
        let lx = self.range(r.x0 + 1, r.x1 - 1);
        let ladder = [(GridPos::new(lx, r.y0), GridPos::new(lx, r.y0 - 1)), (GridPos::new(lx, r.y1), GridPos::new(lx, r.y1 + 1))]
            .into_iter()
            .find(|(_, out)| self.inside(*out) && self.map.is_walkable(*out));
        let Some((gap, outside)) = ladder else { return };
        for t in r.tiles().collect::<Vec<_>>() {
            self.map.set_elevation(t, level);
            self.map.set_surface(t, surface);
            if r.is_edge(t) && t != gap && self.roll(70) {
                self.map.set_terrain(t, Terrain::LowCover);
                self.map.set_surface(t, rim);
            }
        }
        self.map.set_ladder(outside, true);
        self.keep.insert(outside);
        self.keep.insert(gap);
    }

    /// A line of cover with gaps.
    fn wall_line(&mut self, from: GridPos, len: i32, horizontal: bool, terrain: Terrain, surface: Surface, gap_pct: i32) {
        for i in 0..len {
            let p = if horizontal { GridPos::new(from.x + i, from.y) } else { GridPos::new(from.x, from.y + i) };
            if !self.roll(gap_pct) {
                self.block(p, terrain, surface);
            }
        }
    }

    fn random_tile(&mut self, area: Rect) -> GridPos {
        GridPos::new(self.range(area.x0, area.x1), self.range(area.y0, area.y1))
    }

    // ------------------------------------------------------------------
    // Themes
    // ------------------------------------------------------------------

    fn urban(&mut self) {
        let (w, h) = (self.w(), self.h());
        self.fill(Rect::new(0, 0, w - 1, h - 1), Surface::Concrete);
        // Streets: two east-west avenues and one north-south street.
        let rows = [h / 3, (2 * h) / 3 + 1];
        let col = w / 2 + self.range(-3, 3);
        for &ry in &rows {
            for x in 0..w {
                for dy in -1..=1 {
                    let p = GridPos::new(x, ry + dy);
                    self.map.set_surface(p, if dy == 0 && x % 3 != 0 { Surface::Lines } else { Surface::Asphalt });
                    self.keep.insert(p);
                }
            }
        }
        for y in 0..h {
            for dx in -1..=1 {
                let p = GridPos::new(col + dx, y);
                self.map.set_surface(p, if dx == 0 && y % 3 != 0 && !rows.iter().any(|r| (y - r).abs() <= 1) { Surface::Lines } else { Surface::Asphalt });
                self.keep.insert(p);
            }
        }
        // Blocks between the streets.
        let ys = [(0, rows[0] - 2), (rows[0] + 2, rows[1] - 2), (rows[1] + 2, h - 1)];
        let xs = [(0, col - 2), (col + 2, w - 1)];
        for &(y0, y1) in &ys {
            for &(x0, x1) in &xs {
                let block = Rect::new(x0 + 1, y0 + 1, x1 - 1, y1 - 1);
                if block.w() < 4 || block.h() < 4 {
                    continue;
                }
                let roll = self.range(0, 99);
                if roll < 55 {
                    // One or two buildings.
                    if block.w() >= 11 && self.roll(50) {
                        let split = block.x0 + block.w() / 2;
                        self.building(Rect::new(block.x0, block.y0, split - 1, block.y1), Surface::Brick, Surface::Interior);
                        self.building(Rect::new(split + 1, block.y0, block.x1, block.y1), Surface::Brick, Surface::Interior);
                    } else {
                        let b = Rect::new(block.x0, block.y0, block.x1.min(block.x0 + 9), block.y1.min(block.y0 + 7));
                        self.building(b, Surface::Brick, Surface::Interior);
                    }
                } else if roll < 80 {
                    let r = Rect::new(block.x0, block.y0, block.x1.min(block.x0 + 6), block.y1.min(block.y0 + 5));
                    self.rooftop(r, 2, Surface::Roof, Surface::Concrete);
                } else {
                    // Plaza: grass, trees, hedges.
                    self.fill(block, Surface::Grass);
                    for _ in 0..self.range(2, 4) {
                        let t = self.random_tile(block);
                        self.block(t, Terrain::HighCover, Surface::Tree);
                    }
                    let start = self.random_tile(block);
                    let horizontal = self.roll(50);
                    self.wall_line(start, 3, horizontal, Terrain::LowCover, Surface::Hedge, 0);
                }
            }
        }
        // Street clutter: cars, barrels, crates (on asphalt, not blocking whole streets).
        let street: Vec<GridPos> = self.keep.iter().copied().filter(|p| p.y > 4).collect();
        for _ in 0..(w * h / 70) {
            let p = street[self.range(0, street.len() as i32 - 1) as usize];
            let kind = self.range(0, 9);
            self.keep.remove(&p);
            match kind {
                0..=4 => {
                    let tag = format!("car_{}_{}", p.x, p.y);
                    self.prop(tag, "car", p, CoverLevel::Half, Some(8), Vec::new());
                }
                5..=6 => self.barrel(p),
                _ => self.crate_at(p),
            }
            self.keep.insert(p);
        }
    }

    fn industrial(&mut self) {
        let (w, h) = (self.w(), self.h());
        self.fill(Rect::new(0, 0, w - 1, h - 1), Surface::Asphalt);
        // Concrete pads.
        for _ in 0..4 {
            let x = self.range(0, w - 8);
            let y = self.range(4, h - 6);
            let (pw, ph) = (self.range(5, 9), self.range(4, 6));
            self.fill(Rect::new(x, y, (x + pw).min(w - 1), (y + ph).min(h - 1)), Surface::Concrete);
        }
        // Warehouses.
        for i in 0..2 {
            let bw = self.range(8, 11);
            let bh = self.range(6, 8);
            let x = if i == 0 { self.range(1, w / 2 - bw) } else { self.range(w / 2 + 1, w - bw - 2) };
            let y = self.range(h / 2, h - bh - 2);
            self.building(Rect::new(x, y, x + bw - 1, y + bh - 1), Surface::Container, Surface::Metal);
        }
        // Container rows: some stacked high with a ladder.
        for _ in 0..(w * h / 60) {
            let horizontal = self.roll(50);
            let (cw, ch) = if horizontal { (self.range(2, 4), 2) } else { (2, self.range(2, 4)) };
            let x = self.range(1, w - cw - 2);
            let y = self.range(5, h - ch - 2);
            let r = Rect::new(x, y, x + cw - 1, y + ch - 1);
            if !r.tiles().all(|t| self.free(t)) {
                continue;
            }
            if self.roll(25) && cw >= 2 && ch >= 2 {
                let big = Rect::new(x, y, (x + 3).min(w - 2), (y + 3).min(h - 2));
                self.rooftop(big, 2, Surface::Metal, Surface::Container);
            } else {
                for t in r.tiles().collect::<Vec<_>>() {
                    self.block(t, Terrain::HighCover, Surface::Container);
                }
            }
        }
        // Fences and barrel clusters.
        for _ in 0..3 {
            let start = GridPos::new(self.range(1, w - 8), self.range(6, h - 3));
            let horizontal = self.roll(60);
            let len = self.range(4, 7);
            self.wall_line(start, len, horizontal, Terrain::LowCover, Surface::Fence, 25);
        }
        for _ in 0..3 {
            let c = GridPos::new(self.range(2, w - 3), self.range(6, h - 3));
            for d in [(0, 0), (1, 0), (0, 1)] {
                if self.roll(70) {
                    self.barrel(GridPos::new(c.x + d.0, c.y + d.1));
                }
            }
        }
        for _ in 0..(w * h / 90) {
            let p = GridPos::new(self.range(1, w - 2), self.range(5, h - 2));
            self.crate_at(p);
        }
    }

    fn wilds(&mut self) {
        let (w, h) = (self.w(), self.h());
        self.fill(Rect::new(0, 0, w - 1, h - 1), Surface::Grass);
        // Hills: a level-1 blob with a level-2 crown (climbable steps).
        for _ in 0..self.range(2, 3) {
            let c = GridPos::new(self.range(3, w - 4), self.range(h / 3, h - 4));
            let r = self.range(3, 5) as f32;
            for p in Rect::new(c.x - 6, c.y - 6, c.x + 6, c.y + 6).tiles().collect::<Vec<_>>() {
                if !self.inside(p) || self.keep.contains(&p) {
                    continue;
                }
                let d = p.distance(c);
                if d <= r {
                    self.map.set_elevation(p, if d <= r - 2.0 { 2 } else { 1 }.max(self.map.elevation(p)));
                    if d <= r - 2.0 {
                        self.map.set_surface(p, Surface::Dirt);
                    }
                }
            }
        }
        // River with bridges.
        let ry = (h as f32 * 0.55) as i32;
        let bridges: Vec<i32> = vec![self.range(3, w / 3), self.range(w / 3 + 3, 2 * w / 3), self.range(2 * w / 3 + 2, w - 4)];
        let mut y = ry;
        for x in 0..w {
            if self.roll(20) {
                y = (y + self.range(-1, 1)).clamp(ry - 2, ry + 2);
            }
            let on_bridge = bridges.iter().any(|b| (x - b).abs() <= 1);
            for dy in 0..2 {
                let p = GridPos::new(x, y + dy);
                self.map.set_elevation(p, 0);
                if on_bridge {
                    self.map.set_surface(p, Surface::Metal);
                    self.keep.insert(p);
                } else {
                    self.map.set_terrain(p, Terrain::Void);
                    self.map.set_surface(p, Surface::Water);
                }
            }
        }
        // Dirt track from the south to the north over the middle bridge.
        let mut x = self.spawn.x;
        for yy in 0..h {
            let target = bridges[1];
            if yy < ry && x != target {
                x += (target - x).signum();
            } else if self.roll(25) {
                x = (x + self.range(-1, 1)).clamp(1, w - 2);
            }
            for dx in 0..2 {
                let p = GridPos::new(x + dx, yy);
                if self.map.terrain(p) == Terrain::Floor && self.map.surface(p) != Surface::Metal {
                    self.map.set_surface(p, Surface::Dirt);
                }
            }
        }
        // A cabin on flat ground north of the river.
        for _ in 0..60 {
            let x = self.range(2, w - 9);
            let y = self.range(ry + 3, h - 6);
            let r = Rect::new(x, y, x + 5, y + 4);
            if r.tiles().all(|t| self.map.elevation(t) == 0 && self.free(t)) {
                self.building(r, Surface::Brick, Surface::Interior);
                break;
            }
        }
        // Woods: tree clusters; rocks; hedgerows.
        for _ in 0..(w * h / 55) {
            let c = GridPos::new(self.range(0, w - 1), self.range(4, h - 1));
            for _ in 0..self.range(2, 5) {
                let p = GridPos::new(c.x + self.range(-2, 2), c.y + self.range(-2, 2));
                self.block(p, Terrain::HighCover, Surface::Tree);
            }
        }
        for _ in 0..(w * h / 50) {
            let p = GridPos::new(self.range(0, w - 1), self.range(4, h - 1));
            let big = self.roll(30);
            self.block(p, if big { Terrain::HighCover } else { Terrain::LowCover }, Surface::Rock);
        }
        for _ in 0..3 {
            let start = GridPos::new(self.range(1, w - 8), self.range(5, h - 3));
            let horizontal = self.roll(50);
            let len = self.range(4, 7);
            self.wall_line(start, len, horizontal, Terrain::LowCover, Surface::Hedge, 20);
        }
    }

    // ------------------------------------------------------------------
    // Playability and placement
    // ------------------------------------------------------------------

    /// Tiles reachable from the spawn (doors open, normal climbing).
    fn reachable(&self) -> ReachableSet {
        let mut map = self.map.clone();
        for d in &self.doors {
            for t in &d.tiles {
                map.set_terrain(*t, Terrain::Floor);
            }
        }
        pathfinding::reachable(&map, self.spawn, u32::MAX / 4, Climb::default(), |_| true)
    }

    /// Reachable, free tiles in `area`, sorted (deterministic).
    fn spots(&self, reach: &ReachableSet, area: Rect, taken: &BTreeSet<GridPos>) -> Vec<GridPos> {
        area.tiles()
            .filter(|p| self.inside(*p) && reach.contains(*p) && self.free(*p) && !taken.contains(p) && *p != self.spawn)
            .collect()
    }

    fn pick(&mut self, list: &[GridPos]) -> Option<GridPos> {
        (!list.is_empty()).then(|| list[self.range(0, list.len() as i32 - 1) as usize])
    }

    /// Clears and reserves the deployment area.
    fn deploy_area(&mut self) {
        let s = self.spawn;
        for p in Rect::new(s.x - 3, 0, s.x + 3, 3).tiles().collect::<Vec<_>>() {
            if self.inside(p) {
                self.map.set_terrain(p, Terrain::Floor);
                self.map.set_elevation(p, 0);
                self.map.set_ladder(p, false);
                self.keep.insert(p);
            }
        }
    }

    /// Enemy pods and a patrol, on reachable tiles away from the squad.
    fn enemies(&mut self, reach: &ReachableSet, taken: &mut BTreeSet<GridPos>) {
        let (w, h) = (self.w(), self.h());
        let classes = if self.p.enemy_classes.is_empty() { vec!["sectoid".to_string()] } else { self.p.enemy_classes.clone() };
        let pods = ((w * h) / 280).clamp(2, 4);
        let far: Vec<GridPos> = self
            .spots(reach, Rect::new(0, h / 3, w - 1, h - 1), taken)
            .into_iter()
            .filter(|p| p.distance(self.spawn) >= 10.0)
            .collect();
        for pod in 0..pods {
            // Prefer spots next to cover (or indoors).
            let mut best: Option<(GridPos, i32)> = None;
            for _ in 0..12 {
                let Some(c) = self.pick(&far) else { break };
                if taken.iter().any(|t| t.distance(c) < 6.0) {
                    continue;
                }
                let cover = crate::cover::cover_around(&self.map, c).iter().filter(|l| **l != CoverLevel::None).count() as i32;
                let indoor = (self.map.surface(c) == Surface::Interior || self.map.surface(c) == Surface::Metal) as i32;
                let score = cover * 2 + indoor * 3 + self.range(0, 2);
                if best.is_none_or(|(_, s)| score > s) {
                    best = Some((c, score));
                }
            }
            let Some((center, _)) = best else { continue };
            let size = self.range(2, 3);
            let near = self.spots(reach, Rect::new(center.x - 2, center.y - 2, center.x + 2, center.y + 2), taken);
            let mut placed = 0;
            for p in std::iter::once(center).chain(near) {
                if placed >= size || taken.contains(&p) {
                    continue;
                }
                taken.insert(p);
                let class = if placed == 0 || classes.len() == 1 || self.roll(65) { classes[0].clone() } else { classes[(1 + pod as usize) % classes.len()].clone() };
                self.units.push(UnitPlacement {
                    team: Team::Enemy,
                    class: class.clone(),
                    name: format!("{} {}-{}", capitalise(&class), pod + 1, placed + 1),
                    pos: p,
                    tag: None,
                    hooks: Vec::new(),
                    dormant: true,
                    pod: Some(format!("pod{pod}")),
                    patrol: Vec::new(),
                    loadout: None,
                    gear: Default::default(),
                });
                placed += 1;
            }
        }
        // A patrol walking the middle of the map.
        let mid = self.spots(reach, Rect::new(1, h / 3, w - 2, (2 * h) / 3), taken);
        if let (Some(a), Some(b), Some(c)) = (self.pick(&mid), self.pick(&mid), self.pick(&mid))
            && !taken.contains(&a)
        {
            taken.insert(a);
            self.units.push(UnitPlacement {
                team: Team::Enemy,
                class: classes[0].clone(),
                name: format!("{} Patrol", capitalise(&classes[0])),
                pos: a,
                tag: None,
                hooks: Vec::new(),
                dormant: false,
                pod: None,
                patrol: vec![a, b, c],
                loadout: None,
                gear: Default::default(),
            });
        }
    }

    /// A marked zone rectangle (grid coords) and the script call for it.
    fn zone(&mut self, tag: &str, r: Rect, marker: &str, label: &str, hidden: bool, hooks: Vec<ScriptHook>) {
        for t in r.tiles() {
            self.keep.insert(t);
        }
        self.zones.push(ZoneDef {
            tag: tag.into(),
            min: GridPos::new(r.x0, r.y0),
            max: GridPos::new(r.x1, r.y1),
            hooks,
            marker: Some(marker.into()),
            label: Some(label.into()),
            hidden,
        });
    }

    /// `count_in_area(x, y, w, h, team)` for a grid rectangle (scripts use top-down y).
    fn count_expr(&self, r: Rect, team: &str) -> String {
        format!("count_in_area({}, {}, {}, {}, \"{team}\")", r.x0, self.td(r.y1), r.w(), r.h())
    }

    /// Picks a reachable 3x2 (or smaller) area within `band`.
    fn area_in(&mut self, reach: &ReachableSet, band: Rect, taken: &BTreeSet<GridPos>) -> Option<Rect> {
        let spots = self.spots(reach, band, taken);
        for (cw, ch) in [(3, 2), (2, 2), (1, 1)] {
            let fits: Vec<GridPos> = spots
                .iter()
                .copied()
                .filter(|p| Rect::new(p.x, p.y, p.x + cw - 1, p.y + ch - 1).tiles().all(|t| spots.contains(&t)))
                .collect();
            if let Some(p) = self.pick(&fits) {
                return Some(Rect::new(p.x, p.y, p.x + cw - 1, p.y + ch - 1));
            }
        }
        None
    }

    /// Scripts, zones and props for the objective. Returns `eliminate_to_win`.
    fn objective(&mut self, reach: &ReachableSet, taken: &mut BTreeSet<GridPos>) -> (bool, String) {
        let (w, h) = (self.w(), self.h());
        let north = Rect::new(1, (h * 3) / 4, w - 2, h - 1);
        match self.p.objective {
            Objective::Eliminate => {
                self.hooks.push(hook(HookEvent::Start, None, "objective(\"Eliminate all hostiles (\" + alive_count(\"enemy\") + \")\");"));
                self.hooks.push(hook(
                    HookEvent::UnitKilled,
                    None,
                    "let left = alive_count(\"enemy\"); if unit_team(unit) == \"enemy\" && left > 0 { objective(\"Eliminate all hostiles (\" + left + \" left)\"); }",
                ));
                (true, "Find and eliminate every hostile in the area.".into())
            }
            Objective::Extract => {
                let Some(r) = self.area_in(reach, north, taken) else { return (true, "Eliminate all hostiles.".into()) };
                let inside = self.count_expr(r, "player");
                let code = format!(
                    "let inside = {inside}; let squad = alive_count(\"player\"); \
                     if inside == squad {{ message(\"Squad extracted!\"); win(); }} \
                     else {{ message(inside + \" of \" + squad + \" soldiers at the evac point.\"); }}"
                );
                self.zone("evac", r, "evac", "EVAC", false, vec![hook(HookEvent::Enter, Some(Team::Player), code)]);
                self.hooks.push(hook(HookEvent::Start, None, "objective(\"Get the whole squad to the EVAC point (north) - or clear the area\");"));
                (true, "Push through to the evac point in the north with the whole squad.".into())
            }
            Objective::Recover => {
                let mid = Rect::new(1, h / 2, w - 2, h - 2);
                let Some(data) = self.area_in(reach, mid, taken) else { return (true, "Eliminate all hostiles.".into()) };
                let data = Rect::new(data.x0, data.y0, data.x0, data.y0);
                // Evac: next to where the squad came in.
                let s = self.spawn;
                let evac = Rect::new(s.x - 1, 0, s.x + 1, 1);
                let take = "if flag(\"data\") == 0 { set_flag(\"data\", 1); message(unit_name(unit) + \" has the data. Back to the evac point!\"); \
                            objective(\"Bring the data back to the EVAC point (south)\"); show_zone(\"data\", false); show_zone(\"evac\", true); }";
                self.zone("data", data, "objective", "DATA", false, vec![hook(HookEvent::Enter, Some(Team::Player), take)]);
                // A terminal beside the data tile, if there's room.
                if let Some(t) = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().map(|(dx, dy)| GridPos::new(data.x0 + dx, data.y0 + dy)).find(|p| self.free(*p)) {
                    self.prop("terminal".into(), "console", t, CoverLevel::Half, None, Vec::new());
                }
                let leave = "if flag(\"data\") == 1 { message(\"Data recovered. Mission complete!\"); win(); }";
                self.zone("evac", evac, "evac", "EVAC", true, vec![hook(HookEvent::Enter, Some(Team::Player), leave)]);
                self.hooks.push(hook(HookEvent::Start, None, "set_flag(\"data\", 0); objective(\"Recover the DATA, then return to the evac point\");"));
                (false, "Recover the data from the marked terminal and bring it back to the evac point.".into())
            }
            Objective::Hold => {
                let centre = Rect::new(w / 2 - 6, h / 2 - 2, w / 2 + 6, h / 2 + 4);
                let spots = self.spots(reach, centre, taken);
                let Some(b) = self.pick(&spots) else { return (true, "Eliminate all hostiles.".into()) };
                taken.insert(b);
                self.prop("beacon".into(), "beacon", b, CoverLevel::Half, None, Vec::new());
                // Sandbags around it.
                for (dx, dy) in [(-2, 1), (-2, 0), (2, 0), (2, -1), (0, 2), (1, 2), (-1, -2)] {
                    let p = GridPos::new(b.x + dx, b.y + dy);
                    if self.roll(70) {
                        self.block(p, Terrain::LowCover, Surface::Sandbags);
                    }
                }
                let ring = Rect::new(b.x - 2, b.y - 2, b.x + 2, b.y + 2);
                self.zone("beacon_area", ring, "objective", "BEACON", false, Vec::new());
                let count = self.count_expr(ring, "enemy");
                let class = self.p.enemy_classes.first().cloned().unwrap_or_else(|| "sectoid".into());
                let (ex, ey) = (b.x, self.td(h - 1));
                self.hooks.push(hook(
                    HookEvent::Start,
                    None,
                    format!("set_flag(\"integrity\", 8); ai_goal(\"enemy\", {}, {}); objective(\"Hold the BEACON for 6 turns (integrity 8)\");", b.x, self.td(b.y)),
                ));
                self.hooks.push(hook(
                    HookEvent::TurnStart,
                    Some(Team::Enemy),
                    format!(
                        "let n = {count}; if n > 0 {{ set_flag(\"integrity\", flag(\"integrity\") - n * 2); message(\"The beacon is under attack!\"); \
                         if flag(\"integrity\") <= 0 {{ message(\"The beacon is lost.\"); lose(); }} }} \
                         if round % 2 == 0 && round <= 6 {{ message(\"Enemy reinforcements!\"); \
                         spawn_unit(\"enemy\", \"{class}\", \"Raider\", {ex}, {ey}); spawn_unit(\"enemy\", \"{class}\", \"Raider\", {}, {ey}); }}",
                        (ex + 3).min(w - 1)
                    ),
                ));
                self.hooks.push(hook(
                    HookEvent::TurnStart,
                    Some(Team::Player),
                    "if round >= 7 { message(\"The beacon held!\"); win(); } else { objective(\"Hold the BEACON: \" + (7 - round) + \" turns left (integrity \" + flag(\"integrity\") + \")\"); }",
                ));
                (false, "Hold the beacon in the middle of the map for six turns against waves of enemies.".into())
            }
        }
    }

    fn finish(mut self) -> MissionDef {
        // Fill the default surface by theme.
        let base = match self.p.theme {
            Theme::Urban => Surface::Concrete,
            Theme::Industrial => Surface::Asphalt,
            Theme::Wilds => Surface::Grass,
        };
        for p in self.map.positions().collect::<Vec<_>>() {
            if self.map.surface(p) == Surface::Default {
                self.map.set_surface(p, base);
            }
        }
        self.deploy_area();
        let reach = self.reachable();
        let mut taken: BTreeSet<GridPos> = BTreeSet::new();
        let (eliminate, goal) = self.objective(&reach, &mut taken);
        self.enemies(&reach, &mut taken);
        let name = format!("{} {}", self.p.theme.name(), self.p.objective.name());
        let mut keep_doors = Vec::new();
        for d in std::mem::take(&mut self.doors) {
            // A door whose tile ended up reserved (zone) is dropped.
            if !d.tiles.iter().any(|t| taken.contains(t)) {
                keep_doors.push(d);
            }
        }
        MissionDef {
            id: "generated".into(),
            name,
            description: format!("{goal} (generated, seed {:#x})", self.p.seed),
            map: self.map,
            units: self.units,
            deploys: vec![DeployDef { team: Team::Player, squad: "player".into(), pos: self.spawn }],
            props: self.props,
            zones: self.zones,
            doors: keep_doors,
            fog: self.p.fog,
            concealment: false,
            detection: 5.0,
            hooks: self.hooks,
            library: self.library,
            seed: Some(self.p.seed),
            eliminate_to_win: eliminate,
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Generates a mission (see the module docs).
pub fn generate(params: GenParams) -> MissionDef {
    let mut g = Gen::new(params);
    // The deployment strip stays clear while the theme builds around it.
    let s = g.spawn;
    for p in Rect::new(s.x - 3, 0, s.x + 3, 3).tiles() {
        g.keep.insert(p);
    }
    match g.p.theme {
        Theme::Urban => g.urban(),
        Theme::Industrial => g.industrial(),
        Theme::Wilds => g.wilds(),
    }
    g.finish()
}

/// Enemy classes in `rules` (the non-soldier, non-dummy classes), most
/// common first: `sectoid` if present, then the rest alphabetically.
pub fn enemy_classes(rules: &crate::content::Ruleset) -> Vec<String> {
    let mut list: Vec<String> = rules
        .classes()
        .filter(|c| c.model != "soldier" && c.id != "training_dummy")
        .map(|c| c.id.clone())
        .collect();
    list.sort_by_key(|c| (c != "sectoid", c.clone()));
    list
}

#[cfg(all(test, feature = "scripting"))]
mod tests {
    use super::*;
    use crate::ai::{self, AiConfig};
    use crate::battle::Battle;
    use crate::content::Ruleset;
    use crate::event::GameEvent;
    use crate::modules::GrenadierModule;

    fn rules() -> Ruleset {
        let mut r = Ruleset::standard();
        r.add_module(&GrenadierModule);
        r
    }

    #[test]
    fn deterministic() {
        let p = GenParams::new(Theme::Urban, Objective::Extract, 42);
        assert_eq!(generate(p.clone()), generate(p));
    }

    /// Every theme × objective over several seeds builds a valid, playable
    /// mission: no warnings, scripts all run, enemies placed, objectives
    /// reachable, and AI-vs-AI play makes progress without script errors.
    #[test]
    fn every_combination_is_playable() {
        let rules = rules();
        for theme in Theme::ALL {
            for objective in Objective::ALL {
                for seed in 0..4u64 {
                    let mut p = GenParams::new(theme, objective, seed * 7919 + 3);
                    p.enemy_classes = enemy_classes(&rules);
                    let def = generate(p);
                    let tag = format!("{theme:?}/{objective:?}/{seed}");
                    let (mut b, warnings) = Battle::from_mission(rules.clone(), &def, 1);
                    assert!(warnings.is_empty(), "{tag}: {warnings:?}");
                    assert_eq!(b.living(Team::Player).count(), 4, "{tag}: squad deployed");
                    assert!(b.living(Team::Enemy).count() >= 4, "{tag}: enemies placed ({})", b.living(Team::Enemy).count());
                    assert!(b.dry_run_scripts().is_empty(), "{tag}: {:?}", b.dry_run_scripts());
                    // Marked zones are reachable from the squad.
                    let start = b.living(Team::Player).next().unwrap().pos;
                    let mut open = b.map().clone();
                    for pr in b.props().iter().filter(|p| p.door.is_some()) {
                        open.set_terrain(pr.pos, Terrain::Floor);
                    }
                    let reach = pathfinding::reachable(&open, start, u32::MAX / 4, Climb::default(), |_| true);
                    for z in b.zones() {
                        assert!(
                            (z.min.y..=z.max.y).any(|y| (z.min.x..=z.max.x).any(|x| reach.contains(GridPos::new(x, y)) || GridPos::new(x, y) == start)),
                            "{tag}: zone {} unreachable",
                            z.tag
                        );
                    }
                    let mut events = b.begin();
                    for _ in 0..1500 {
                        if b.is_over() || b.round() > 12 {
                            break;
                        }
                        let a = ai::next_team_action(&b, &AiConfig::default());
                        events.extend(b.apply(a.clone()).unwrap_or_else(|e| panic!("{tag}: illegal {a:?}: {e}")));
                    }
                    let errors: Vec<_> = events.iter().filter(|e| matches!(e, GameEvent::ScriptError { .. })).collect();
                    assert!(errors.is_empty(), "{tag}: {errors:?}");
                    assert!(b.round() > 1 || b.is_over(), "{tag}: no progress");
                }
            }
        }
    }

    #[test]
    fn themes_look_different() {
        let count = |def: &MissionDef, s: Surface| def.map.positions().filter(|p| def.map.surface(*p) == s).count();
        let urban = generate(GenParams::new(Theme::Urban, Objective::Eliminate, 5));
        let yard = generate(GenParams::new(Theme::Industrial, Objective::Eliminate, 5));
        let wild = generate(GenParams::new(Theme::Wilds, Objective::Eliminate, 5));
        assert!(count(&urban, Surface::Brick) > 10 && count(&urban, Surface::Asphalt) > 20);
        assert!(count(&yard, Surface::Container) > 10);
        assert!(count(&wild, Surface::Tree) > 5 && count(&wild, Surface::Water) > 10);
        assert!(wild.map.max_elevation() >= 2, "hills");
        assert!(!urban.doors.is_empty(), "buildings have doors");
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// Text picture of a generated map (authoring aid):
    /// `#` wall, `=` low cover, `~` water, `^` raised, `D` door, `p` prop,
    /// `E` enemy, `S` spawn, `o` zone, `,` grass, `:` asphalt, `.` other.
    pub fn picture(def: &MissionDef) -> String {
        let m = &def.map;
        let mut out = String::new();
        for y in (0..m.height()).rev() {
            for x in 0..m.width() {
                let p = GridPos::new(x, y);
                let c = if def.units.iter().any(|u| u.pos == p) {
                    'E'
                } else if def.deploys.iter().any(|d| d.pos == p) {
                    'S'
                } else if def.doors.iter().any(|d| d.tiles.contains(&p)) {
                    'D'
                } else if def.props.iter().any(|q| q.pos == p) {
                    'p'
                } else if def.zones.iter().any(|z| z.contains(p)) {
                    'o'
                } else {
                    match (m.terrain(p), m.surface(p)) {
                        (Terrain::Void, _) => '~',
                        (Terrain::HighCover, Surface::Tree) => 'T',
                        (Terrain::HighCover, _) => '#',
                        (Terrain::LowCover, _) => '=',
                        _ if m.is_ladder(p) => 'H',
                        _ if m.elevation(p) > 0 => '^',
                        (_, Surface::Grass) => ',',
                        (_, Surface::Asphalt | Surface::Lines) => ':',
                        _ => '.',
                    }
                };
                out.push(c);
            }
            out.push('\n');
        }
        out
    }

    #[test]
    #[ignore]
    fn print_examples() {
        for theme in Theme::ALL {
            let def = generate(GenParams::new(theme, Objective::Extract, 11));
            eprintln!("{} ({} enemies, {} doors, {} props)\n{}", def.name, def.units.len(), def.doors.len(), def.props.len(), picture(&def));
        }
    }
}

//! Conversion between grid coordinates and Bevy world space.
//!
//! Convention: one tile = one world unit, `grid.x -> +X`, `grid.y -> -Z`
//! (so "north" is away from a default camera), `Y` is up, and tile `(0, 0)` is
//! centred on the origin.

use bevy::prelude::*;
use tactics_core::grid::GridPos;

/// Size of a tile in world units.
pub const TILE_SIZE: f32 = 1.0;

/// World-space centre of a tile at ground level.
pub fn grid_to_world(pos: GridPos) -> Vec3 {
    Vec3::new(pos.x as f32 * TILE_SIZE, 0.0, -(pos.y as f32) * TILE_SIZE)
}

/// The tile containing a world-space point (height ignored).
pub fn world_to_grid(world: Vec3) -> GridPos {
    GridPos::new(
        (world.x / TILE_SIZE).round() as i32,
        (-world.z / TILE_SIZE).round() as i32,
    )
}

/// World height of one elevation level.
pub const LEVEL_HEIGHT: f32 = 0.5;

/// World-space centre of the **top** of a tile (its ground at its elevation).
/// Use this for anything standing on the map: units, overlays, effects.
pub fn tile_top(map: &tactics_core::grid::Map, pos: GridPos) -> Vec3 {
    grid_to_world(pos) + Vec3::Y * (map.elevation(pos) as f32 * LEVEL_HEIGHT)
}

/// The tile a ray (from the camera through the pointer) hits, taking terrain
/// height into account: tests the ray against each elevation level from the
/// top down and accepts the first tile whose own height matches.
pub fn pick_tile(map: &tactics_core::grid::Map, ray: Ray3d) -> Option<GridPos> {
    for level in (0..=map.max_elevation().max(0)).rev() {
        let y = level as f32 * LEVEL_HEIGHT;
        let Some(point) = ray.plane_intersection_point(Vec3::Y * y, InfinitePlane3d::new(Vec3::Y)) else { continue };
        let pos = world_to_grid(point);
        if map.in_bounds(pos) && map.elevation(pos) as i32 == level as i32 {
            return Some(pos);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for x in -3..10 {
            for y in -3..10 {
                let p = GridPos::new(x, y);
                assert_eq!(world_to_grid(grid_to_world(p)), p);
            }
        }
    }
}

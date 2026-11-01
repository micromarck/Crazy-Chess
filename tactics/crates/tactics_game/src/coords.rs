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

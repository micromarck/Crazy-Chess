//! Camera cutaway: tall walls between the camera and what the player is
//! looking at (the camera focus, the selected soldier, the hovered tile)
//! sink smoothly to a stub, then rise again once the view moves on. Keeps
//! soldiers in cramped interiors visible from any angle.
//!
//! The board tags tall blocks with [`Occluder`] (walls, closed doors,
//! full-cover props); nothing else needs to know about it.

use bevy::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::tile_top;
use crate::state::{BattleState, Selection};

pub struct CutawayPlugin;

impl Plugin for CutawayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, cut_walls.in_set(FrameSet::Present));
    }
}

/// A tall board piece that may be cut down to keep the view clear.
#[derive(Component, Debug, Clone, Copy)]
pub struct Occluder {
    /// Centre of its base.
    pub bottom: Vec3,
    pub height: f32,
    /// Current height factor (1 = full, `CUT` = cut down).
    pub factor: f32,
}

impl Occluder {
    pub fn new(bottom: Vec3, height: f32) -> Self {
        Self { bottom, height, factor: 1.0 }
    }
}

/// Height factor of a cut wall.
pub const CUT: f32 = 0.15;
/// How close (tiles, sideways) to the sight line a wall must be to be cut.
const LANE: f32 = 1.15;
/// Walls further than this from the point of interest stay up.
const REACH: f32 = 7.0;

/// Does a wall at `wall` block the view from `camera` to `point` (top view)?
pub fn blocks_view(camera: Vec3, point: Vec3, wall: Vec3) -> bool {
    let to_cam = Vec2::new(camera.x - point.x, camera.z - point.z);
    let len = to_cam.length();
    if len < 1e-3 {
        return false;
    }
    let dir = to_cam / len;
    let rel = Vec2::new(wall.x - point.x, wall.z - point.z);
    let along = rel.dot(dir);
    let side = (rel - dir * along).length();
    along > 0.35 && along < len.min(REACH) && side < LANE
}

fn cut_walls(
    time: Res<Time<Real>>,
    state: Res<BattleState>,
    selection: Res<Selection>,
    camera: Single<&Transform, With<CameraRig>>,
    rig: Single<&CameraRig>,
    mut walls: Query<(&mut Occluder, &mut Transform), Without<CameraRig>>,
) {
    let cam = camera.translation;
    let map = state.battle.map();
    let mut points = vec![rig.focus];
    if let Some(u) = selection.unit.and_then(|id| state.battle.unit(id)) {
        points.push(tile_top(map, u.pos));
    }
    // The hovered tile only counts if it matters (somewhere you could move,
    // or a unit), so sweeping the mouse across the map doesn't bob walls.
    if let Some(h) = selection.hovered.filter(|h| {
        selection.range.as_ref().is_some_and(|r| r.contains(*h)) || state.battle.unit_at(*h).is_some()
    }) {
        points.push(tile_top(map, h));
    }
    let k = 1.0 - (-10.0 * time.delta_secs()).exp();
    for (mut wall, mut tr) in &mut walls {
        let blocking = points.iter().any(|p| blocks_view(cam, *p, wall.bottom));
        let goal = if blocking { CUT } else { 1.0 };
        if (wall.factor - goal).abs() < 1e-3 {
            continue;
        }
        wall.factor += (goal - wall.factor) * k;
        if (wall.factor - goal).abs() < 0.01 {
            wall.factor = goal;
        }
        tr.scale.y = wall.factor;
        tr.translation.y = wall.bottom.y + wall.height * wall.factor / 2.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_walls_on_the_camera_side_and_near_the_line_are_cut() {
        let point = Vec3::ZERO;
        let camera = Vec3::new(0.0, 12.0, 15.0);
        assert!(blocks_view(camera, point, Vec3::new(0.0, 0.0, 2.0)), "right in front, camera side");
        assert!(!blocks_view(camera, point, Vec3::new(0.0, 0.0, -2.0)), "behind the subject");
        assert!(!blocks_view(camera, point, Vec3::new(3.0, 0.0, 2.0)), "off to the side");
        assert!(!blocks_view(camera, point, Vec3::new(0.0, 0.0, 9.0)), "far away towards the camera");
    }
}

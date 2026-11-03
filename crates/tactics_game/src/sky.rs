//! The sky behind the level, from the mission's `<sky>` element
//! ([`SkyDef`](tactics_core::mission::SkyDef)):
//!
//! * `color="#88aacc"`: a solid background colour;
//! * `top="#2a5d9a" horizon="#e8c49a"`: a vertical gradient (drawn on a dome);
//! * `image="skies/dusk.png" rotation="90"`: a panorama image
//!   (equirectangular, 2:1) wrapped around the battlefield on a dome. The
//!   colours, if any, show while it loads.
//!
//! Nothing set: the game's default dark sky. Generated missions get a
//! gradient that suits their theme. The dome is unlit, casts and receives
//! no shadows, and follows the camera so it never gets closer.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use tactics_core::mission::SkyDef;

use crate::camera::CameraRig;
use crate::state::BattleState;
use crate::theme;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply_sky, follow_camera).chain());
    }
}

/// Radius of the sky dome: inside the camera's far plane, beyond any level.
const DOME_RADIUS: f32 = 800.0;

#[derive(Component)]
struct SkyDome;

fn srgb(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

/// Rebuilds the sky whenever the battle's sky changes (a new mission).
#[allow(clippy::too_many_arguments)]
fn apply_sky(
    mut commands: Commands,
    state: Res<BattleState>,
    mut shown: Local<Option<SkyDef>>,
    mut clear: ResMut<ClearColor>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    domes: Query<Entity, With<SkyDome>>,
) {
    let sky = state.battle.sky();
    if shown.as_ref() == Some(sky) {
        return;
    }
    *shown = Some(sky.clone());
    for e in &domes {
        commands.entity(e).despawn();
    }
    // Background colour: the solid colour, else the horizon, else the default.
    clear.0 = sky.color.or(sky.horizon).map_or(theme::SKY, srgb);

    let dome = |mesh: Mesh, material: StandardMaterial, transform: Transform, commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>| {
        commands.spawn((
            SkyDome,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(material)),
            transform,
            NotShadowCaster,
            NotShadowReceiver,
        ));
    };
    let base = StandardMaterial { unlit: true, cull_mode: None, fog_enabled: false, ..default() };
    if let Some(path) = &sky.image {
        // Seen from inside, the sphere's texture would be mirrored: flip X.
        let turn = Quat::from_rotation_y(sky.rotation.to_radians());
        let transform = Transform::from_rotation(turn).with_scale(Vec3::new(-DOME_RADIUS, DOME_RADIUS, DOME_RADIUS));
        let material = StandardMaterial { base_color_texture: Some(assets.load(path.clone())), ..base };
        dome(Sphere::new(1.0).mesh().uv(64, 32), material, transform, &mut commands, &mut meshes, &mut materials);
    } else if let (Some(top), Some(horizon)) = (sky.top, sky.horizon) {
        let mesh = gradient_dome(srgb(top), srgb(horizon));
        let material = StandardMaterial { base_color: Color::WHITE, ..base };
        dome(mesh, material, Transform::from_scale(Vec3::splat(DOME_RADIUS)), &mut commands, &mut meshes, &mut materials);
    }
}

/// A unit sphere whose vertex colours fade from `horizon` (at and below the
/// horizon) to `top` (straight up).
fn gradient_dome(top: Color, horizon: Color) -> Mesh {
    let mut mesh = Sphere::new(1.0).mesh().uv(48, 24);
    mesh.asset_usage = RenderAssetUsages::default();
    let (top, horizon) = (top.to_linear(), horizon.to_linear());
    let colors: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(positions)) => positions
            .iter()
            .map(|p| {
                // Quick change near the horizon, slower towards the top.
                let t = p[1].max(0.0).powf(0.55);
                let c = horizon.mix(&top, t);
                [c.red, c.green, c.blue, 1.0]
            })
            .collect(),
        _ => Vec::new(),
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

/// The dome stays centred on the camera, so it always looks infinitely far.
fn follow_camera(camera: Query<&GlobalTransform, With<CameraRig>>, mut domes: Query<&mut Transform, With<SkyDome>>) {
    let Ok(cam) = camera.single() else { return };
    for mut t in &mut domes {
        t.translation = cam.translation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_runs_from_horizon_to_top() {
        let mesh = gradient_dome(Color::srgb(0.0, 0.0, 1.0), Color::srgb(1.0, 0.0, 0.0));
        let (Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Float32x4(col))) =
            (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_COLOR))
        else {
            panic!("missing attributes")
        };
        let (top, _) = pos.iter().zip(col).max_by(|a, b| a.0[1].total_cmp(&b.0[1])).unwrap();
        let (bottom, _) = pos.iter().zip(col).min_by(|a, b| a.0[1].total_cmp(&b.0[1])).unwrap();
        let at = |p: &[f32; 3]| col[pos.iter().position(|q| q == p).unwrap()];
        assert!(at(top)[2] > 0.9 && at(top)[0] < 0.1, "blue overhead");
        assert!(at(bottom)[0] > 0.9 && at(bottom)[2] < 0.1, "horizon colour below");
    }
}

//! Graphics quality (`GameSettings::graphics`): **Performance**, **Balanced**
//! or **Pretty**. Only looks change; the rules never do.
//!
//! | | Performance | Balanced | Pretty |
//! |---|---|---|---|
//! | shadows | off | 2048 map | 4096 map, 4 tight cascades (smoother, sharper up close) |
//! | anti-aliasing (MSAA) | off | 4x | 4x |
//! | lighting | sun + ambient | sun + ambient | warm sun, cool fill light, brighter ambient, bloom (desktop) |
//! | fire / gas / vines | fewer flames and puffs, no fire lights, no tendrils | full | full |
//!
//! Effects already on the map keep the detail they were made with; new ones
//! follow the setting.

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, GlobalAmbientLight, ShadowFilteringMethod};
use bevy::prelude::*;

use crate::settings::GameSettings;

/// Graphics modes (`GameSettings::graphics`).
pub const PERFORMANCE: u8 = 0;
pub const BALANCED: u8 = 1;
pub const PRETTY: u8 = 2;

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply_graphics);
    }
}

/// The sun (spawned with the board).
#[derive(Component)]
pub struct KeyLight;

/// Pretty mode's second light, from the other side (no shadows).
#[derive(Component)]
struct FillLight;

/// Applies the mode whenever it changes (and once the camera and sun exist).
#[allow(clippy::too_many_arguments)]
fn apply_graphics(
    settings: Res<GameSettings>,
    mut applied: Local<Option<u8>>,
    mut commands: Commands,
    mut key: Query<(Entity, &mut DirectionalLight, &Transform), With<KeyLight>>,
    fill: Query<Entity, With<FillLight>>,
    cameras: Query<Entity, With<Camera3d>>,
    ambient: Option<ResMut<GlobalAmbientLight>>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    let mode = settings.graphics.min(PRETTY);
    if *applied == Some(mode) {
        return;
    }
    let (Ok((sun, mut light, sun_at)), Ok(camera), Some(mut ambient)) = (key.single_mut(), cameras.single(), ambient) else { return };
    *applied = Some(mode);

    // Shadows.
    light.shadow_maps_enabled = mode != PERFORMANCE;
    shadow_map.size = if mode == PRETTY { 4096 } else { 2048 };
    let cascades = if mode == PRETTY {
        // More, tighter cascades: crisp contact shadows near the camera, no
        // blocky steps further out.
        CascadeShadowConfigBuilder { num_cascades: 4, first_cascade_far_bound: 8.0, maximum_distance: 70.0, overlap_proportion: 0.3, ..default() }
    } else {
        CascadeShadowConfigBuilder::default()
    };
    commands.entity(sun).insert(cascades.build());
    commands.entity(camera).insert((
        if mode == PERFORMANCE { Msaa::Off } else { Msaa::Sample4 },
        if mode == PERFORMANCE { ShadowFilteringMethod::Hardware2x2 } else { ShadowFilteringMethod::Gaussian },
    ));

    // Lighting.
    for e in &fill {
        commands.entity(e).despawn();
    }
    if mode == PRETTY {
        light.illuminance = 10_500.0;
        light.color = Color::srgb(1.0, 0.96, 0.88);
        light.shadow_depth_bias = 0.03;
        light.shadow_normal_bias = 1.2;
        ambient.brightness = 420.0;
        // A cool fill from the opposite side lifts the shadowed faces.
        let dir = sun_at.forward();
        commands.spawn((
            FillLight,
            DirectionalLight { illuminance: 2_200.0, color: Color::srgb(0.72, 0.82, 1.0), shadow_maps_enabled: false, ..default() },
            Transform::default().looking_to(Vec3::new(-dir.x, dir.y * 0.6, -dir.z), Vec3::Y),
        ));
    } else {
        let d = DirectionalLight::default();
        light.illuminance = 9_000.0;
        light.color = d.color;
        light.shadow_depth_bias = d.shadow_depth_bias;
        light.shadow_normal_bias = d.shadow_normal_bias;
        ambient.brightness = 350.0;
    }
    bloom(&mut commands, camera, mode == PRETTY);
}

/// A soft glow on bright things (fires, lasers, the Heart). Desktop only:
/// it needs HDR, which browsers and phones handle badly.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn bloom(commands: &mut Commands, camera: Entity, on: bool) {
    use bevy::post_process::bloom::Bloom;
    if on {
        commands.entity(camera).insert((bevy::camera::Hdr, Bloom { intensity: 0.12, ..Bloom::NATURAL }));
    } else {
        commands.entity(camera).remove::<(bevy::camera::Hdr, Bloom)>();
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn bloom(_: &mut Commands, _: Entity, _: bool) {}

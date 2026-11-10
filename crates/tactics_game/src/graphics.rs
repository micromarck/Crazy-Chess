//! Graphics quality (`GameSettings::graphics`): **Performance**, **Balanced**
//! or **Pretty**. Only looks change; the rules never do.
//!
//! | | Performance | Balanced | Pretty |
//! |---|---|---|---|
//! | shadows | off | 2048 map | 4096 map, 4 tight cascades (smoother, sharper up close) |
//! | anti-aliasing (MSAA) | off | 4x | 4x |
//! | lighting | sun + ambient | sun + the sky: ambient, an overhead skylight and a horizon bounce tinted by the mission's sky | brighter sun and sky light, bloom (desktop) |
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

/// Extra lights from the sky (Balanced and Pretty): a soft light from straight
/// above in the sky's overhead colour, and a weak bounce in its horizon colour.
#[derive(Component)]
struct SkyLight;

/// The sky's light colours: (overhead, horizon). Panoramas are averaged once
/// loaded (top third, middle band); colour skies use their colours.
fn sky_colors(sky: &tactics_core::mission::SkyDef, images: &Assets<Image>, assets: &AssetServer) -> Option<(Color, Color)> {
    let c = |rgb: [u8; 3]| Color::srgb_u8(rgb[0], rgb[1], rgb[2]);
    if let Some(path) = &sky.image {
        let image = images.get(&assets.load::<Image>(path.clone()))?;
        let (w, h) = (image.width() as usize, image.height() as usize);
        let data = image.data.as_ref()?;
        if w == 0 || h == 0 || data.len() < w * h * 4 {
            return None;
        }
        let avg = |rows: std::ops::Range<usize>| {
            let mut sum = [0u64; 3];
            let mut n = 0u64;
            for y in rows.step_by(4) {
                for x in (0..w).step_by(8) {
                    let i = (y * w + x) * 4;
                    for k in 0..3 {
                        sum[k] += data[i + k] as u64;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            Color::srgb_u8((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8)
        };
        return Some((avg(0..h / 3), avg(h * 2 / 5..h / 2)));
    }
    match (sky.color, sky.top, sky.horizon) {
        (_, Some(t), Some(hz)) => Some((c(t), c(hz))),
        (Some(col), _, _) => Some((c(col), c(col))),
        _ => Some((Color::srgb(0.62, 0.74, 0.95), Color::srgb(0.85, 0.85, 0.8))),
    }
}

/// A sky colour as a light tint: its hue, at full brightness, half-way to
/// white - so a dark night sky tints the light without plunging the map into
/// darkness.
fn tint(c: Color) -> Color {
    let l = c.to_linear();
    let m = l.red.max(l.green).max(l.blue).max(0.02);
    let (r, g, b) = (l.red / m, l.green / m, l.blue / m);
    Color::LinearRgba(LinearRgba::rgb(0.5 + 0.5 * r, 0.5 + 0.5 * g, 0.5 + 0.5 * b))
}

/// Applies the mode whenever it (or the mission's sky) changes, once the
/// camera and sun exist.
#[allow(clippy::too_many_arguments)]
fn apply_graphics(
    settings: Res<GameSettings>,
    state: Res<crate::state::BattleState>,
    images: Res<Assets<Image>>,
    assets: Res<AssetServer>,
    mut applied: Local<Option<(u8, tactics_core::mission::SkyDef, bool)>>,
    mut commands: Commands,
    mut key: Query<(Entity, &mut DirectionalLight, &Transform), With<KeyLight>>,
    extra: Query<Entity, With<SkyLight>>,
    cameras: Query<Entity, With<Camera3d>>,
    ambient: Option<ResMut<GlobalAmbientLight>>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    let mode = settings.graphics.min(PRETTY);
    let sky = state.battle.sky().clone();
    let colors = sky_colors(&sky, &images, &assets);
    let key_now = (mode, sky, colors.is_some());
    if applied.as_ref() == Some(&key_now) {
        return;
    }
    let (Ok((sun, mut light, sun_at)), Ok(camera), Some(mut ambient)) = (key.single_mut(), cameras.single(), ambient) else { return };
    *applied = Some(key_now);

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

    // Lighting: the sun, plus the sky itself (ambient, overhead, bounce).
    for e in &extra {
        commands.entity(e).despawn();
    }
    let (sky_top, sky_horizon) = colors.unwrap_or((Color::srgb(0.62, 0.74, 0.95), Color::srgb(0.85, 0.85, 0.8)));
    let (top, horizon) = (tint(sky_top), tint(sky_horizon));
    let d = DirectionalLight::default();
    match mode {
        PRETTY => {
            light.illuminance = 13_000.0;
            light.color = Color::srgb(1.0, 0.97, 0.9);
            light.shadow_depth_bias = 0.03;
            light.shadow_normal_bias = 1.2;
            ambient.color = top.mix(&horizon, 0.4);
            ambient.brightness = 900.0;
        }
        BALANCED => {
            light.illuminance = 9_500.0;
            light.color = d.color;
            light.shadow_depth_bias = d.shadow_depth_bias;
            light.shadow_normal_bias = d.shadow_normal_bias;
            ambient.color = Color::WHITE.mix(&top.mix(&horizon, 0.4), 0.5);
            ambient.brightness = 450.0;
        }
        _ => {
            light.illuminance = 9_000.0;
            light.color = d.color;
            light.shadow_depth_bias = d.shadow_depth_bias;
            light.shadow_normal_bias = d.shadow_normal_bias;
            ambient.color = Color::srgb(0.75, 0.8, 1.0);
            ambient.brightness = 400.0;
        }
    }
    if mode != PERFORMANCE {
        let strength = if mode == PRETTY { 1.0 } else { 0.55 };
        // Skylight from straight above (slightly tilted so walls catch some).
        commands.spawn((
            SkyLight,
            DirectionalLight { illuminance: 4_000.0 * strength, color: top, shadow_maps_enabled: false, ..default() },
            Transform::default().looking_to(Vec3::new(0.15, -1.0, 0.1), Vec3::Z),
        ));
        // Horizon bounce from the side opposite the sun: lifts the shadowed faces.
        let dir = sun_at.forward();
        commands.spawn((
            SkyLight,
            DirectionalLight { illuminance: 2_500.0 * strength, color: horizon, shadow_maps_enabled: false, ..default() },
            Transform::default().looking_to(Vec3::new(-dir.x, -0.35, -dir.z), Vec3::Y),
        ));
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

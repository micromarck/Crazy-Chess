//! Minimap: a north-up overview of the battlefield in the bottom-right
//! corner, for finding your way around the large levels.
//!
//! * terrain (floor, cover, walls, cut-outs, height shading), marked zones
//!   (evac green, objectives yellow, danger red) and doors;
//! * soldiers (blue), enemies the squad can see (red; fogged ones are not
//!   shown), the selected soldier highlighted, and a marker for the camera;
//! * an **enemy heatmap** once only a few hostiles are left (5 or fewer)
//!   and some are out of sight: a soft red glow roughly where each hidden
//!   one is — big, blurred and shifted a little every round, so it points
//!   the way without giving positions away;
//! * **click** it to move the camera there, **M** to show/hide it (shown by
//!   default on maps bigger than 24 tiles).

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;
use tactics_core::grid::Surface;
use tactics_core::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::coords::world_to_grid;
use crate::state::{BattleState, Selection};
use crate::theme;

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapState>()
            .add_systems(Startup, spawn_minimap)
            .add_systems(Update, (toggle, redraw, draw_heat, place_dots, click_to_focus).chain().in_set(FrameSet::Present));
    }
}

/// Largest side of the minimap in pixels.
const SIZE: f32 = 190.0;

#[derive(Resource, Default)]
struct MinimapState {
    /// What the image was drawn for: (battle seed, size and terrain version, alive props, open doors,
    /// visible zones). Units are dots on top, so moves don't redraw it.
    drawn: Option<(u64, usize, usize, usize)>,
    /// `Some(on)` once the player pressed M (overrides the default).
    user_choice: Option<bool>,
    image: Handle<Image>,
    /// The enemy heatmap texture and what it was drawn for.
    heat: Handle<Image>,
    heat_key: Option<u64>,
}

/// The heatmap layer over the minimap (under the dots).
#[derive(Component)]
struct HeatLayer;

/// Heatmap texture resolution (it is drawn smoothed, so low is fine).
const HEAT_RES: u32 = 48;
/// The heatmap shows when this many hostiles or fewer are left.
const HEAT_MAX_ENEMIES: usize = 5;

#[derive(Component)]
struct MinimapRoot;

#[derive(Component)]
struct MinimapImage;

/// A dot for a unit (or the camera, `None`).
#[derive(Component)]
struct Dot(Option<UnitId>);

fn spawn_minimap(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut state: ResMut<MinimapState>) {
    let mut image = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    state.image = images.add(image);
    let mut heat = Image::new_fill(
        Extent3d { width: HEAT_RES, height: HEAT_RES, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    heat.sampler = ImageSampler::linear();
    state.heat = images.add(heat);
    commands
        .spawn((
            MinimapRoot,
            Node {
                position_type: PositionType::Absolute,
                right: px(12),
                bottom: px(150),
                padding: UiRect::all(px(4)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(theme::UI_PANEL),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.1)),
            Visibility::Hidden,
        ))
        .with_children(|root| {
            root.spawn((
                MinimapImage,
                ImageNode::new(state.image.clone()),
                Node { width: px(SIZE), height: px(SIZE), ..default() },
                RelativeCursorPosition::default(),
            ))
            .with_child((
                HeatLayer,
                ImageNode::new(state.heat.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Pickable::IGNORE,
                Visibility::Hidden,
            ));
        });
}

/// Paints the enemy heatmap (see the module docs). Only enemies the squad
/// can't see count (visible ones are dots already).
fn draw_heat(
    battle: Res<BattleState>,
    mut state: ResMut<MinimapState>,
    mut images: ResMut<Assets<Image>>,
    mut layer: Single<&mut Visibility, With<HeatLayer>>,
) {
    let b = &battle.battle;
    let map = b.map();
    let living: Vec<&Unit> = b.living(Team::Enemy).collect();
    let hidden: Vec<&Unit> = living.iter().copied().filter(|u| !b.is_revealed(u.id)).collect();
    let show = !b.is_over() && !hidden.is_empty() && living.len() <= HEAT_MAX_ENEMIES;
    let want = if show { Visibility::Inherited } else { Visibility::Hidden };
    if **layer != want {
        **layer = want;
    }
    if !show {
        state.heat_key = None;
        return;
    }
    // Redraw when the round or a hidden enemy's position changes.
    let mut key = b.round() as u64 ^ 0xA5A5;
    for u in &hidden {
        key = key.rotate_left(11) ^ ((u.id.0 as u64) << 40 | (u.pos.x as u64) << 20 | u.pos.y as u32 as u64);
    }
    if state.heat_key == Some(key) {
        return;
    }
    state.heat_key = Some(key);

    let (w, h) = (map.width().max(1) as f32, map.height().max(1) as f32);
    // Blob radius in tiles: a good chunk of the map, never pinpoint.
    let radius = (w.max(h) * 0.09).max(5.0);
    // Each enemy's blob is shifted by up to ~half a radius, differently
    // every round (deterministic, so it doesn't flicker within a round).
    let blobs: Vec<(f32, f32)> = hidden
        .iter()
        .map(|u| {
            let mut z = (u.id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b.round() as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z ^= z >> 31;
            let jx = ((z & 0xFFFF) as f32 / 65535.0 - 0.5) * radius;
            let jy = (((z >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5) * radius;
            (u.pos.x as f32 + jx, u.pos.y as f32 + jy)
        })
        .collect();
    let mut data = Vec::with_capacity((HEAT_RES * HEAT_RES * 4) as usize);
    for row in 0..HEAT_RES {
        for col in 0..HEAT_RES {
            // Texel centre in grid coordinates (row 0 = north edge).
            let gx = (col as f32 + 0.5) / HEAT_RES as f32 * w - 0.5;
            let gy = h - 0.5 - (row as f32 + 0.5) / HEAT_RES as f32 * h;
            let heat: f32 = blobs.iter().map(|(bx, by)| (-((gx - bx).powi(2) + (gy - by).powi(2)) / (radius * radius)).exp()).sum();
            let a = (heat.min(1.0) * 0.6 * 255.0) as u8;
            data.extend_from_slice(&[235, 60, 40, a]);
        }
    }
    if let Some(mut img) = images.get_mut(&state.heat) {
        img.data = Some(data);
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    battle: Res<BattleState>,
    mut state: ResMut<MinimapState>,
    mut root: Single<&mut Visibility, With<MinimapRoot>>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        let now = **root != Visibility::Hidden;
        state.user_choice = Some(!now);
    }
    let map = battle.battle.map();
    let default_on = map.width().max(map.height()) > 24;
    let lab = battle.mission.as_deref() == Some(crate::lab::LAB_MISSION);
    let on = state.user_choice.unwrap_or(default_on) && !lab;
    let want = if on { Visibility::Inherited } else { Visibility::Hidden };
    if **root != want {
        **root = want;
    }
}

/// Colour of one map cell.
fn cell_color(battle: &Battle, props: &std::collections::HashMap<GridPos, &tactics_core::battle::Prop>, p: GridPos) -> [u8; 4] {
    let map = battle.map();
    let zone = battle.zones().iter().find(|z| z.visible && z.contains(p));
    if let Some(z) = zone {
        return match z.kind.as_str() {
            "evac" => [60, 230, 130, 255],
            "danger" => [230, 70, 60, 255],
            _ => [240, 200, 60, 255],
        };
    }
    if let Some(prop) = props.get(&p) {
        if prop.door.is_some_and(|s| !s.open) {
            return [70, 150, 165, 255];
        }
        // Supply caches: gold, so the squad can plan a route through them.
        if prop.kind.starts_with("supply") {
            return [250, 215, 70, 255];
        }
    }
    let shade = (map.elevation(p) as i32 * 14).min(70);
    let lift = |v: i32| (v + shade).clamp(0, 255) as u8;
    let surface = map.surface(p);
    match map.terrain(p) {
        Terrain::Void if surface == Surface::Water => [30, 80, 130, 255],
        Terrain::Void => [8, 9, 12, 255],
        Terrain::HighCover => match surface {
            Surface::Tree => [lift(30), lift(80), lift(35), 255],
            Surface::Brick => [lift(140), lift(70), lift(55), 255],
            _ => [lift(120), lift(124), lift(134), 255],
        },
        Terrain::LowCover => [lift(120), lift(92), lift(55), 255],
        Terrain::Floor => match surface {
            Surface::Grass => [lift(45), lift(75), lift(35), 255],
            Surface::Dirt => [lift(85), lift(65), lift(45), 255],
            Surface::Asphalt | Surface::Lines => [lift(28), lift(29), lift(32), 255],
            Surface::Interior => [lift(95), lift(70), lift(45), 255],
            _ => [lift(42), lift(46), lift(52), 255],
        },
    }
}

fn redraw(
    battle: Res<BattleState>,
    mut state: ResMut<MinimapState>,
    mut images: ResMut<Assets<Image>>,
    mut node: Single<&mut Node, With<MinimapImage>>,
) {
    let b = &battle.battle;
    let map = b.map();
    let alive = b.props().iter().filter(|p| p.alive).count();
    let open = b.props().iter().filter(|p| p.door.is_some_and(|d| d.open)).count();
    let key = (battle.seed ^ (map.width() as u64) << 20 ^ (b.map_version() as u64) << 40, alive, open, b.zones().iter().filter(|z| z.visible).count());
    if state.drawn == Some(key) {
        return;
    }
    state.drawn = Some(key);
    let props: std::collections::HashMap<GridPos, &tactics_core::battle::Prop> =
        b.props().iter().filter(|p| p.alive).map(|p| (p.pos, p)).collect();
    let (w, h) = (map.width().max(1) as u32, map.height().max(1) as u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as i32 {
        for x in 0..w as i32 {
            // Row 0 of the image is the north edge (highest grid y).
            data.extend_from_slice(&cell_color(b, &props, GridPos::new(x, h as i32 - 1 - row)));
        }
    }
    let mut image = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    if let Some(mut slot) = images.get_mut(&state.image) {
        *slot = image;
    }
    // Keep the map's aspect ratio.
    let scale = SIZE / w.max(h) as f32;
    node.width = px(w as f32 * scale);
    node.height = px(h as f32 * scale);
}

/// Pixel position (inside the image node) of a grid position.
fn to_minimap(map: &Map, x: f32, y: f32) -> Vec2 {
    let (w, h) = (map.width().max(1) as f32, map.height().max(1) as f32);
    let scale = SIZE / w.max(h);
    Vec2::new((x + 0.5) * scale, (h - 1.0 - y + 0.5) * scale)
}

fn place_dots(
    mut commands: Commands,
    battle: Res<BattleState>,
    selection: Res<Selection>,
    rig: Single<&CameraRig>,
    image: Single<Entity, With<MinimapImage>>,
    mut dots: Query<(Entity, &Dot, &mut Node, &mut BackgroundColor, &mut Visibility)>,
) {
    let b = &battle.battle;
    let map = b.map();
    // One dot per unit plus the camera marker; create missing ones.
    let have: Vec<Option<UnitId>> = dots.iter().map(|(_, d, ..)| d.0).collect();
    let wanted = b.units().iter().map(|u| Some(u.id)).chain(std::iter::once(None));
    for id in wanted.filter(|id| !have.contains(id)) {
        let dot = commands
            .spawn((
                Dot(id),
                Node { position_type: PositionType::Absolute, width: px(6), height: px(6), border_radius: BorderRadius::all(px(3)), ..default() },
                BackgroundColor(Color::NONE),
                Pickable::IGNORE,
            ))
            .id();
        commands.entity(*image).add_child(dot);
    }
    for (e, dot, mut node, mut color, mut vis) in &mut dots {
        match dot.0 {
            None => {
                // Camera: a hollow-looking larger marker at the focus point.
                let g = world_to_grid(rig.focus);
                let p = to_minimap(map, g.x as f32, g.y as f32);
                node.width = px(10);
                node.height = px(10);
                node.left = px(p.x - 5.0);
                node.top = px(p.y - 5.0);
                color.0 = Color::srgba(1.0, 1.0, 1.0, 0.35);
                *vis = Visibility::Inherited;
            }
            Some(id) => {
                let Some(u) = b.unit(id) else {
                    commands.entity(e).despawn();
                    continue;
                };
                let shown = u.is_alive() && b.is_revealed(id);
                *vis = if shown { Visibility::Inherited } else { Visibility::Hidden };
                let selected = selection.unit == Some(id);
                let size = if selected { 8.0 } else { 6.0 };
                let p = to_minimap(map, u.pos.x as f32, u.pos.y as f32);
                node.width = px(size);
                node.height = px(size);
                node.left = px(p.x - size / 2.0);
                node.top = px(p.y - size / 2.0);
                color.0 = match (u.team, selected) {
                    (Team::Player, true) => theme::SELECTED,
                    (Team::Player, false) => theme::PLAYER,
                    (Team::Enemy, _) => theme::ENEMY,
                };
            }
        }
    }
}

fn click_to_focus(
    battle: Res<BattleState>,
    buttons: Res<ButtonInput<MouseButton>>,
    image: Single<(&RelativeCursorPosition, &InheritedVisibility), With<MinimapImage>>,
    mut rig: Single<&mut CameraRig>,
) {
    let (cursor, inherited) = *image;
    if !inherited.get() || !buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(n) = cursor.normalized.filter(|_| cursor.cursor_over()) else { return };
    // `normalized` is centred: -0.5..0.5 across the node.
    let map = battle.battle.map();
    let (w, h) = (map.width() as f32, map.height() as f32);
    let x = ((n.x + 0.5) * w).floor().clamp(0.0, w - 1.0) as i32;
    let y = (h - 1.0 - ((n.y + 0.5) * h).floor()).clamp(0.0, h - 1.0) as i32;
    let p = GridPos::new(x, y);
    rig.focus_at(crate::coords::tile_top(map, p));
}

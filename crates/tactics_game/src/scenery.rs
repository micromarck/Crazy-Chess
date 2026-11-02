//! How the battlefield looks: ground materials per [`Surface`] and shaped
//! cover per wall style (brick walls with windows, shipping containers,
//! hedges, trees, rocks, sandbags, fences), raised ground that matches what
//! it is (brick buildings, container stacks, earth hills), water in river
//! cut-outs, and cars. Purely visual: rules only read the terrain.
//!
//! Mission files paint surfaces with a `<surfaces>` grid (see MISSIONS.md);
//! generated levels set them per theme.

use bevy::prelude::*;
use tactics_core::grid::{GridPos, Surface, Terrain};

use crate::theme;

/// Shared meshes and materials for scenery.
#[derive(Resource)]
pub struct Scenery {
    cube: Handle<Mesh>,
    ball: Handle<Mesh>,
    tube: Handle<Mesh>,
    /// Two shades per floor surface (checkerboard keeps distances readable).
    floors: Vec<(Surface, Handle<StandardMaterial>, Handle<StandardMaterial>)>,
    stripe: Handle<StandardMaterial>,
    water: Handle<StandardMaterial>,
    brick: Handle<StandardMaterial>,
    brick_top: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
    containers: Vec<Handle<StandardMaterial>>,
    hedge: Handle<StandardMaterial>,
    leaves: Handle<StandardMaterial>,
    bark: Handle<StandardMaterial>,
    rock: Handle<StandardMaterial>,
    sand: Handle<StandardMaterial>,
    fence: Handle<StandardMaterial>,
    concrete: Handle<StandardMaterial>,
    earth: Handle<StandardMaterial>,
    cliff: Handle<StandardMaterial>,
    cars: Vec<Handle<StandardMaterial>>,
    tyre: Handle<StandardMaterial>,
}

fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

pub fn setup_scenery(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mut mat = |c: [f32; 3], rough: f32| materials.add(StandardMaterial { base_color: rgb(c), perceptual_roughness: rough, ..default() });
    let two = |m: &mut dyn FnMut([f32; 3], f32) -> Handle<StandardMaterial>, a: [f32; 3], b: [f32; 3], r: f32| (m(a, r), m(b, r));
    let mut floors = Vec::new();
    for (s, a, b, r) in [
        (Surface::Concrete, [0.30, 0.31, 0.32], [0.33, 0.34, 0.35], 0.9),
        (Surface::Asphalt, [0.13, 0.135, 0.145], [0.15, 0.155, 0.165], 0.95),
        (Surface::Lines, [0.13, 0.135, 0.145], [0.15, 0.155, 0.165], 0.95),
        (Surface::Grass, [0.24, 0.36, 0.17], [0.27, 0.40, 0.19], 1.0),
        (Surface::Dirt, [0.40, 0.31, 0.21], [0.44, 0.34, 0.23], 1.0),
        (Surface::Interior, [0.45, 0.32, 0.20], [0.49, 0.35, 0.22], 0.7),
        (Surface::Roof, [0.22, 0.22, 0.25], [0.25, 0.25, 0.28], 0.8),
        (Surface::Metal, [0.33, 0.37, 0.42], [0.36, 0.40, 0.45], 0.4),
    ] {
        let (ma, mb) = two(&mut mat, a, b, r);
        floors.push((s, ma, mb));
    }
    let containers = [[0.62, 0.18, 0.14], [0.16, 0.32, 0.58], [0.2, 0.45, 0.3], [0.8, 0.45, 0.12], [0.45, 0.47, 0.5]].iter().map(|c| mat(*c, 0.5)).collect();
    let cars = [[0.75, 0.75, 0.78], [0.12, 0.13, 0.15], [0.55, 0.1, 0.1], [0.15, 0.25, 0.5], [0.85, 0.7, 0.2]].iter().map(|c| mat(*c, 0.3)).collect();
    let scenery = Scenery {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ball: meshes.add(Sphere::new(0.5)),
        tube: meshes.add(Cylinder::new(0.5, 1.0)),
        floors,
        stripe: mat([0.85, 0.75, 0.2], 0.8),
        brick: mat([0.55, 0.27, 0.2], 0.9),
        brick_top: mat([0.42, 0.22, 0.17], 0.9),
        containers,
        hedge: mat([0.18, 0.38, 0.16], 1.0),
        leaves: mat([0.16, 0.34, 0.14], 1.0),
        bark: mat([0.33, 0.22, 0.13], 1.0),
        rock: mat([0.45, 0.45, 0.47], 1.0),
        sand: mat([0.62, 0.55, 0.38], 1.0),
        fence: mat([0.5, 0.52, 0.55], 0.4),
        concrete: mat([0.38, 0.40, 0.45], 0.6),
        earth: mat([0.36, 0.28, 0.19], 1.0),
        cliff: mat([0.16, 0.17, 0.19], 0.95),
        tyre: mat([0.06, 0.06, 0.06], 0.9),
        cars,
        water: materials.add(StandardMaterial {
            base_color: Color::srgba(0.12, 0.32, 0.5, 0.85),
            perceptual_roughness: 0.1,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
        glass: materials.add(StandardMaterial {
            base_color: Color::srgba(0.6, 0.8, 0.9, 0.35),
            perceptual_roughness: 0.05,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    };
    commands.insert_resource(scenery);
}

/// A stable pseudo-random number for a tile (varies colours per object).
fn hash(p: GridPos) -> u32 {
    (p.x as u32).wrapping_mul(73_856_093) ^ (p.y as u32).wrapping_mul(19_349_663)
}

/// One spawned piece: mesh, material, transform.
pub struct Piece {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub transform: Transform,
}

impl Scenery {
    /// The floor material for a tile, or `None` for the default checkerboard.
    pub fn floor(&self, surface: Surface, p: GridPos) -> Option<Handle<StandardMaterial>> {
        let (_, a, b) = self.floors.iter().find(|(s, _, _)| *s == surface)?;
        Some(if (p.x + p.y) % 2 == 0 { a.clone() } else { b.clone() })
    }

    /// Extra floor decoration (road markings).
    pub fn floor_detail(&self, surface: Surface, top: Vec3) -> Option<Piece> {
        (surface == Surface::Lines).then(|| Piece {
            mesh: self.cube.clone(),
            material: self.stripe.clone(),
            transform: Transform::from_translation(top + Vec3::Y * 0.006).with_scale(Vec3::new(0.12, 0.012, 0.6)),
        })
    }

    /// Material for the side of raised ground, from what's on top.
    pub fn column(&self, surface: Surface, p: GridPos) -> Handle<StandardMaterial> {
        match surface {
            Surface::Roof | Surface::Brick | Surface::Interior => self.brick.clone(),
            Surface::Metal | Surface::Container => self.containers[(hash(GridPos::new(p.x / 3, p.y / 3)) % 5) as usize].clone(),
            Surface::Grass | Surface::Dirt | Surface::Hedge | Surface::Tree | Surface::Rock => self.earth.clone(),
            Surface::Concrete => self.concrete.clone(),
            _ => self.cliff.clone(),
        }
    }

    /// Water filling a river cut-out tile.
    pub fn water(&self, world: Vec3) -> Piece {
        Piece { mesh: self.cube.clone(), material: self.water.clone(), transform: Transform::from_translation(world - Vec3::Y * 0.22).with_scale(Vec3::new(1.0, 0.04, 1.0)) }
    }

    /// The pieces of a cover block in a given style, standing on `top`, and
    /// its height (for the camera cutaway). `None` = use the plain block.
    pub fn cover(&self, terrain: Terrain, surface: Surface, top: Vec3, p: GridPos) -> Option<(Vec<Piece>, f32)> {
        let high = terrain == Terrain::HighCover;
        let full = theme::HIGH_COVER_HEIGHT;
        let low = theme::LOW_COVER_HEIGHT;
        let piece = |mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, pos: Vec3, size: Vec3| Piece {
            mesh: mesh.clone(),
            material: material.clone(),
            transform: Transform::from_translation(top + pos).with_scale(size),
        };
        let h = hash(p);
        Some(match surface {
            Surface::Brick if high => (
                vec![
                    piece(&self.cube, &self.brick, Vec3::Y * full / 2.0, Vec3::new(0.98, full, 0.98)),
                    piece(&self.cube, &self.brick_top, Vec3::Y * (full + 0.03), Vec3::new(1.0, 0.06, 1.0)),
                ],
                full,
            ),
            // A low brick tile in a wall is a window: sill plus glass.
            Surface::Brick => (
                vec![
                    piece(&self.cube, &self.brick, Vec3::Y * low / 2.0, Vec3::new(0.98, low, 0.98)),
                    piece(&self.cube, &self.glass, Vec3::Y * (low + (full - low) / 2.0), Vec3::new(0.9, full - low, 0.08)),
                    piece(&self.cube, &self.brick_top, Vec3::Y * (full + 0.03), Vec3::new(1.0, 0.06, 1.0)),
                ],
                low,
            ),
            Surface::Container => {
                let m = &self.containers[(hash(GridPos::new(p.x / 2, p.y / 2)) % 5) as usize];
                let height = if high { full + 0.4 } else { low + 0.2 };
                let mut v = vec![piece(&self.cube, m, Vec3::Y * height / 2.0, Vec3::new(0.99, height, 0.99))];
                // Corrugation ribs.
                for i in [-0.3, 0.0, 0.3] {
                    v.push(piece(&self.cube, m, Vec3::new(i, height / 2.0, 0.0), Vec3::new(0.05, height * 0.95, 1.02)));
                }
                (v, height)
            }
            Surface::Hedge => {
                let height = if high { 1.6 } else { 0.75 };
                (vec![piece(&self.cube, &self.hedge, Vec3::Y * height / 2.0, Vec3::new(0.95, height, 0.85))], height)
            }
            Surface::Tree => {
                let s = 0.85 + (h % 30) as f32 / 100.0;
                (
                    vec![
                        piece(&self.tube, &self.bark, Vec3::Y * 0.6 * s, Vec3::new(0.2, 1.2 * s, 0.2)),
                        piece(&self.ball, &self.leaves, Vec3::Y * 1.55 * s, Vec3::splat(1.2 * s)),
                        piece(&self.ball, &self.leaves, Vec3::new(0.15, 2.15 * s, -0.1), Vec3::splat(0.8 * s)),
                    ],
                    2.4 * s,
                )
            }
            Surface::Rock => {
                let (sx, sz) = (0.8 + (h % 20) as f32 / 100.0, 0.75 + (h % 17) as f32 / 100.0);
                let height = if high { 1.5 } else { 0.6 };
                (vec![piece(&self.ball, &self.rock, Vec3::Y * height * 0.45, Vec3::new(sx, height, sz))], height)
            }
            Surface::Sandbags => (
                (0..3)
                    .map(|i| piece(&self.cube, &self.sand, Vec3::Y * (0.1 + i as f32 * 0.18), Vec3::new(0.95 - i as f32 * 0.06, 0.17, 0.5)))
                    .collect(),
                low,
            ),
            Surface::Fence => (
                vec![
                    piece(&self.tube, &self.fence, Vec3::new(-0.45, 0.5, 0.0), Vec3::new(0.06, 1.0, 0.06)),
                    piece(&self.tube, &self.fence, Vec3::new(0.45, 0.5, 0.0), Vec3::new(0.06, 1.0, 0.06)),
                    piece(&self.cube, &self.glass, Vec3::Y * 0.5, Vec3::new(0.95, 0.9, 0.03)),
                    piece(&self.cube, &self.fence, Vec3::Y * 0.97, Vec3::new(0.95, 0.04, 0.04)),
                ],
                1.0,
            ),
            _ => return None,
        })
    }

    /// A parked car (prop kind `car`): body, cabin and wheels.
    pub fn car(&self, top: Vec3, p: GridPos) -> Vec<Piece> {
        let paint = &self.cars[(hash(p) % self.cars.len() as u32) as usize];
        let along_x = hash(p).is_multiple_of(2);
        let r = if along_x { Quat::IDENTITY } else { Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) };
        let at = |v: Vec3| top + r * v;
        let mut v = vec![
            Piece { mesh: self.cube.clone(), material: paint.clone(), transform: Transform::from_translation(at(Vec3::Y * 0.33)).with_rotation(r).with_scale(Vec3::new(0.95, 0.32, 0.55)) },
            Piece { mesh: self.cube.clone(), material: self.glass.clone(), transform: Transform::from_translation(at(Vec3::new(-0.05, 0.6, 0.0))).with_rotation(r).with_scale(Vec3::new(0.5, 0.24, 0.5)) },
        ];
        for (x, z) in [(-0.3, -0.28), (0.3, -0.28), (-0.3, 0.28), (0.3, 0.28)] {
            v.push(Piece {
                mesh: self.tube.clone(),
                material: self.tyre.clone(),
                transform: Transform::from_translation(at(Vec3::new(x, 0.13, z))).with_rotation(r * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(0.26, 0.08, 0.26)),
            });
        }
        v
    }
}

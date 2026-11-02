//! How individual units look: body, colours, hair, headgear, gear and the
//! weapon in their hands.
//!
//! Everything hangs off **sockets**, named attachment points on the model:
//!
//! | socket | built-in rig (humanoid) | what goes there |
//! |---|---|---|
//! | `weapon` | right-hand grip, -Z = barrel | the held weapon |
//! | `hand_l` | left hand | (off-hand items) |
//! | `head` | centre of the head | hair, helmet / cap / beret / visor / hood |
//! | `back` | between the shoulders | backpack |
//! | `chest` | front of the chest | vest, armour plates, grenades, tags |
//! | `hip_l`, `hip_r` | sides of the hips | medkit, stims |
//!
//! * **Built-in rigs** take their sockets from the skeleton
//!   (`SkeletonDef::sockets`), so custom skeletons can move or add them.
//! * **Drop-in glTF characters** map sockets to nodes of the file: explicit
//!   `sockets` in the `.character.ron` sidecar (or a module `ModelDef`), else
//!   a guess from node names (`RightHand`, `hand_r`, `mixamorig:RightHand`,
//!   `Head`, `Spine2`...). The weapon is always drawn; headgear and gear only
//!   with `show_gear: true`.
//!
//! Looks come from the unit's [`Appearance`] (saved soldiers carry theirs;
//! other soldiers get a stable random one from their name). Enemies without
//! one keep their team colours. Weapons are drawn from their
//! [`WeaponLook`] unless the weapon names a custom `model`; items likewise.

use bevy::prelude::*;
use tactics_core::content::{Appearance, BoneDef, Hair, Headgear, Ruleset, SocketDef};
use tactics_core::prelude::*;
use tactics_core::unit::WeaponLook;

use crate::FrameSet;
use crate::models::{ModelLibrary, ModelOwner, SceneReady};
use crate::units::UnitVisual;

pub struct LooksPlugin;

impl Plugin for LooksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup_looks)
            .add_systems(Update, (dress_gltf, fix_gltf_sockets).chain().in_set(FrameSet::Present));
    }
}

// ----------------------------------------------------------------------
// Palettes and shared assets
// ----------------------------------------------------------------------

const SKIN: [[f32; 3]; 6] = [[0.98, 0.85, 0.72], [0.92, 0.74, 0.58], [0.80, 0.60, 0.44], [0.64, 0.45, 0.31], [0.45, 0.31, 0.21], [0.32, 0.22, 0.15]];
const HAIR: [[f32; 3]; 6] = [[0.08, 0.07, 0.06], [0.30, 0.18, 0.10], [0.85, 0.70, 0.40], [0.60, 0.22, 0.10], [0.62, 0.62, 0.62], [0.20, 0.35, 0.80]];
/// Armour colours: team blue, olive, urban grey, desert, black, white, crimson, teal.
const ARMOR: [[f32; 3]; 8] = [
    [0.25, 0.45, 0.85],
    [0.35, 0.42, 0.25],
    [0.45, 0.47, 0.50],
    [0.70, 0.60, 0.42],
    [0.15, 0.16, 0.18],
    [0.85, 0.86, 0.88],
    [0.60, 0.15, 0.15],
    [0.15, 0.50, 0.50],
];

/// Names for the HUD (same order as the palettes).
pub const SKIN_NAMES: [&str; 6] = ["Pale", "Light", "Tan", "Olive", "Brown", "Dark"];
pub const HAIR_NAMES: [&str; 6] = ["Black", "Brown", "Blond", "Red", "Grey", "Blue"];
pub const ARMOR_NAMES: [&str; 8] = ["Blue", "Olive", "Urban", "Desert", "Black", "White", "Crimson", "Teal"];

/// Meshes and materials shared by every unit's looks.
#[derive(Resource)]
pub struct LooksAssets {
    cube: Handle<Mesh>,
    ball: Handle<Mesh>,
    tube: Handle<Mesh>,
    skin: Vec<Handle<StandardMaterial>>,
    hair: Vec<Handle<StandardMaterial>>,
    armor: Vec<Handle<StandardMaterial>>,
    trim: Vec<Handle<StandardMaterial>>,
    gun: Handle<StandardMaterial>,
    gun_light: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    visor: Handle<StandardMaterial>,
    white: Handle<StandardMaterial>,
    red: Handle<StandardMaterial>,
    strap: Handle<StandardMaterial>,
    grenade: Handle<StandardMaterial>,
    ghost: Handle<StandardMaterial>,
}

fn setup_looks(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mut mat = |c: [f32; 3], rough: f32| {
        materials.add(StandardMaterial { base_color: Color::srgb(c[0], c[1], c[2]), perceptual_roughness: rough, ..default() })
    };
    let skin = SKIN.iter().map(|c| mat(*c, 0.8)).collect();
    let hair = HAIR.iter().map(|c| mat(*c, 0.9)).collect();
    let armor = ARMOR.iter().map(|c| mat(*c, 0.55)).collect();
    let trim = ARMOR.iter().map(|c| mat([c[0] * 0.55, c[1] * 0.55, c[2] * 0.55], 0.7)).collect();
    let gun = mat([0.12, 0.12, 0.13], 0.4);
    let gun_light = mat([0.32, 0.33, 0.35], 0.5);
    let white = mat([0.9, 0.9, 0.88], 0.6);
    let red = mat([0.8, 0.1, 0.1], 0.6);
    let strap = mat([0.13, 0.12, 0.11], 0.9);
    let grenade = mat([0.28, 0.33, 0.2], 0.6);
    let ghost = mat([0.18, 0.2, 0.24], 0.3);
    let glow = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 1.0, 0.5),
        emissive: LinearRgba::rgb(1.2, 4.0, 1.5),
        ..default()
    });
    let visor = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.8, 1.0),
        emissive: LinearRgba::rgb(0.6, 2.4, 3.2),
        ..default()
    });
    commands.insert_resource(LooksAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ball: meshes.add(Sphere::new(0.5)),
        tube: meshes.add(Cylinder::new(0.5, 1.0)),
        skin,
        hair,
        armor,
        trim,
        gun,
        gun_light,
        glow,
        visor,
        white,
        red,
        strap,
        grenade,
        ghost,
    });
}

// ----------------------------------------------------------------------
// What a unit wears
// ----------------------------------------------------------------------

/// Everything that decides how one unit is dressed.
#[derive(Debug, Clone, PartialEq)]
pub struct Outfit {
    /// `None`: keep team colours (enemies without a saved look).
    pub appearance: Option<Appearance>,
    pub weapon: WeaponLook,
    pub weapon_model: Option<String>,
    pub scoped: bool,
    pub armor: Option<String>,
    pub items: Vec<String>,
}

impl Outfit {
    pub fn of(unit: &Unit, rules: &Ruleset) -> Self {
        let appearance = unit.appearance.or_else(|| (unit.team == Team::Player).then(|| Appearance::from_name(&unit.name)));
        let weapon = rules.weapon(&unit.weapon.id).unwrap_or(&unit.weapon);
        Self {
            appearance,
            weapon: weapon.look,
            weapon_model: weapon.model.clone(),
            scoped: unit.gear.items.iter().any(|i| i == "scope"),
            armor: unit.gear.armor.clone(),
            items: unit.gear.items.clone(),
        }
    }
}

/// Which material a part uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mat {
    Armor,
    Trim,
    Skin,
    /// Hair colour.
    Locks,
    Gun,
    GunLight,
    Glow,
    Visor,
    White,
    Red,
    Strap,
    Grenade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Cube,
    Ball,
    /// Cylinder along Y (rotate to lay it along Z).
    Tube,
}

/// One primitive piece of an attachment, in socket space (metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    pub shape: Shape,
    pub pos: Vec3,
    pub size: Vec3,
    /// Rotation, degrees (XYZ).
    pub rot: Vec3,
    pub mat: Mat,
}

fn cube(pos: [f32; 3], size: [f32; 3], mat: Mat) -> Part {
    Part { shape: Shape::Cube, pos: Vec3::from(pos), size: Vec3::from(size), rot: Vec3::ZERO, mat }
}
fn ball(pos: [f32; 3], size: [f32; 3], mat: Mat) -> Part {
    Part { shape: Shape::Ball, pos: Vec3::from(pos), size: Vec3::from(size), rot: Vec3::ZERO, mat }
}
/// A cylinder lying along Z (a barrel, a scope).
fn barrel(pos: [f32; 3], diameter: f32, length: f32, mat: Mat) -> Part {
    Part { shape: Shape::Tube, pos: Vec3::from(pos), size: Vec3::new(diameter, length, diameter), rot: Vec3::new(90.0, 0.0, 0.0), mat }
}

/// The held weapon, grip at the origin, barrel along -Z.
pub fn weapon_parts(look: WeaponLook, scoped: bool) -> Vec<Part> {
    use Mat::*;
    let mut p = match look {
        WeaponLook::Rifle => vec![
            cube([0.0, 0.02, -0.08], [0.06, 0.09, 0.30], Gun),
            cube([0.0, 0.03, -0.26], [0.055, 0.06, 0.12], GunLight),
            barrel([0.0, 0.035, -0.40], 0.025, 0.2, Gun),
            cube([0.0, 0.0, 0.14], [0.05, 0.08, 0.16], Gun),
            cube([0.0, -0.07, -0.12], [0.035, 0.11, 0.06], GunLight),
            cube([0.0, 0.085, -0.06], [0.02, 0.03, 0.06], Gun),
        ],
        WeaponLook::Shotgun => vec![
            cube([0.0, 0.02, -0.06], [0.07, 0.1, 0.26], Gun),
            barrel([0.0, 0.04, -0.33], 0.04, 0.3, Gun),
            cube([0.0, -0.01, -0.3], [0.06, 0.05, 0.12], Trim),
            cube([0.0, -0.01, 0.15], [0.05, 0.1, 0.18], Trim),
        ],
        WeaponLook::Sniper => vec![
            cube([0.0, 0.02, -0.08], [0.06, 0.08, 0.34], Gun),
            barrel([0.0, 0.03, -0.5], 0.022, 0.5, Gun),
            barrel([0.0, 0.1, -0.08], 0.045, 0.24, GunLight),
            cube([0.0, -0.01, 0.17], [0.05, 0.1, 0.22], Trim),
            cube([0.0, -0.06, -0.06], [0.03, 0.06, 0.05], Gun),
        ],
        WeaponLook::Smg => vec![
            cube([0.0, 0.02, -0.05], [0.06, 0.09, 0.22], Gun),
            barrel([0.0, 0.035, -0.21], 0.025, 0.1, Gun),
            cube([0.0, -0.1, -0.04], [0.035, 0.16, 0.05], GunLight),
            cube([0.0, 0.0, 0.1], [0.02, 0.05, 0.12], Gun),
        ],
        WeaponLook::Lmg => vec![
            cube([0.0, 0.02, -0.08], [0.08, 0.12, 0.36], Gun),
            barrel([0.0, 0.04, -0.42], 0.035, 0.34, Gun),
            cube([0.0, -0.09, -0.08], [0.1, 0.1, 0.1], Trim),
            cube([0.0, 0.1, -0.1], [0.03, 0.03, 0.12], GunLight),
            cube([0.0, 0.0, 0.17], [0.06, 0.1, 0.18], Gun),
        ],
        WeaponLook::Pistol => vec![cube([0.0, 0.04, -0.05], [0.035, 0.045, 0.16], Gun), cube([0.0, -0.01, 0.01], [0.03, 0.09, 0.04], GunLight)],
        WeaponLook::Plasma => vec![
            cube([0.0, 0.02, -0.05], [0.06, 0.07, 0.18], Gun),
            ball([0.0, 0.05, -0.08], [0.05, 0.05, 0.05], Glow),
            barrel([0.0, 0.03, -0.17], 0.035, 0.08, Glow),
        ],
        WeaponLook::None => Vec::new(),
    };
    if scoped && !matches!(look, WeaponLook::Sniper | WeaponLook::None) {
        p.push(barrel([0.0, 0.1, -0.08], 0.035, 0.13, GunLight));
    }
    p
}

/// Where the support hand holds a two-handed weapon: metres down the
/// barrel from the grip. `None` = one-handed (pistols) or nothing held.
pub fn foregrip(look: WeaponLook) -> Option<f32> {
    match look {
        WeaponLook::Rifle => Some(0.22),
        WeaponLook::Shotgun => Some(0.24),
        WeaponLook::Sniper => Some(0.24),
        WeaponLook::Smg => Some(0.16),
        WeaponLook::Lmg => Some(0.24),
        WeaponLook::Pistol | WeaponLook::Plasma | WeaponLook::None => None,
    }
}

/// Hair and headgear, relative to the centre of the head (radius ~0.12).
pub fn head_parts(a: &Appearance) -> Vec<Part> {
    use Mat::*;
    let mut p = Vec::new();
    let hair_shows = matches!(a.headgear, Headgear::None | Headgear::Cap | Headgear::Beret);
    if hair_shows {
        match a.hair {
            Hair::Bald => {}
            Hair::Short => p.push(ball([0.0, 0.04, 0.012], [0.255, 0.17, 0.26], Locks)),
            Hair::Long => {
                p.push(ball([0.0, 0.04, 0.012], [0.255, 0.17, 0.26], Locks));
                p.push(cube([0.0, -0.06, 0.1], [0.2, 0.2, 0.06], Locks));
            }
            Hair::Mohawk => {
                if a.headgear == Headgear::None {
                    p.push(cube([0.0, 0.12, 0.0], [0.03, 0.08, 0.24], Locks));
                }
            }
            Hair::Bun => {
                p.push(ball([0.0, 0.04, 0.012], [0.255, 0.17, 0.26], Locks));
                p.push(ball([0.0, 0.06, 0.12], [0.09, 0.09, 0.09], Locks));
            }
        }
    }
    match a.headgear {
        Headgear::None => {}
        Headgear::Helmet => {
            p.push(ball([0.0, 0.045, 0.005], [0.27, 0.2, 0.28], Armor));
            p.push(Part { shape: Shape::Tube, pos: Vec3::new(0.0, 0.0, 0.0), size: Vec3::new(0.285, 0.02, 0.285), rot: Vec3::ZERO, mat: Trim });
        }
        Headgear::Cap => {
            p.push(Part { shape: Shape::Tube, pos: Vec3::new(0.0, 0.07, 0.0), size: Vec3::new(0.25, 0.06, 0.25), rot: Vec3::ZERO, mat: Trim });
            p.push(cube([0.0, 0.05, -0.13], [0.16, 0.012, 0.09], Trim));
        }
        Headgear::Beret => p.push(Part {
            shape: Shape::Ball,
            pos: Vec3::new(-0.02, 0.095, 0.0),
            size: Vec3::new(0.27, 0.07, 0.26),
            rot: Vec3::new(0.0, 0.0, 10.0),
            mat: Trim,
        }),
        Headgear::Visor => {
            p.push(ball([0.0, 0.045, 0.005], [0.27, 0.2, 0.28], Armor));
            p.push(cube([0.0, 0.0, -0.115], [0.2, 0.05, 0.04], Visor));
        }
        Headgear::Hood => p.push(ball([0.0, 0.015, 0.035], [0.29, 0.29, 0.29], Strap)),
    }
    // Eyes, unless a visor covers them.
    if a.headgear != Headgear::Visor {
        for x in [-0.04, 0.04] {
            p.push(cube([x, 0.015, -0.115], [0.03, 0.02, 0.01], Strap));
        }
    }
    p
}

pub fn backpack_parts() -> Vec<Part> {
    vec![
        cube([0.0, 0.0, 0.06], [0.26, 0.28, 0.12], Mat::Trim),
        Part { shape: Shape::Tube, pos: Vec3::new(0.0, 0.16, 0.06), size: Vec3::new(0.07, 0.24, 0.07), rot: Vec3::new(0.0, 0.0, 90.0), mat: Mat::Strap },
    ]
}

/// Built-in visuals for known items: `(socket, parts)`. `index` counts
/// repeats (two grenades sit side by side).
pub fn item_parts(item: &str, index: usize) -> Option<(&'static str, Vec<Part>)> {
    use Mat::*;
    let k = index as f32;
    Some(match item {
        "kevlar" => ("chest", vec![cube([0.0, -0.02, 0.02], [0.41, 0.22, 0.05], Strap)]),
        "plated_armor" => (
            "chest",
            vec![
                cube([0.0, 0.0, 0.0], [0.36, 0.2, 0.05], Armor),
                cube([-0.24, 0.07, 0.12], [0.12, 0.06, 0.16], Armor),
                cube([0.24, 0.07, 0.12], [0.12, 0.06, 0.16], Armor),
            ],
        ),
        "frag" => ("chest", vec![ball([0.1 - k * 0.07, -0.08, -0.03], [0.06, 0.07, 0.06], Grenade)]),
        "medkit_item" => (
            "hip_l",
            vec![
                cube([-0.02 - k * 0.02, 0.0, -0.02 + k * 0.08], [0.06, 0.08, 0.09], White),
                cube([-0.055 - k * 0.02, 0.0, -0.02 + k * 0.08], [0.005, 0.05, 0.015], Red),
                cube([-0.055 - k * 0.02, 0.0, -0.02 + k * 0.08], [0.005, 0.015, 0.05], Red),
            ],
        ),
        "stims" => ("hip_r", vec![Part { shape: Shape::Tube, pos: Vec3::new(0.03, 0.0, -0.02), size: Vec3::new(0.025, 0.08, 0.025), rot: Vec3::ZERO, mat: Glow }]),
        "dog_tags" => ("chest", vec![cube([0.0, 0.05, -0.035], [0.02, 0.03, 0.005], GunLight)]),
        _ => return None,
    })
}

// ----------------------------------------------------------------------
// Spawning
// ----------------------------------------------------------------------

impl LooksAssets {
    fn material(&self, mat: Mat, outfit: &Outfit) -> Handle<StandardMaterial> {
        let a = outfit.appearance.unwrap_or_default();
        let ghost = outfit.armor.as_deref() == Some("ghost_suit");
        match mat {
            Mat::Armor if ghost => self.ghost.clone(),
            Mat::Armor => self.armor[a.armor as usize % self.armor.len()].clone(),
            Mat::Trim => self.trim[a.accent as usize % self.trim.len()].clone(),
            Mat::Skin => self.skin[a.skin as usize % self.skin.len()].clone(),
            Mat::Locks => self.hair[a.hair_color as usize % self.hair.len()].clone(),
            Mat::Gun => self.gun.clone(),
            Mat::GunLight => self.gun_light.clone(),
            Mat::Glow => self.glow.clone(),
            Mat::Visor => self.visor.clone(),
            Mat::White => self.white.clone(),
            Mat::Red => self.red.clone(),
            Mat::Strap => self.strap.clone(),
            Mat::Grenade => self.grenade.clone(),
        }
    }

    fn spawn_parts(&self, commands: &mut Commands, parent: Entity, parts: &[Part], outfit: &Outfit) {
        for part in parts {
            let mesh = match part.shape {
                Shape::Cube => &self.cube,
                Shape::Ball => &self.ball,
                Shape::Tube => &self.tube,
            };
            let r = part.rot * std::f32::consts::PI / 180.0;
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(self.material(part.mat, outfit)),
                Transform::from_translation(part.pos)
                    .with_rotation(Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z))
                    .with_scale(part.size),
                ChildOf(parent),
            ));
        }
    }

    /// Material and shape scale for a bone of the built-in humanoid rig when
    /// the unit has a look: skin for head and hands, armour on the torso and
    /// arms, trim for the trousers, dark boots; slimmer/bulkier limbs.
    pub fn dress_bone(&self, bone: &BoneDef, outfit: &Outfit) -> Option<(Handle<StandardMaterial>, Vec3)> {
        let a = outfit.appearance?;
        let n = bone.name.as_str();
        let role = if n.starts_with("head") || n.starts_with("neck") || n.starts_with("hand") {
            Mat::Skin
        } else if n.starts_with("chest") || n.starts_with("spine") || n.starts_with("upper_arm") || n.starts_with("forearm") {
            Mat::Armor
        } else if n.starts_with("hips") || n.starts_with("thigh") || n.starts_with("shin") {
            Mat::Trim
        } else if n.starts_with("foot") {
            Mat::Strap
        } else {
            return None;
        };
        let bulk = if role == Mat::Skin { 1.0 } else { a.bulk() };
        Some((self.material(role, outfit), Vec3::new(bulk, 1.0, bulk)))
    }

    /// Hangs the outfit on the given sockets (`name -> entity`). `full`
    /// = also headgear, backpack and gear (not just the weapon).
    pub fn dress_sockets(
        &self,
        commands: &mut Commands,
        library: &ModelLibrary,
        sockets: &[(String, Entity)],
        outfit: &Outfit,
        full: bool,
    ) {
        let at = |name: &str| sockets.iter().find(|(n, _)| n == name).map(|(_, e)| *e);
        if let Some(hand) = at("weapon") {
            match outfit.weapon_model.as_deref().and_then(|m| library.get(m)).and_then(|m| m.scene.clone().map(|s| (s, m.def.scale))) {
                Some((scene, scale)) => {
                    commands.spawn((WorldAssetRoot(scene), Transform::from_scale(Vec3::splat(scale)), ChildOf(hand)));
                }
                None => self.spawn_parts(commands, hand, &weapon_parts(outfit.weapon, outfit.scoped), outfit),
            }
        }
        if !full {
            return;
        }
        if let Some(a) = outfit.appearance {
            if let Some(head) = at("head") {
                self.spawn_parts(commands, head, &head_parts(&a), outfit);
            }
            if a.backpack
                && let Some(back) = at("back")
            {
                self.spawn_parts(commands, back, &backpack_parts(), outfit);
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for item in outfit.armor.iter().chain(outfit.items.iter()) {
            let index = seen.iter().filter(|s| **s == item.as_str()).count();
            seen.push(item);
            if let Some((socket, parts)) = item_parts(item, index)
                && let Some(e) = at(socket)
            {
                self.spawn_parts(commands, e, &parts, outfit);
            }
        }
    }
}

/// Spawns socket entities under the rig's bones (`bones[i]` is bone `i`).
pub fn rig_sockets(commands: &mut Commands, skeleton: &tactics_core::content::ResolvedSkeleton, bones: &[Entity]) -> Vec<(String, Entity)> {
    skeleton
        .sockets
        .iter()
        .filter_map(|s| {
            let bone = bones.get(skeleton.index_of(&s.bone)?)?;
            let r = Vec3::from(s.rotation) * std::f32::consts::PI / 180.0;
            let e = commands
                .spawn((
                    Transform::from_translation(Vec3::from(s.offset)).with_rotation(Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z)),
                    Visibility::default(),
                    Name::new(format!("socket:{}", s.name)),
                    ChildOf(*bone),
                ))
                .id();
            Some((s.name.clone(), e))
        })
        .collect()
}

// ----------------------------------------------------------------------
// glTF characters
// ----------------------------------------------------------------------

/// Common node names per socket, tried in order (lower-case, separators removed).
const GUESSES: [(&str, &[&str]); 7] = [
    ("weapon", &["righthand", "handr", "rhand", "handright", "wristr", "rightwrist", "lowerarmr", "forearmr", "rightforearm"]),
    ("hand_l", &["lefthand", "handl", "lhand", "handleft", "lowerarml", "forearml", "leftforearm"]),
    ("head", &["head", "headtop"]),
    ("back", &["spine2", "upperchest", "chest", "spine1", "spine"]),
    ("chest", &["spine2", "upperchest", "chest", "spine1", "spine"]),
    ("hip_l", &["leftupleg", "thighl", "upperlegl", "hips", "pelvis"]),
    ("hip_r", &["rightupleg", "thighr", "upperlegr", "hips", "pelvis"]),
];

fn simplify(name: &str) -> String {
    let base = name.rsplit([':', '|']).next().unwrap_or(name);
    base.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// Picks the node for each socket: explicit bindings first, then guesses.
pub fn bind_sockets(explicit: &[SocketDef], nodes: &[(Entity, String)]) -> Vec<(SocketDef, Entity)> {
    let mut out = Vec::new();
    for (socket, guesses) in GUESSES {
        if let Some(def) = explicit.iter().find(|s| s.name == socket) {
            if let Some((e, _)) = nodes.iter().find(|(_, n)| *n == def.bone || simplify(n) == simplify(&def.bone)) {
                out.push((def.clone(), *e));
            }
            continue;
        }
        let found = guesses.iter().find_map(|g| nodes.iter().find(|(_, n)| simplify(n) == *g));
        if let Some((e, name)) = found {
            out.push((SocketDef { name: socket.into(), bone: name.clone(), ..Default::default() }, *e));
        }
    }
    // Extra explicit sockets with other names.
    for def in explicit.iter().filter(|d| !GUESSES.iter().any(|(s, _)| *s == d.name)) {
        if let Some((e, _)) = nodes.iter().find(|(_, n)| *n == def.bone) {
            out.push((def.clone(), *e));
        }
    }
    out
}

/// Marks a glTF scene that has been dressed.
#[derive(Component)]
struct Dressed;

/// Keeps an attachment on a glTF node at world scale and facing the unit's
/// way, whatever the node's own scale and rotation are.
#[derive(Component)]
struct SocketFix {
    node: Entity,
    unit_root: Entity,
    offset: Vec3,
    rotation: Quat,
}

#[allow(clippy::too_many_arguments)]
fn dress_gltf(
    mut commands: Commands,
    looks: Res<LooksAssets>,
    library: Res<ModelLibrary>,
    state: Res<crate::state::BattleState>,
    scenes: Query<(Entity, &ModelOwner), (With<SceneReady>, Without<Dressed>)>,
    units: Query<&UnitVisual>,
    children: Query<&Children>,
    names: Query<&Name>,
    globals: Query<&GlobalTransform>,
) {
    for (scene, owner) in &scenes {
        let Ok(visual) = units.get(owner.unit_root) else { continue };
        let Some(unit) = state.battle.unit(visual.id) else { continue };
        let Some(model) = library.get(&owner.model) else { continue };
        let nodes: Vec<(Entity, String)> = children
            .iter_descendants(scene)
            .filter_map(|e| names.get(e).ok().map(|n| (e, n.as_str().to_string())))
            .collect();
        // Wait until transforms have propagated (identity = not yet).
        if nodes.iter().all(|(e, _)| globals.get(*e).is_ok_and(|g| *g == GlobalTransform::IDENTITY)) {
            continue;
        }
        commands.entity(scene).insert(Dressed);
        let outfit = Outfit::of(unit, state.battle.rules());
        let mut sockets = Vec::new();
        for (def, node) in bind_sockets(&model.def.sockets, &nodes) {
            let r = Vec3::from(def.rotation) * std::f32::consts::PI / 180.0;
            let holder = commands
                .spawn((
                    Transform::default(),
                    Visibility::default(),
                    Name::new(format!("socket:{}", def.name)),
                    SocketFix {
                        node,
                        unit_root: owner.unit_root,
                        offset: Vec3::from(def.offset),
                        rotation: Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z),
                    },
                    ChildOf(node),
                ))
                .id();
            sockets.push((def.name.clone(), holder));
        }
        looks.dress_sockets(&mut commands, &library, &sockets, &outfit, model.show_gear);
    }
}

/// Every frame: undo the node's scale and turn the attachment to face the
/// unit's forward (plus the socket's own rotation), so a gun in a glTF hand
/// is the right size and points where the soldier looks.
fn fix_gltf_sockets(mut fixes: Query<(&SocketFix, &mut Transform)>, globals: Query<&GlobalTransform>) {
    for (fix, mut tr) in &mut fixes {
        let (Ok(node), Ok(unit)) = (globals.get(fix.node), globals.get(fix.unit_root)) else { continue };
        let (node_scale, node_rot, _) = node.to_scale_rotation_translation();
        let (_, unit_rot, _) = unit.to_scale_rotation_translation();
        let inv = node_rot.inverse();
        let safe = node_scale.max(Vec3::splat(1e-4));
        tr.scale = Vec3::ONE / safe;
        tr.rotation = inv * unit_rot * fix.rotation;
        tr.translation = (inv * (unit_rot * fix.offset)) / safe;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_weapon_look_has_parts_except_none() {
        for look in [WeaponLook::Rifle, WeaponLook::Shotgun, WeaponLook::Sniper, WeaponLook::Smg, WeaponLook::Lmg, WeaponLook::Pistol, WeaponLook::Plasma] {
            assert!(!weapon_parts(look, false).is_empty(), "{look:?}");
            assert!(weapon_parts(look, true).len() >= weapon_parts(look, false).len());
        }
        assert!(weapon_parts(WeaponLook::None, true).is_empty());
    }

    #[test]
    fn headgear_hides_hair_and_visor_hides_eyes() {
        let mut a = Appearance { hair: Hair::Long, headgear: Headgear::Helmet, ..Default::default() };
        assert!(!head_parts(&a).iter().any(|p| p.mat == Mat::Locks));
        a.headgear = Headgear::None;
        assert!(head_parts(&a).iter().any(|p| p.mat == Mat::Locks));
        a.headgear = Headgear::Visor;
        assert!(head_parts(&a).iter().any(|p| p.mat == Mat::Visor));
    }

    #[test]
    fn sockets_bind_by_explicit_name_then_guess() {
        let e = |i: u32| Entity::from_raw_u32(i).unwrap();
        let nodes = vec![(e(1), "mixamorig:RightHand".to_string()), (e(2), "mixamorig:Head".to_string()), (e(3), "Spine2".to_string())];
        let bound = bind_sockets(&[], &nodes);
        let find = |s: &str| bound.iter().find(|(d, _)| d.name == s).map(|(_, e)| *e);
        assert_eq!(find("weapon"), Some(e(1)));
        assert_eq!(find("head"), Some(e(2)));
        assert_eq!(find("back"), Some(e(3)));
        // An explicit binding wins (even a "wrong" node).
        let explicit = [SocketDef { name: "weapon".into(), bone: "Spine2".into(), ..Default::default() }];
        let bound = bind_sockets(&explicit, &nodes);
        assert_eq!(bound.iter().find(|(d, _)| d.name == "weapon").map(|(_, e)| *e), Some(e(3)));
        // The robot test model: no hands, the forearm is used.
        let robot = vec![(e(9), "LowerArm.R".to_string())];
        assert_eq!(bind_sockets(&[], &robot)[0].1, e(9));
    }
}

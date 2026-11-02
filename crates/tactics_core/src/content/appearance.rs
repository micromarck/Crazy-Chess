//! How an individual looks: body, colours, hair and headgear.
//!
//! An [`Appearance`] stores small indices into palettes and style lists; the
//! frontend decides the actual colours and shapes (see the game's
//! `looks.rs`), so the same saved soldier can be drawn by the built-in rig
//! or tinted on a custom model. Soldiers saved in the roster keep theirs;
//! units without one get a stable random look from their name.

use crate::rng::GameRng;

/// Body build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Build {
    Slim,
    #[default]
    Standard,
    Heavy,
}

/// What's on the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Headgear {
    None,
    #[default]
    Helmet,
    Cap,
    Beret,
    Visor,
    Hood,
}

/// Hair style (shown when the headgear leaves it visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize))]
pub enum Hair {
    Bald,
    #[default]
    Short,
    Long,
    Mohawk,
    Bun,
}

/// Sizes of the frontend's palettes (indices wrap).
pub const SKIN_TONES: u8 = 6;
pub const HAIR_COLORS: u8 = 6;
pub const ARMOR_COLORS: u8 = 8;

/// One soldier's looks.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "data", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Appearance {
    pub build: Build,
    /// Height multiplier (0.9 - 1.1).
    pub height: f32,
    pub skin: u8,
    pub hair: Hair,
    pub hair_color: u8,
    pub headgear: Headgear,
    /// Main armour colour (palette index).
    pub armor: u8,
    /// Trim / gear colour (palette index).
    pub accent: u8,
    pub backpack: bool,
}

impl Appearance {
    pub const BUILDS: [Build; 3] = [Build::Slim, Build::Standard, Build::Heavy];
    pub const HEADGEAR: [Headgear; 6] = [Headgear::None, Headgear::Helmet, Headgear::Cap, Headgear::Beret, Headgear::Visor, Headgear::Hood];
    pub const HAIR: [Hair; 5] = [Hair::Bald, Hair::Short, Hair::Long, Hair::Mohawk, Hair::Bun];

    /// A random look (deterministic for a seed).
    pub fn random(seed: u64) -> Self {
        let mut rng = GameRng::new(seed ^ 0xA77E_A2A9_CE00);
        let mut pick = |n: usize| rng.range_inclusive(0, n as i32 - 1) as usize;
        let build = Self::BUILDS[pick(3)];
        let headgear = Self::HEADGEAR[pick(6)];
        let hair = Self::HAIR[pick(5)];
        Self {
            build,
            height: 0.92 + pick(9) as f32 * 0.02,
            skin: pick(SKIN_TONES as usize) as u8,
            hair,
            hair_color: pick(HAIR_COLORS as usize) as u8,
            headgear,
            armor: pick(ARMOR_COLORS as usize) as u8,
            accent: pick(ARMOR_COLORS as usize) as u8,
            backpack: pick(3) > 0,
        }
    }

    /// A stable random look derived from a name (for units without one).
    pub fn from_name(name: &str) -> Self {
        Self::random(name_seed(name))
    }

    /// Height multiplier, clamped to something sensible.
    pub fn height_scale(&self) -> f32 {
        if self.height <= 0.0 { 1.0 } else { self.height.clamp(0.8, 1.2) }
    }

    /// Width multiplier of the torso and limbs for the build.
    pub fn bulk(&self) -> f32 {
        match self.build {
            Build::Slim => 0.88,
            Build::Standard => 1.0,
            Build::Heavy => 1.15,
        }
    }

    /// Cycles one field to its next value: `build`, `height`, `skin`,
    /// `hair`, `hair_color`, `headgear`, `armor`, `accent`, `backpack`.
    /// Returns `false` for an unknown field.
    pub fn cycle(&mut self, field: &str) -> bool {
        fn next<T: Copy + PartialEq>(list: &[T], v: T) -> T {
            let i = list.iter().position(|x| *x == v).map_or(0, |i| (i + 1) % list.len());
            list[i]
        }
        match field {
            "build" => self.build = next(&Self::BUILDS, self.build),
            "height" => {
                let h = self.height_scale() + 0.04;
                self.height = if h > 1.1 { 0.9 } else { (h * 100.0).round() / 100.0 };
            }
            "skin" => self.skin = (self.skin + 1) % SKIN_TONES,
            "hair" => self.hair = next(&Self::HAIR, self.hair),
            "hair_color" => self.hair_color = (self.hair_color + 1) % HAIR_COLORS,
            "headgear" => self.headgear = next(&Self::HEADGEAR, self.headgear),
            "armor" => self.armor = (self.armor + 1) % ARMOR_COLORS,
            "accent" => self.accent = (self.accent + 1) % ARMOR_COLORS,
            "backpack" => self.backpack = !self.backpack,
            _ => return false,
        }
        true
    }
}

/// FNV-1a: a stable hash for seeding looks from names.
pub fn name_seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

const FIRST: [&str; 32] = [
    "Alex", "Sam", "Jordan", "Riley", "Maya", "Kenji", "Amara", "Lars", "Sofia", "Diego", "Nia", "Ivan", "Leila", "Tomas", "Aiko", "Omar",
    "Freya", "Kwame", "Ines", "Rafael", "Yara", "Mateo", "Hana", "Viktor", "Zara", "Elias", "Noor", "Pavel", "Lucia", "Tariq", "Mei", "Jonas",
];
const LAST: [&str; 32] = [
    "Vance", "Okoye", "Moreau", "Kowalski", "Reyes", "Lindqvist", "Tanaka", "Haddad", "Novak", "Silva", "Mensah", "Ivanova", "Byrne", "Costa",
    "Nakamura", "Petrov", "Adeyemi", "Larsen", "Romero", "Khan", "Schmidt", "Dubois", "Kaur", "Ortega", "Sato", "Varga", "Ali", "Brennan",
    "Fischer", "Mbeki", "Rossi", "Yilmaz",
];
const RANKS: [&str; 5] = ["Pvt.", "Pvt.", "Cpl.", "Sgt.", "Spc."];

/// A random soldier name like "Cpl. Maya Novak" (deterministic for a seed).
pub fn random_name(seed: u64) -> String {
    let mut rng = GameRng::new(seed ^ 0x0000_005E_ED0F_A3E5);
    let mut pick = |n: usize| rng.range_inclusive(0, n as i32 - 1) as usize;
    format!("{} {} {}", RANKS[pick(RANKS.len())], FIRST[pick(FIRST.len())], LAST[pick(LAST.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_looks_vary_and_are_stable() {
        assert_eq!(Appearance::from_name("Sgt. Vance"), Appearance::from_name("Sgt. Vance"));
        let looks: Vec<Appearance> = (0..20).map(Appearance::random).collect();
        assert!(looks.windows(2).any(|w| w[0].headgear != w[1].headgear));
        assert!(looks.iter().all(|a| (0.9..=1.1).contains(&a.height_scale())));
        assert_ne!(random_name(1), random_name(2));
    }

    #[test]
    fn cycling_wraps_every_field() {
        let mut a = Appearance::random(3);
        for field in ["build", "height", "skin", "hair", "hair_color", "headgear", "armor", "accent", "backpack"] {
            let start = a;
            let mut changed = false;
            for _ in 0..12 {
                assert!(a.cycle(field));
                changed |= a != start;
            }
            assert!(changed, "{field} cycles");
        }
        assert!(!a.cycle("nope"));
    }
}

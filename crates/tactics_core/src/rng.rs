//! Deterministic pseudo-random number generation.
//!
//! The rules layer must be **reproducible**: given the same seed and the same
//! sequence of actions, a battle must play out identically. That property is what
//! makes replays, save-scumming protection, lock-step multiplayer and unit tests
//! possible. We therefore never use thread-local or OS randomness inside the core
//! crate; every roll goes through this small, dependency-free generator whose state
//! lives inside [`crate::battle::Battle`].
//!
//! The algorithm is SplitMix64: tiny, fast, statistically decent for gameplay, and
//! trivially serializable (a single `u64`). It also avoids pulling in `getrandom`,
//! which needs special configuration on `wasm32-unknown-unknown`.

/// A seeded, deterministic random number generator (SplitMix64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRng {
    state: u64,
}

impl GameRng {
    /// Creates a generator from a seed. Equal seeds produce equal sequences.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a value uniformly distributed in `low..=high` (inclusive).
    ///
    /// If `high < low` the bounds are swapped rather than panicking, because a
    /// mis-configured weapon should never crash a match.
    pub fn range_inclusive(&mut self, low: i32, high: i32) -> i32 {
        let (lo, hi) = if low <= high { (low, high) } else { (high, low) };
        let span = (hi as i64 - lo as i64 + 1) as u64;
        // Modulo bias is negligible for the tiny spans used in gameplay.
        lo + (self.next_u64() % span) as i32
    }

    /// Rolls a d100 (1..=100) and returns `true` if the roll is `<= chance`.
    ///
    /// `chance` is a percentage; values `<= 0` always fail and `>= 100` always succeed.
    pub fn roll_percent(&mut self, chance: i32) -> bool {
        self.range_inclusive(1, 100) <= chance
    }

    /// Exposes the internal state so it can be saved and restored.
    pub fn state(&self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = GameRng::new(42);
        let mut b = GameRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn range_is_inclusive_and_bounded() {
        let mut rng = GameRng::new(7);
        let mut seen_lo = false;
        let mut seen_hi = false;
        for _ in 0..1000 {
            let v = rng.range_inclusive(3, 5);
            assert!((3..=5).contains(&v));
            seen_lo |= v == 3;
            seen_hi |= v == 5;
        }
        assert!(seen_lo && seen_hi);
    }

    #[test]
    fn percent_extremes() {
        let mut rng = GameRng::new(1);
        for _ in 0..200 {
            assert!(!rng.roll_percent(0));
            assert!(rng.roll_percent(100));
        }
    }
}

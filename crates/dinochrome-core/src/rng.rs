//! The simulation's random number source.
//!
//! Everything the game rolls for after the maze is generated — which way a
//! wandering drone turns next, which kind a factory builds — comes out of here, so
//! that a run is reproducible from its seed and not merely its maze.
//!
//! It is a concrete type with a handful of concrete methods rather than the `Rng`
//! trait, and that is the point of it: the Bevy layer holds one of these in a
//! resource and never has to depend on `rand` or bring a trait into scope. The
//! generator is xoshiro256++ for the same reason [`crate::maze::generate`] uses
//! it — `rand` documents its algorithm as stable across releases and platforms,
//! which is what makes a printed seed worth anything.

use rand::{RngExt, SeedableRng, rngs::Xoshiro256PlusPlus};

/// A seeded, reproducible source of randomness for the simulation.
#[derive(Clone, Debug)]
pub struct SimRng(Xoshiro256PlusPlus);

impl SimRng {
    /// A generator that will produce the same sequence for the same `seed`.
    pub fn from_seed(seed: u64) -> Self {
        Self(Xoshiro256PlusPlus::seed_from_u64(seed))
    }

    /// A value in `0.0..1.0`.
    ///
    /// The half-open range is what callers want: it indexes a table by
    /// multiplication without a one-in-four-billion chance of landing past the
    /// end of it.
    pub fn unit(&mut self) -> f32 {
        self.0.random::<f32>()
    }

    /// A value in `low..high`, or `low` if the range is empty or backwards.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        if high <= low {
            return low;
        }
        low + self.unit() * (high - low)
    }

    /// An index in `0..len`, or `None` if `len` is zero.
    pub fn below(&mut self, len: usize) -> Option<usize> {
        (len > 0).then(|| (self.unit() * len as f32) as usize % len)
    }

    /// True with probability `chance`, clamped to `0.0..=1.0`.
    pub fn chance(&mut self, chance: f32) -> bool {
        self.unit() < chance
    }
}

impl Default for SimRng {
    fn default() -> Self {
        Self::from_seed(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_produces_the_same_sequence() {
        let mut a = SimRng::from_seed(12345);
        let mut b = SimRng::from_seed(12345);
        for _ in 0..64 {
            assert_eq!(a.unit(), b.unit());
        }
    }

    #[test]
    fn different_seeds_produce_different_sequences() {
        let mut a = SimRng::from_seed(1);
        let mut b = SimRng::from_seed(2);
        let differ = (0..64).any(|_| a.unit() != b.unit());
        assert!(differ, "two seeds gave the same first 64 draws");
    }

    #[test]
    fn unit_stays_inside_the_half_open_range_it_promises() {
        let mut rng = SimRng::from_seed(7);
        for _ in 0..10_000 {
            let value = rng.unit();
            assert!((0.0..1.0).contains(&value), "got {value}");
        }
    }

    #[test]
    fn below_indexes_a_table_without_ever_running_off_the_end() {
        // The reason this is worth asserting rather than assuming: `unit` is a
        // float and rounding at the top of its range would index one past the last
        // element, which is exactly the sort of thing that shows up once an hour
        // in a shipped game and never in a test that only draws ten times.
        let mut rng = SimRng::from_seed(99);
        for len in [1usize, 2, 3, 4, 7] {
            for _ in 0..2_000 {
                let index = rng.below(len).expect("a non-empty table");
                assert!(index < len, "index {index} for a table of {len}");
            }
        }
    }

    #[test]
    fn an_empty_table_has_no_index_to_pick() {
        assert_eq!(SimRng::from_seed(3).below(0), None);
    }

    #[test]
    fn range_stays_between_its_ends_and_copes_with_a_backwards_one() {
        let mut rng = SimRng::from_seed(5);
        for _ in 0..1_000 {
            let value = rng.range(2.0, 5.0);
            assert!((2.0..5.0).contains(&value), "got {value}");
        }
        assert_eq!(rng.range(4.0, 4.0), 4.0, "an empty range");
        assert_eq!(rng.range(9.0, 1.0), 9.0, "a backwards range");
    }

    #[test]
    fn a_certainty_and_an_impossibility_are_decided_without_being_rolled_for() {
        let mut rng = SimRng::from_seed(17);
        for _ in 0..1_000 {
            assert!(rng.chance(1.0));
            assert!(!rng.chance(0.0));
        }
    }
}

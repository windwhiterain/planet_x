//! A small, dependency-free, deterministic PRNG (SplitMix64).
//!
//! SplitMix64 gives us a sequence of `u64`s from a single `u64` seed. It is
//! tiny, fast, and perfectly reproducible, which is all a trajectory generator
//! needs. We layer a couple of convenience methods on top for picking integers
//! and floats in ranges so the simulation code stays readable.

pub struct Prng {
    state: u64,
}

impl Prng {
    pub fn new(seed: u64) -> Self {
        Prng { state: seed }
    }

    /// The current internal state. Combined with [`Prng::from_state`] this lets a
    /// checkpoint capture and later restore the exact RNG position, so a saved
    /// run resumes deterministically.
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Rebuild a PRNG at a previously-saved internal position.
    pub fn from_state(state: u64) -> Self {
        Prng { state }
    }

    /// A freshly seeded RNG.
    #[allow(clippy::should_implement_trait)]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `[0, n)`. Returns 0 when `n == 0`.
    pub fn range(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Rejection sampling to avoid modulo bias.
        let limit = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < limit {
                return v % n;
            }
        }
    }

    /// Uniform float in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform float in `[lo, hi)`.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Pick an element from a slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let idx = self.range(items.len() as u64) as usize;
        &items[idx]
    }
}

/// Reasonable seed derived from system time for the `random` option.
pub fn random_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5EED);
    // Mix in a fresh value so two runs in the same nanosecond differ.
    nanos ^ 0x9E37_79B9_7F4A_7C15
}

#[cfg(test)]
#[path = "tests/prng.rs"]
mod tests;

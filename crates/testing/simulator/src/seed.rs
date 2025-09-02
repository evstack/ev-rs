//! Seed-based deterministic random number generation.
//!
//! This module provides a seeded PRNG wrapper that tracks its seed for reproducibility.
//! Every simulation run can be exactly reproduced by providing the same seed.

use rand::distributions::uniform::{SampleRange, SampleUniform};
use rand::distributions::{Distribution, Standard};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};

/// A deterministic random number generator with seed tracking.
///
/// Uses ChaCha20 for cryptographic quality randomness while maintaining
/// reproducibility via explicit seeding.
#[derive(Clone)]
pub struct SeededRng {
    /// The original seed used to initialize this RNG.
    seed: u64,
    /// The underlying ChaCha20 RNG instance.
    rng: ChaCha20Rng,
}

impl SeededRng {
    /// Creates a new seeded RNG from the given seed.
    ///
    /// The same seed will always produce the same sequence of random values.
    pub fn new(seed: u64) -> Self {
        let rng = ChaCha20Rng::seed_from_u64(seed);
        Self { seed, rng }
    }

    /// Creates a new RNG with a random seed from system entropy.
    ///
    /// Returns both the RNG and the seed used, allowing the seed to be
    /// recorded for later reproduction.
    pub fn from_entropy() -> (Self, u64) {
        let seed: u64 = rand::random();
        (Self::new(seed), seed)
    }

    /// Returns the seed used to initialize this RNG.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Generates a random value of type T.
    #[inline]
    pub fn gen<T>(&mut self) -> T
    where
        Standard: Distribution<T>,
    {
        self.rng.gen()
    }

    /// Generates a random value in the given range.
    #[inline]
    pub fn gen_range<T, R>(&mut self, range: R) -> T
    where
        T: SampleUniform,
        R: SampleRange<T>,
    {
        self.rng.gen_range(range)
    }

    /// Returns true with the given probability (0.0 to 1.0).
    #[inline]
    pub fn gen_bool(&mut self, probability: f64) -> bool {
        self.rng.gen_bool(probability.clamp(0.0, 1.0))
    }

    /// Generates random bytes to fill the given slice.
    #[inline]
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.rng.fill(dest);
    }

    /// Creates a child RNG that is deterministically derived from this one.
    ///
    /// Useful for creating independent random streams while maintaining
    /// overall reproducibility.
    pub fn derive_child(&mut self) -> Self {
        let child_seed: u64 = self.gen();
        Self::new(child_seed)
    }
}

/// Seed information for reproduction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SeedInfo {
    /// The primary seed used for the simulation.
    pub seed: u64,
    /// Optional git commit hash for exact reproduction.
    pub commit_hash: Option<[u8; 20]>,
}

impl SeedInfo {
    /// Creates new seed info with just a seed.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            commit_hash: None,
        }
    }

    /// Creates seed info with both seed and commit hash.
    pub fn with_commit(seed: u64, commit_hash: [u8; 20]) -> Self {
        Self {
            seed,
            commit_hash: Some(commit_hash),
        }
    }

    /// Returns a reproduction command string.
    pub fn reproduction_command(&self) -> String {
        match self.commit_hash {
            Some(hash) => {
                let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
                format!(
                    "EVOLVE_TEST_SEED={} EVOLVE_TEST_COMMIT={} cargo test",
                    self.seed, hash_hex
                )
            }
            None => format!("EVOLVE_TEST_SEED={} cargo test", self.seed),
        }
    }
}

impl std::fmt::Display for SeedInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.commit_hash {
            Some(hash) => {
                let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
                write!(f, "seed={}, commit={}", self.seed, hash_hex)
            }
            None => write!(f, "seed={}", self.seed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determinism() {
        let seed = 12345u64;
        let mut rng1 = SeededRng::new(seed);
        let mut rng2 = SeededRng::new(seed);

        // Same seed should produce identical sequences
        for _ in 0..100 {
            let v1: u64 = rng1.gen();
            let v2: u64 = rng2.gen();
            assert_eq!(v1, v2);
        }
    }

    #[test]
    fn test_different_seeds_different_sequences() {
        let mut rng1 = SeededRng::new(1);
        let mut rng2 = SeededRng::new(2);

        let values1: Vec<u64> = (0..10).map(|_| rng1.gen()).collect();
        let values2: Vec<u64> = (0..10).map(|_| rng2.gen()).collect();

        assert_ne!(values1, values2);
    }

    #[test]
    fn test_gen_bool() {
        let mut rng = SeededRng::new(42);

        // With p=0.0, should never return true
        for _ in 0..100 {
            assert!(!rng.gen_bool(0.0));
        }

        // With p=1.0, should always return true
        for _ in 0..100 {
            assert!(rng.gen_bool(1.0));
        }
    }

    #[test]
    fn test_derive_child() {
        let seed = 999u64;
        let mut rng1 = SeededRng::new(seed);
        let mut rng2 = SeededRng::new(seed);

        let child1 = rng1.derive_child();
        let child2 = rng2.derive_child();

        // Children should have the same seed since parent was deterministic
        assert_eq!(child1.seed(), child2.seed());
    }

    #[test]
    fn test_seed_info_display() {
        let info = SeedInfo::new(12345);
        assert_eq!(info.to_string(), "seed=12345");

        let info_with_commit = SeedInfo::with_commit(12345, [0xab; 20]);
        assert!(info_with_commit.to_string().contains("commit="));
    }
}

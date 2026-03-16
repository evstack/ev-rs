//! Shared property-test configuration for the Evolve workspace.
//!
//! Provides a single source of truth for case counts so every crate
//! respects `EVOLVE_PROPTEST_CASES`, CI detection, and a sensible local
//! default without duplicating the logic.

const DEFAULT_CASES: u32 = 128;
const CI_CASES: u32 = 32;

/// Return the number of proptest cases to run.
///
/// Priority:
/// 1. `EVOLVE_PROPTEST_CASES` env var (must parse to a positive `u32`).
/// 2. `CI` or `EVOLVE_CI` env var present → [`CI_CASES`].
/// 3. Otherwise → [`DEFAULT_CASES`].
pub fn proptest_cases() -> u32 {
    if let Ok(value) = std::env::var("EVOLVE_PROPTEST_CASES") {
        if let Ok(parsed) = value.parse::<u32>() {
            if parsed > 0 {
                return parsed;
            }
        }
    }

    if std::env::var("EVOLVE_CI").is_ok() || std::env::var("CI").is_ok() {
        return CI_CASES;
    }

    DEFAULT_CASES
}

/// Build a [`proptest::test_runner::Config`] using [`proptest_cases`].
pub fn proptest_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: proptest_cases(),
        ..Default::default()
    }
}

//! Startup checks and initialization.
//!
//! This module provides:
//! - Storage path validation
//! - Disk space checks
//! - State integrity verification
//! - Startup sequence orchestration

pub mod checks;
mod sequence;

pub use checks::{check_disk_space, check_state_integrity, check_storage_path};
pub use sequence::{run_startup_sequence, run_startup_with_mkdir, StartupResult};

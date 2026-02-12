//! Simulator storage configuration.
//!
//! The simulator supports `mock` and `qmdb` backends. These settings are kept to preserve
//! existing configuration callsites, but fault-injection knobs are no-ops for current backends.

use serde::{Deserialize, Serialize};

/// Configuration for simulator storage behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Read fault probability in [0.0, 1.0]. Currently unused by `mock` and `qmdb`.
    pub read_fault_prob: f64,
    /// Write fault probability in [0.0, 1.0]. Currently unused by `mock` and `qmdb`.
    pub write_fault_prob: f64,
    /// Enables storage operation logging when supported by the backend.
    pub log_operations: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            read_fault_prob: 0.0,
            write_fault_prob: 0.0,
            log_operations: false,
        }
    }
}

impl StorageConfig {
    pub fn with_faults(read_fault_prob: f64, write_fault_prob: f64) -> Self {
        Self {
            read_fault_prob,
            write_fault_prob,
            ..Default::default()
        }
    }

    pub fn no_faults() -> Self {
        Self::default()
    }

    pub fn with_logging(mut self) -> Self {
        self.log_operations = true;
        self
    }
}

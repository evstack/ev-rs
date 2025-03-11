use evolve_core::SdkResult;
use evolve_gas::account::{StorageGasConfig, ERR_OUT_OF_GAS};

/// Represents either infinite gas or finite gas mode, with relevant fields.
pub enum GasCounter<'a> {
    Infinite,
    Finite {
        gas_limit: u64,
        gas_used: u64,
        storage_gas_config: &'a StorageGasConfig,
    },
}

impl GasCounter<'_> {
    /// Create an infinite `GasCounter`.
    pub fn infinite() -> Self {
        GasCounter::Infinite
    }

    /// Create a finite `GasCounter`.
    pub fn finite(gas_limit: u64, config: &'_ StorageGasConfig) -> GasCounter<'_> {
        GasCounter::Finite {
            gas_limit,
            gas_used: 0,
            storage_gas_config: config,
        }
    }

    /// Consume `gas` units, if finite. For infinite, it's a no-op.
    pub fn consume_gas(&mut self, gas: u64) -> SdkResult<()> {
        match self {
            GasCounter::Infinite => Ok(()),
            GasCounter::Finite {
                gas_limit,
                gas_used,
                ..
            } => {
                *gas_used += gas;
                if *gas_used > *gas_limit {
                    return Err(ERR_OUT_OF_GAS);
                }
                Ok(())
            }
        }
    }

    pub fn consume_get_gas(&mut self, key: &[u8], value: &Option<Vec<u8>>) -> SdkResult<()> {
        match self {
            GasCounter::Infinite => Ok(()),
            GasCounter::Finite {
                storage_gas_config, ..
            } => {
                let value_len = match value {
                    None => 1,
                    Some(value) => 1 + value.len() as u64,
                };
                let gas =
                    storage_gas_config.storage_get_charge * ((key.len() + 1) as u64 + value_len);
                self.consume_gas(gas)
            }
        }
    }

    pub fn consume_set_gas(&mut self, key: &[u8], value: &[u8]) -> SdkResult<()> {
        match self {
            GasCounter::Infinite => Ok(()),
            GasCounter::Finite {
                storage_gas_config, ..
            } => {
                let gas = storage_gas_config.storage_get_charge
                    * (key.len() + 1 + value.len() + 1) as u64;
                self.consume_gas(gas)
            }
        }
    }

    pub fn consume_remove_gas(&mut self, key: &[u8]) -> SdkResult<()> {
        match self {
            GasCounter::Infinite => Ok(()),
            GasCounter::Finite {
                storage_gas_config, ..
            } => {
                let gas = storage_gas_config.storage_get_charge * (key.len() + 1) as u64;
                self.consume_gas(gas)
            }
        }
    }

    /// Returns the total gas used so far. Always zero in infinite mode.
    pub fn gas_used(&self) -> u64 {
        match self {
            GasCounter::Infinite => 0,
            GasCounter::Finite { gas_used, .. } => *gas_used,
        }
    }
}

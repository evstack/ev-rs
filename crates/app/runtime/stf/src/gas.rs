use evolve_core::{Message, SdkResult};
use evolve_gas::account::{StorageGasConfig, ERR_OUT_OF_GAS};

/// Represents how gas is tracked and consumed.
///
/// - **Infinite** mode: No gas limit applies.
/// - **Finite** mode: Tracks gas usage against a specified limit.
///
/// # Examples
///
/// ```
/// use evolve_core::SdkResult;
/// use evolve_gas::account::StorageGasConfig;
/// use evolve_stf::gas::GasCounter;
///
/// // Assume you have a StorageGasConfig instance.
/// // In a real scenario, this would be defined or passed in.
/// let config = StorageGasConfig {
///     storage_get_charge: 10,
///     storage_set_charge: 0,
///     storage_remove_charge: 0
/// };
///
/// // Create a finite gas counter.
/// let mut gc = GasCounter::finite(1000, config);
///
/// // Consume some gas.
/// gc.consume_gas(200).unwrap(); // OK
///
/// // Attempt to consume more gas than remaining.
/// let result = gc.consume_gas(10000);
/// assert!(result.is_err()); // Should exceed limit and return ERR_OUT_OF_GAS
///
/// // Create an infinite gas counter.
/// let mut gc_infinite = GasCounter::infinite();
/// // Consumes no matter how much gas without errors.
/// gc_infinite.consume_gas(9999999).unwrap();
/// assert_eq!(gc_infinite.gas_used(), 0);
/// ```
#[allow(dead_code)]
pub enum GasCounter {
    /// Infinite gas mode, no tracking or limit.
    Infinite,
    /// Finite gas mode, tracking gas usage against a `gas_limit`.
    ///
    /// - `gas_limit`: Maximum allowed gas usage.
    /// - `gas_used`: Current amount of gas used.
    /// - `storage_gas_config`: Configuration for storage-based gas operations.
    Finite {
        gas_limit: u64,
        gas_used: u64,
        storage_gas_config: StorageGasConfig,
    },
}

impl GasCounter {
    /// Creates a new [`GasCounter`] in infinite gas mode.
    ///
    /// In this mode, any gas-consuming operations will succeed
    /// and [`gas_used`](GasCounter::gas_used) always returns 0.
    pub fn infinite() -> Self {
        GasCounter::Infinite
    }

    /// Creates a new [`GasCounter`] in finite gas mode.
    ///
    /// The `gas_limit` is the total amount of gas that can be consumed
    /// before an error is returned. The `config` parameter provides the
    /// storage-related gas consumption configuration.
    ///
    /// # Parameters
    ///
    /// - `gas_limit`: Maximum gas allowed.
    /// - `config`: Reference to a [`StorageGasConfig`] instance.
    ///
    /// # Returns
    ///
    /// A [`GasCounter`] instance in finite mode.
    pub fn finite(gas_limit: u64, config: StorageGasConfig) -> GasCounter {
        GasCounter::Finite {
            gas_limit,
            gas_used: 0,
            storage_gas_config: config,
        }
    }

    /// Consumes the specified `gas` amount if in finite mode.
    ///
    /// - In **Infinite** mode, this is a no-op (always returns `Ok(())`).
    /// - In **Finite** mode, increases `gas_used` by `gas`. If this exceeds
    ///   the `gas_limit`, returns an [`ERR_OUT_OF_GAS`].
    ///
    /// # Errors
    ///
    /// Returns [`ERR_OUT_OF_GAS`] if `gas_used` would exceed `gas_limit` in finite mode.
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

    /// Consumes gas for a "get" operation involving the given `key` and `value`.
    ///
    /// The amount of gas consumed is based on:
    ///
    ///  - `storage_gas_config.storage_get_charge`
    ///  - The length of the key plus the length of the value.
    ///  - A small overhead for absent values (`None`).
    ///
    /// # Errors
    ///
    /// Returns [`ERR_OUT_OF_GAS`] if the resulting gas usage exceeds the limit in finite mode.
    pub fn consume_get_gas(&mut self, key: &[u8], value: &Option<Message>) -> SdkResult<()> {
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

    /// Consumes gas for a "set" operation for the given `key` and `value`.
    ///
    /// The amount of gas consumed is based on:
    ///
    /// - `storage_gas_config.storage_get_charge`
    /// - The length of both key and value, plus overhead.
    ///
    /// # Errors
    ///
    /// Returns [`ERR_OUT_OF_GAS`] if the resulting gas usage exceeds the limit in finite mode.
    pub fn consume_set_gas(&mut self, key: &[u8], value: &Message) -> SdkResult<()> {
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

    /// Consumes gas for a "remove" operation involving the given `key`.
    ///
    /// The amount of gas consumed is based on:
    ///
    /// - `storage_gas_config.storage_get_charge`
    /// - The length of the key, plus overhead.
    ///
    /// # Errors
    ///
    /// Returns [`ERR_OUT_OF_GAS`] if the resulting gas usage exceeds the limit in finite mode.
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

    /// Returns the total gas used so far.
    ///
    /// Always returns `0` in **Infinite** mode.
    ///
    /// # Examples
    ///
    /// ```
    /// # use evolve_core::SdkResult;
    /// # use evolve_gas::account::StorageGasConfig;
    /// # use evolve_stf::gas::GasCounter;
    /// # let config = StorageGasConfig { storage_get_charge: 5, storage_set_charge: 10,storage_remove_charge: 10};
    /// # let mut gc = GasCounter::finite(100, config);
    /// gc.consume_gas(10).unwrap();
    /// assert_eq!(gc.gas_used(), 10);
    /// ```
    pub fn gas_used(&self) -> u64 {
        match self {
            GasCounter::Infinite => 0,
            GasCounter::Finite { gas_used, .. } => *gas_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::Message;
    use evolve_gas::account::{StorageGasConfig, ERR_OUT_OF_GAS};

    #[test]
    fn test_infinite_mode() {
        let mut gc = GasCounter::infinite();

        // Consuming gas should never fail and never increase usage.
        assert!(gc.consume_gas(1000).is_ok());
        assert_eq!(gc.gas_used(), 0);

        // Test other consume_* methods in infinite mode.
        assert!(gc
            .consume_get_gas(b"key", &Some(Message::from_bytes(vec![1, 2, 3])))
            .is_ok());
        assert!(gc
            .consume_set_gas(b"key", &Message::from_bytes(b"value".to_vec()))
            .is_ok());
        assert!(gc.consume_remove_gas(b"key").is_ok());
        assert_eq!(gc.gas_used(), 0);
    }

    #[test]
    fn test_finite_mode_within_limit() {
        let config = StorageGasConfig {
            storage_get_charge: 10,
            storage_set_charge: 10,
            storage_remove_charge: 10,
        };
        let mut gc = GasCounter::finite(100, config);

        // Initial usage should be 0
        assert_eq!(gc.gas_used(), 0);

        // Consume some gas within limit
        gc.consume_gas(50).unwrap();
        assert_eq!(gc.gas_used(), 50);

        // Still within limit
        gc.consume_gas(50).unwrap();
        assert_eq!(gc.gas_used(), 100);

        // Exactly at limit is okay, but going beyond should fail
        let res = gc.consume_gas(1);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), ERR_OUT_OF_GAS);
    }

    #[test]
    fn test_finite_mode_get() {
        let config = StorageGasConfig {
            storage_get_charge: 10,
            storage_set_charge: 0,
            storage_remove_charge: 0,
        };
        let mut gc = GasCounter::finite(200, config);

        // key len = 3, value len = 3 => total length considered = (3+1) + (3+1) = 8
        // gas = 10 * 8 = 80
        gc.consume_get_gas(b"key", &Some(Message::from_bytes(vec![1, 2, 3])))
            .unwrap();
        assert_eq!(gc.gas_used(), 80);

        // Next operation to see if we can still consume more gas
        gc.consume_get_gas(b"k", &None).unwrap();
        // key len = 1, no value => total length considered = (1+1) + 1 = 3
        // gas = 10 * 3 = 30
        assert_eq!(gc.gas_used(), 80 + 30);
    }

    #[test]
    fn test_finite_mode_set() {
        let config = StorageGasConfig {
            storage_get_charge: 5,
            storage_set_charge: 5,
            storage_remove_charge: 0,
        };
        let mut gc = GasCounter::finite(50, config);

        // key len = 3, value len = 5 => total length considered = (3+1) + (5+1) = 10
        // gas = 5 * 10 = 50
        let result = gc.consume_set_gas(b"key", &Message::from_bytes(b"value".to_vec()));
        assert!(result.is_ok());
        assert_eq!(gc.gas_used(), 50);

        // Next consume would exceed the limit
        let result = gc.consume_set_gas(b"key", &Message::from_bytes(b"v".to_vec()));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);
    }

    #[test]
    fn test_finite_mode_remove() {
        let config = StorageGasConfig {
            storage_get_charge: 10,
            storage_set_charge: 10,
            storage_remove_charge: 10,
        };
        let mut gc = GasCounter::finite(30, config);

        // key len = 3 => total length considered = (3+1) = 4
        // gas = 10 * 4 = 40 which exceeds the limit
        let result = gc.consume_remove_gas(b"key");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);

        // If we shorten the key or increase the limit, it would pass.
    }
}

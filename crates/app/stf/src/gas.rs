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
#[derive(Debug, Clone)]
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
                // Use saturating_add to prevent overflow
                let new_gas_used = gas_used.saturating_add(gas);
                if new_gas_used > *gas_limit {
                    return Err(ERR_OUT_OF_GAS);
                }
                *gas_used = new_gas_used;
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
                // Calculate total size with overflow protection
                let key_len = key.len() as u64;
                let value_len = match value {
                    None => 1u64,
                    Some(value) => (value.len() as u64).saturating_add(1),
                };

                // Use saturating operations to prevent overflow
                let total_size = key_len.saturating_add(1).saturating_add(value_len);
                let gas = storage_gas_config
                    .storage_get_charge
                    .saturating_mul(total_size);

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
                // Calculate total size with overflow protection
                let total_size = (key.len() as u64)
                    .saturating_add(1)
                    .saturating_add(value.len() as u64)
                    .saturating_add(1);
                let gas = storage_gas_config
                    .storage_get_charge
                    .saturating_mul(total_size);

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
                // Calculate total size with overflow protection
                let total_size = (key.len() as u64).saturating_add(1);
                let gas = storage_gas_config
                    .storage_get_charge
                    .saturating_mul(total_size);

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
    use proptest::prelude::*;

    const MAX_OPS: usize = 64;
    const DEFAULT_CASES: u32 = 128;
    const CI_CASES: u32 = 32;

    fn proptest_cases() -> u32 {
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

    fn proptest_config() -> proptest::test_runner::Config {
        proptest::test_runner::Config {
            cases: proptest_cases(),
            ..Default::default()
        }
    }

    #[derive(Clone, Debug)]
    enum Op {
        Consume(u64),
        Get(Vec<u8>, Option<Vec<u8>>),
        Set(Vec<u8>, Vec<u8>),
        Remove(Vec<u8>),
    }

    fn op_strategy() -> impl Strategy<Value = Vec<Op>> {
        let key = proptest::collection::vec(any::<u8>(), 0..=16);
        let value = proptest::collection::vec(any::<u8>(), 0..=16);
        let opt_value = prop_oneof![Just(None), value.clone().prop_map(Some)];
        let op = prop_oneof![
            3 => any::<u64>().prop_map(Op::Consume),
            3 => (key.clone(), opt_value).prop_map(|(k, v)| Op::Get(k, v)),
            3 => (key.clone(), value.clone()).prop_map(|(k, v)| Op::Set(k, v)),
            2 => key.prop_map(Op::Remove),
        ];

        proptest::collection::vec(op, 0..=MAX_OPS)
    }

    fn get_gas(config: &StorageGasConfig, key: &[u8], value: &Option<Vec<u8>>) -> u64 {
        let key_len = key.len() as u64;
        let value_len = match value {
            None => 1u64,
            Some(value) => (value.len() as u64).saturating_add(1),
        };
        let total_size = key_len.saturating_add(1).saturating_add(value_len);
        config.storage_get_charge.saturating_mul(total_size)
    }

    fn set_gas(config: &StorageGasConfig, key: &[u8], value: &[u8]) -> u64 {
        let total_size = (key.len() as u64)
            .saturating_add(1)
            .saturating_add(value.len() as u64)
            .saturating_add(1);
        config.storage_get_charge.saturating_mul(total_size)
    }

    fn remove_gas(config: &StorageGasConfig, key: &[u8]) -> u64 {
        let total_size = (key.len() as u64).saturating_add(1);
        config.storage_get_charge.saturating_mul(total_size)
    }

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

    #[test]
    fn test_saturating_add_prevents_overflow() {
        let config = StorageGasConfig {
            storage_get_charge: 1,
            storage_set_charge: 1,
            storage_remove_charge: 1,
        };
        // Set limit lower than MAX to properly test
        let mut gc = GasCounter::finite(1000, config);

        // Consume most of the gas
        gc.consume_gas(990).unwrap();
        assert_eq!(gc.gas_used(), 990);

        // This would overflow with regular addition if we tried to add u64::MAX
        // With saturating add, it should just hit the limit
        let result = gc.consume_gas(u64::MAX);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);

        // Gas used should still be at 990 since the operation failed
        assert_eq!(gc.gas_used(), 990);
    }

    #[test]
    fn test_saturating_mul_prevents_overflow() {
        let config = StorageGasConfig {
            storage_get_charge: u64::MAX / 2,
            storage_set_charge: u64::MAX / 2,
            storage_remove_charge: u64::MAX / 2,
        };
        // Use a reasonable limit to test overflow behavior
        let mut gc = GasCounter::finite(1000, config);

        // Create a key that would cause overflow with regular multiplication
        // With charge = u64::MAX/2 and key length = 10, multiplication would overflow
        let key = vec![0u8; 10];
        let value = Message::from_bytes(vec![0u8; 10]);

        // These operations would panic with regular arithmetic due to overflow
        // With saturating arithmetic, they should fail gracefully due to gas limit
        let result = gc.consume_get_gas(&key, &Some(value.clone()));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);

        let result = gc.consume_set_gas(&key, &value);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);

        let result = gc.consume_remove_gas(&key);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);

        // Verify no gas was consumed since all operations failed
        assert_eq!(gc.gas_used(), 0);
    }

    #[test]
    fn test_gas_calculation_accuracy() {
        let config = StorageGasConfig {
            storage_get_charge: 2,
            storage_set_charge: 3,
            storage_remove_charge: 1,
        };
        let mut gc = GasCounter::finite(1000, config);

        // Test get gas calculation
        // key=3 bytes, value=5 bytes: (3+1) + (5+1) = 10 * 2 = 20
        gc.consume_get_gas(b"abc", &Some(Message::from_bytes(b"hello".to_vec())))
            .unwrap();
        assert_eq!(gc.gas_used(), 20);

        // Test set gas calculation (using storage_get_charge as per implementation)
        // key=4 bytes, value=6 bytes: (4+1) + (6+1) = 12 * 2 = 24
        gc.consume_set_gas(b"test", &Message::from_bytes(b"world!".to_vec()))
            .unwrap();
        assert_eq!(gc.gas_used(), 44); // 20 + 24

        // Test remove gas calculation
        // key=2 bytes: (2+1) = 3 * 2 = 6
        gc.consume_remove_gas(b"hi").unwrap();
        assert_eq!(gc.gas_used(), 50); // 44 + 6
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn prop_gas_counter_model(
            gas_limit in 0u64..=2_000,
            storage_get_charge in 0u64..=50,
            storage_set_charge in 0u64..=50,
            storage_remove_charge in 0u64..=50,
            ops in op_strategy(),
        ) {
            let config = StorageGasConfig {
                storage_get_charge,
                storage_set_charge,
                storage_remove_charge,
            };
            let mut gc = GasCounter::finite(gas_limit, config.clone());
            let mut expected_used = 0u64;

            for op in ops {
                match op {
                    Op::Consume(gas) => {
                        let expected_next = expected_used.saturating_add(gas);
                        let result = gc.consume_gas(gas);
                        if expected_next > gas_limit {
                            prop_assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        } else {
                            result.unwrap();
                            expected_used = expected_next;
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        }
                    }
                    Op::Get(key, value) => {
                        let gas = get_gas(&config, &key, &value);
                        let expected_next = expected_used.saturating_add(gas);
                        let msg = value.clone().map(Message::from_bytes);
                        let result = gc.consume_get_gas(&key, &msg);
                        if expected_next > gas_limit {
                            prop_assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        } else {
                            result.unwrap();
                            expected_used = expected_next;
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        }
                    }
                    Op::Set(key, value) => {
                        let gas = set_gas(&config, &key, &value);
                        let expected_next = expected_used.saturating_add(gas);
                        let result = gc.consume_set_gas(&key, &Message::from_bytes(value));
                        if expected_next > gas_limit {
                            prop_assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        } else {
                            result.unwrap();
                            expected_used = expected_next;
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        }
                    }
                    Op::Remove(key) => {
                        let gas = remove_gas(&config, &key);
                        let expected_next = expected_used.saturating_add(gas);
                        let result = gc.consume_remove_gas(&key);
                        if expected_next > gas_limit {
                            prop_assert_eq!(result.unwrap_err(), ERR_OUT_OF_GAS);
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        } else {
                            result.unwrap();
                            expected_used = expected_next;
                            prop_assert_eq!(gc.gas_used(), expected_used);
                        }
                    }
                }
            }
        }
    }
}

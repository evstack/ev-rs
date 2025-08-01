use evolve_macros::account_impl;

#[account_impl(GasService)]
pub mod account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{define_error, Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_macros::{exec, init, query};

    define_error!(ERR_OUT_OF_GAS, 0x1, "out of gas");

    #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
    pub struct StorageGasConfig {
        pub storage_get_charge: u64,
        pub storage_set_charge: u64,
        pub storage_remove_charge: u64,
    }

    pub struct GasService {
        pub storage_gas_config: Item<StorageGasConfig>,
    }

    impl Default for GasService {
        fn default() -> Self {
            Self::new()
        }
    }

    impl GasService {
        /// Inits gas.
        pub const fn new() -> Self {
            Self {
                storage_gas_config: Item::new(0),
            }
        }
        /// Only runtime can init this.
        #[init]
        pub fn initialize(
            &self,
            config: StorageGasConfig,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.update_storage_gas_config(config, env)?;
            Ok(())
        }

        /// Only runtime can update the storage gas config.
        #[exec]
        pub fn update_storage_gas_config(
            &self,
            new_config: StorageGasConfig,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            self.storage_gas_config.set(&new_config, env)?;
            Ok(())
        }

        /// Reads the storage gas config.
        #[query]
        pub fn get_storage_gas_config(&self, env: &dyn Environment) -> SdkResult<StorageGasConfig> {
            Ok(self.storage_gas_config.may_get(env)?.unwrap())
        }
    }
}

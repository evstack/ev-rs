use evolve_core::account_impl;

#[account_impl(EoaAccount)]
pub mod eoa_account {
    use crate::TestTx;
    use evolve_authentication::auth_interface::AuthenticationInterface;
    use evolve_collections::item::Item;
    use evolve_core::{Environment, Message, SdkResult};
    use evolve_macros::{exec, init};

    pub struct EoaAccount {
        pub nonce: Item<u64>,
    }

    impl Default for EoaAccount {
        fn default() -> Self {
            Self::new()
        }
    }

    impl EoaAccount {
        pub const fn new() -> EoaAccount {
            EoaAccount {
                nonce: Item::new(0),
            }
        }
        #[init]
        pub fn initialize(&self, env: &mut dyn Environment) -> SdkResult<()> {
            self.nonce.set(&0, env)?;
            Ok(())
        }
    }

    impl AuthenticationInterface for EoaAccount {
        #[exec]
        fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
            let _tx: TestTx = tx.get()?;
            // TODO: add on test tx some mock sig things..
            // increase nonce
            self.nonce.update(|v| Ok(v.unwrap_or_default() + 1), env)?;
            Ok(())
        }
    }
}

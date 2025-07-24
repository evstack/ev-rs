#[allow(dead_code, clippy::module_inception)]
mod example {
    use crate::auth_interface::AuthenticationInterface;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::{ERR_UNAUTHORIZED, Environment, Message, SdkResult};

    #[derive(BorshSerialize, BorshDeserialize, Clone)]
    struct ExampleTx {
        nonce: u64,
        sig: [u8; 64],
    }

    struct ExampleEoaAccount {
        nonce: Item<u64>,
    }

    impl AuthenticationInterface for ExampleEoaAccount {
        fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
            let tx: ExampleTx = tx.get()?;
            // ensure nonce
            if tx.nonce != self.nonce.get(env)? {
                return Err(ERR_UNAUTHORIZED);
            }
            // ensure sig
            // etc.
            Ok(())
        }
    }
}

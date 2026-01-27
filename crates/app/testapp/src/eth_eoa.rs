//! Ethereum-compatible EOA account for mempool transactions.
//!
//! This account type authenticates `TxContext` by verifying that the
//! recovered sender address matches the account's Ethereum address.
//!
//! # Authentication Model
//!
//! EOAs are responsible only for authentication - verifying that the transaction
//! was signed by the owner of this account. The actual transaction dispatch goes
//! to the recipient account specified in the transaction's `to` field.
//!
//! This separation of concerns means:
//! - EOAs own their authentication logic (signature verification, nonce tracking)
//! - Recipient accounts implement business logic (token transfers, order placement, etc.)
//! - The STF orchestrates: authenticate sender → dispatch to recipient

use evolve_core::account_impl;

#[account_impl(EthEoaAccount)]
pub mod eth_eoa_account {
    use evolve_authentication::auth_interface::AuthenticationInterface;
    use evolve_collections::item::Item;
    use evolve_core::{Environment, Message, SdkResult};
    use evolve_macros::{exec, init, query};
    use evolve_mempool::TxContext;

    /// An Ethereum-compatible externally owned account.
    ///
    /// This account type:
    /// - Authenticates transactions by verifying the recovered sender
    /// - Tracks nonce for replay protection
    /// - Stores the expected Ethereum address
    ///
    /// Transactions are dispatched to recipient accounts based on the `to` address.
    pub struct EthEoaAccount {
        /// The nonce, incremented after each successful transaction.
        pub nonce: Item<u64>,
        /// The Ethereum address (20 bytes) that owns this account.
        pub eth_address: Item<[u8; 20]>,
    }

    impl Default for EthEoaAccount {
        fn default() -> Self {
            Self::new()
        }
    }

    impl EthEoaAccount {
        pub const fn new() -> Self {
            Self {
                nonce: Item::new(0),
                eth_address: Item::new(1),
            }
        }

        /// Initialize the account with an Ethereum address.
        #[init]
        pub fn initialize(
            &self,
            eth_address: [u8; 20],
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.nonce.set(&0, env)?;
            self.eth_address.set(&eth_address, env)?;
            Ok(())
        }

        /// Query the current nonce.
        #[query]
        pub fn get_nonce(&self, env: &mut dyn evolve_core::EnvironmentQuery) -> SdkResult<u64> {
            Ok(self.nonce.may_get(env)?.unwrap_or(0))
        }

        /// Query the Ethereum address.
        #[query]
        pub fn get_eth_address(
            &self,
            env: &mut dyn evolve_core::EnvironmentQuery,
        ) -> SdkResult<[u8; 20]> {
            Ok(self.eth_address.may_get(env)?.unwrap_or([0u8; 20]))
        }
    }

    impl AuthenticationInterface for EthEoaAccount {
        /// Authenticate a transaction.
        ///
        /// For TxContext (Ethereum transactions):
        /// 1. Verifies the recovered sender address matches this account's address
        /// 2. Increments nonce
        ///
        /// For other transaction types:
        /// - Just increments nonce (test mode, no signature verification)
        #[exec]
        fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
            // Try to decode as TxContext for full verification
            if let Ok(mempool_tx) = tx.get::<TxContext>() {
                // Get the sender address from the recovered signature
                let sender_bytes: [u8; 20] = mempool_tx.sender_address().into();

                // Verify the sender matches this account's address
                let expected_address = self.eth_address.may_get(env)?.unwrap_or([0u8; 20]);
                if sender_bytes != expected_address {
                    return Err(evolve_core::ErrorCode::new(0x51)); // Sender mismatch
                }
            }
            // For other tx types, skip verification (test mode)

            // Increment nonce
            self.nonce.update(|v| Ok(v.unwrap_or_default() + 1), env)?;

            Ok(())
        }
    }
}

#[cfg(test)]
mod example;

use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{ERR_UNKNOWN_FUNCTION, Environment, ErrorCode, Message, SdkResult};
use evolve_macros::account_impl;
use evolve_server_core::{Transaction, TxValidator};
use std::marker::PhantomData;

pub const ERR_NOT_EOA: ErrorCode = ErrorCode::new(4111, "not an externally owned account");

#[account_impl(AuthenticationInterface)]
pub mod auth_interface {
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::exec;

    pub trait AuthenticationInterface {
        #[exec]
        fn authenticate(
            &self,
            // This is a generic message to avoid to parametrize accounts over the TX.
            // This would also make it possible for the account to support multiple Tx types.
            tx: evolve_core::Message,
            env: &mut dyn Environment,
        ) -> SdkResult<()>;
    }
}

/// Implements the TxValidator for an account that can be authenticated during tx execution.
#[derive(Default)]
pub struct AuthenticationTxValidator<Tx>(PhantomData<Tx>);

impl<Tx: Transaction + Decodable + Encodable> AuthenticationTxValidator<Tx> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Transaction + Clone + Encodable + Decodable> TxValidator<T>
    for AuthenticationTxValidator<T>
{
    fn validate_tx(&self, tx: &T, env: &mut dyn Environment) -> SdkResult<()> {
        // trigger authentication
        auth_interface::AuthenticationInterfaceRef::new(tx.sender())
            .authenticate(Message::new(tx)?, env)
            .map_err(|e| {
                if e == ERR_UNKNOWN_FUNCTION {
                    return ERR_NOT_EOA;
                }
                e
            })
    }
}

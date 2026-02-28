#[cfg(test)]
mod example;

use evolve_core::account_impl;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{ERR_UNKNOWN_FUNCTION, Environment, SdkResult, define_error};
use evolve_stf_traits::{AuthenticationPayload, Transaction, TxValidator};
use std::marker::PhantomData;

define_error!(ERR_NOT_EOA, 0x41, "not an externally owned account");

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

impl<Tx: Transaction + AuthenticationPayload + Decodable + Encodable>
    AuthenticationTxValidator<Tx>
{
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Transaction + AuthenticationPayload + Clone + Encodable + Decodable> TxValidator<T>
    for AuthenticationTxValidator<T>
{
    fn validate_tx(&self, tx: &T, env: &mut dyn Environment) -> SdkResult<()> {
        let sender_account = tx.resolve_sender_account(env)?;
        // trigger authentication
        auth_interface::AuthenticationInterfaceRef::new(sender_account)
            .authenticate(tx.authentication_payload()?, env)
            .map_err(|e| {
                if e == ERR_UNKNOWN_FUNCTION {
                    return ERR_NOT_EOA;
                }
                e
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::{
        AccountId, BlockContext, Environment, EnvironmentQuery, ErrorCode, FungibleAsset,
        InvokeRequest, InvokeResponse, Message,
    };

    #[derive(Clone, BorshSerialize, BorshDeserialize)]
    struct DummyInvoke {
        value: u8,
    }

    impl evolve_core::InvokableMessage for DummyInvoke {
        const FUNCTION_IDENTIFIER: u64 = 9_001;
        const FUNCTION_IDENTIFIER_NAME: &'static str = "dummy";
    }

    #[derive(Clone, BorshSerialize, BorshDeserialize)]
    struct DummyTx {
        sender: AccountId,
        recipient: AccountId,
        request: InvokeRequest,
        payload: Message,
    }

    impl Transaction for DummyTx {
        fn sender(&self) -> AccountId {
            self.sender
        }

        fn recipient(&self) -> AccountId {
            self.recipient
        }

        fn request(&self) -> &InvokeRequest {
            &self.request
        }

        fn gas_limit(&self) -> u64 {
            100_000
        }

        fn funds(&self) -> &[FungibleAsset] {
            &[]
        }

        fn compute_identifier(&self) -> [u8; 32] {
            [7u8; 32]
        }
    }

    impl AuthenticationPayload for DummyTx {
        fn authentication_payload(&self) -> SdkResult<Message> {
            Ok(self.payload.clone())
        }
    }

    struct AuthEnv {
        whoami: AccountId,
        sender: AccountId,
        auth_result: Result<(), ErrorCode>,
        last_exec_to: Option<AccountId>,
        last_exec_function: Option<String>,
    }

    impl AuthEnv {
        fn new(sender: AccountId, auth_result: Result<(), ErrorCode>) -> Self {
            Self {
                whoami: AccountId::from_u64(999),
                sender,
                auth_result,
                last_exec_to: None,
                last_exec_function: None,
            }
        }
    }

    impl EnvironmentQuery for AuthEnv {
        fn whoami(&self) -> AccountId {
            self.whoami
        }

        fn sender(&self) -> AccountId {
            self.sender
        }

        fn funds(&self) -> &[FungibleAsset] {
            &[]
        }

        fn block(&self) -> BlockContext {
            BlockContext::new(0, 0)
        }

        fn do_query(&mut self, _to: AccountId, _data: &InvokeRequest) -> SdkResult<InvokeResponse> {
            Err(ERR_UNKNOWN_FUNCTION)
        }
    }

    impl Environment for AuthEnv {
        fn do_exec(
            &mut self,
            to: AccountId,
            data: &InvokeRequest,
            _funds: Vec<FungibleAsset>,
        ) -> SdkResult<InvokeResponse> {
            self.last_exec_to = Some(to);
            self.last_exec_function = Some(data.human_name().to_string());

            match self.auth_result {
                Ok(()) => InvokeResponse::new(&()),
                Err(e) => Err(e),
            }
        }

        fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
            Ok(())
        }

        fn unique_id(&mut self) -> SdkResult<[u8; 32]> {
            Ok([0u8; 32])
        }
    }

    fn make_tx(sender: AccountId) -> DummyTx {
        DummyTx {
            sender,
            recipient: AccountId::from_u64(200),
            request: InvokeRequest::new(&DummyInvoke { value: 1 }).expect("request should encode"),
            payload: Message::new(&DummyInvoke { value: 42 }).expect("payload should encode"),
        }
    }

    #[test]
    fn validate_tx_dispatches_auth_to_sender_account() {
        let sender = AccountId::from_u64(100);
        let tx = make_tx(sender);
        let validator = AuthenticationTxValidator::<DummyTx>::new();
        let mut env = AuthEnv::new(sender, Ok(()));

        validator
            .validate_tx(&tx, &mut env)
            .expect("auth should succeed");

        assert_eq!(env.last_exec_to, Some(sender));
        assert_eq!(env.last_exec_function.as_deref(), Some("authenticate"));
    }

    #[test]
    fn validate_tx_maps_unknown_function_to_not_eoa() {
        let sender = AccountId::from_u64(100);
        let tx = make_tx(sender);
        let validator = AuthenticationTxValidator::<DummyTx>::new();
        let mut env = AuthEnv::new(sender, Err(ERR_UNKNOWN_FUNCTION));

        let err = validator
            .validate_tx(&tx, &mut env)
            .expect_err("unknown function should map to not-eoa");

        assert_eq!(err, ERR_NOT_EOA);
    }

    #[test]
    fn validate_tx_preserves_non_unknown_errors() {
        let sender = AccountId::from_u64(100);
        let tx = make_tx(sender);
        let validator = AuthenticationTxValidator::<DummyTx>::new();
        let mut env = AuthEnv::new(sender, Err(evolve_core::ERR_UNAUTHORIZED));

        let err = validator
            .validate_tx(&tx, &mut env)
            .expect_err("unauthorized should pass through");

        assert_eq!(err, evolve_core::ERR_UNAUTHORIZED);
    }
}

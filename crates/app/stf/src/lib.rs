//! # State Transition Function (STF) Module
//!
//! This module provides the core state transition functionality for the Evolve blockchain.
//! It manages transaction execution, block processing, and state management.
//!
//! ## Core Components
//!
//! - `Stf`: Main state transition function that processes blocks and transactions
//! - `Invoker`: Execution context for account operations
//! - Error handling and validation
//! - Gas metering and resource management
//!
//! ## Gas Configuration
//!
//! Gas costs for storage operations are configured via [`StorageGasConfig`] when
//! constructing the STF. See the [`gas`] module for details.

// Instant is used for performance metrics, not consensus-affecting logic.
#![allow(clippy::disallowed_types)]
// Test module is in middle of file for logical grouping with model-based tests.
#![allow(clippy::items_after_test_module)]
#![cfg_attr(test, allow(clippy::indexing_slicing))]

pub mod errors;
mod execution_scope;
pub mod execution_state;
pub mod gas;
mod handlers;
mod invoker;
pub mod metrics;
pub mod results;
mod runtime_api_impl;
mod validation;

// Re-export gas types for convenience
pub use gas::{StorageGasConfig, ERR_OUT_OF_GAS};

use crate::execution_state::ExecutionState;
use crate::gas::GasCounter;
use crate::invoker::Invoker;
use crate::metrics::{BlockExecutionMetrics, TxExecutionMetrics};
use crate::results::{BlockResult, TxResult};
use evolve_core::events_api::Event;
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, InvokableMessage,
    InvokeRequest, InvokeResponse, ReadonlyKV, SdkResult,
};
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, Transaction, TxValidator as TxValidatorTrait,
};
use std::marker::PhantomData;
use std::time::Instant;

// Security limits
const MAX_CODE_ID_LENGTH: usize = 256;
const MAX_EVENT_NAME_LENGTH: usize = 128;
const MAX_EVENT_CONTENT_SIZE: usize = 64 * 1024; // 64KB

/// The main State Transition Function (STF) for the Evolve blockchain.
///
/// This struct orchestrates the execution of blocks and transactions, managing
/// the state transitions in a deterministic manner. It coordinates between
/// different components like transaction validation, block processing, and
/// post-transaction handling.
///
/// # Type Parameters
///
/// * `Tx` - Transaction type that implements the `Transaction` trait
/// * `Block` - Block type that implements the `BlockTrait`
/// * `BeginBlocker` - Handler for begin-block logic
/// * `TxValidator` - Transaction validator
/// * `EndBlocker` - Handler for end-block logic
/// * `PostTx` - Post-transaction execution handler
///
/// # Gas Configuration
///
/// The STF is constructed with a `StorageGasConfig` that defines gas costs for
/// storage operations. This configuration is fixed at construction time and
/// applies to all transactions processed by this STF instance.
///
/// To change gas configuration, construct a new STF instance with the desired
/// configuration. For runtime-configurable gas, applications can implement
/// their own governance mechanism that reconstructs the STF with new parameters.
pub struct Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx> {
    begin_blocker: BeginBlocker,
    end_blocker: EndBlocker,
    tx_validator: TxValidator,
    post_tx_handler: PostTx,
    storage_gas_config: StorageGasConfig,
    _phantoms: PhantomData<(Tx, Block)>,
}

#[cfg(test)]
mod model_tests {
    use super::*;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
    use evolve_core::storage_api::{StorageSetRequest, STORAGE_ACCOUNT_ID};
    use evolve_core::{ErrorCode, FungibleAsset, Message};
    use evolve_stf_traits::{
        AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait, EndBlocker,
        PostTxExecution, Transaction, TxValidator, WritableKV,
    };
    use hashbrown::HashMap;
    use proptest::prelude::*;

    const DEFAULT_CASES: u32 = 32;
    const CI_CASES: u32 = 8;
    const MAX_TXS: usize = 16;
    const TEST_ACCOUNT_ID: AccountId = AccountId::new(100);
    const DEFAULT_SENDER_ID: AccountId = AccountId::new(200);

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

    #[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
    struct TestMsg {
        key: Vec<u8>,
        value: Vec<u8>,
        fail_after_write: bool,
    }

    impl InvokableMessage for TestMsg {
        const FUNCTION_IDENTIFIER: u64 = 1;
        const FUNCTION_IDENTIFIER_NAME: &'static str = "test_exec";
    }

    #[derive(Clone, Debug)]
    struct TestTx {
        sender: AccountId,
        recipient: AccountId,
        request: InvokeRequest,
        gas_limit: u64,
        funds: Vec<FungibleAsset>,
        fail_validate: bool,
    }

    impl Transaction for TestTx {
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
            self.gas_limit
        }

        fn funds(&self) -> &[FungibleAsset] {
            self.funds.as_slice()
        }

        fn compute_identifier(&self) -> [u8; 32] {
            [0u8; 32]
        }
    }

    #[derive(Clone)]
    struct TestBlock {
        height: u64,
        time: u64,
        txs: Vec<TestTx>,
    }

    impl BlockTrait<TestTx> for TestBlock {
        fn context(&self) -> BlockContext {
            BlockContext::new(self.height, self.time)
        }

        fn txs(&self) -> &[TestTx] {
            &self.txs
        }
    }

    #[derive(Default)]
    struct NoopBegin;

    impl BeginBlockerTrait<TestBlock> for NoopBegin {
        fn begin_block(&self, _block: &TestBlock, _env: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct NoopEnd;

    impl EndBlocker for NoopEnd {
        fn end_block(&self, _env: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct Validator;

    impl TxValidator<TestTx> for Validator {
        fn validate_tx(&self, tx: &TestTx, _env: &mut dyn Environment) -> SdkResult<()> {
            if tx.fail_validate {
                return Err(ErrorCode::new(100));
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopPostTx;

    impl PostTxExecution<TestTx> for NoopPostTx {
        fn after_tx_executed(
            _tx: &TestTx,
            _gas_consumed: u64,
            _tx_result: &SdkResult<InvokeResponse>,
            _env: &mut dyn Environment,
        ) -> SdkResult<()> {
            Ok(())
        }
    }

    type TestStf<PostTx = NoopPostTx> =
        Stf<TestTx, TestBlock, NoopBegin, Validator, NoopEnd, PostTx>;

    #[derive(Default)]
    struct TestAccount;

    impl AccountCode for TestAccount {
        fn identifier(&self) -> String {
            "test_account".to_string()
        }

        fn schema(&self) -> evolve_core::schema::AccountSchema {
            evolve_core::schema::AccountSchema::new("TestAccount", "test_account")
        }

        fn init(
            &self,
            _env: &mut dyn Environment,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            InvokeResponse::new(&())
        }

        fn execute(
            &self,
            env: &mut dyn Environment,
            request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            let msg: TestMsg = request.get()?;

            let set = StorageSetRequest {
                key: msg.key.clone(),
                value: Message::from_bytes(msg.value.clone()),
            };
            env.do_exec(STORAGE_ACCOUNT_ID, &InvokeRequest::new(&set)?, vec![])?;

            if msg.fail_after_write {
                return Err(ErrorCode::new(200));
            }

            InvokeResponse::new(&())
        }

        fn query(
            &self,
            _env: &mut dyn EnvironmentQuery,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            InvokeResponse::new(&())
        }
    }

    struct CodeStore {
        codes: HashMap<String, Box<dyn AccountCode>>,
    }

    impl CodeStore {
        fn new() -> Self {
            Self {
                codes: HashMap::new(),
            }
        }

        fn add_code(&mut self, code: impl AccountCode + 'static) {
            self.codes.insert(code.identifier(), Box::new(code));
        }
    }

    impl AccountsCodeStorage for CodeStore {
        fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
        where
            F: FnOnce(Option<&dyn AccountCode>) -> R,
        {
            Ok(f(self.codes.get(identifier).map(|c| c.as_ref())))
        }

        fn list_identifiers(&self) -> Vec<String> {
            self.codes.keys().cloned().collect()
        }
    }

    #[derive(Default)]
    struct InMemoryStorage {
        data: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl ReadonlyKV for InMemoryStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
            Ok(self.data.get(key).cloned())
        }
    }

    impl evolve_stf_traits::WritableKV for InMemoryStorage {
        fn apply_changes(
            &mut self,
            changes: Vec<evolve_stf_traits::StateChange>,
        ) -> Result<(), ErrorCode> {
            for change in changes {
                match change {
                    evolve_stf_traits::StateChange::Set { key, value } => {
                        self.data.insert(key, value);
                    }
                    evolve_stf_traits::StateChange::Remove { key } => {
                        self.data.remove(&key);
                    }
                }
            }
            Ok(())
        }
    }

    fn account_storage_key(account: AccountId, key: &[u8]) -> Vec<u8> {
        let mut out = account.as_bytes().to_vec();
        out.extend_from_slice(key);
        out
    }

    fn account_code_key(account: AccountId) -> Vec<u8> {
        let mut out = vec![ACCOUNT_IDENTIFIER_PREFIX];
        out.extend_from_slice(&account.as_bytes());
        out
    }

    fn default_gas_config() -> StorageGasConfig {
        StorageGasConfig {
            storage_get_charge: 1,
            storage_set_charge: 1,
            storage_remove_charge: 1,
        }
    }

    fn setup_env_with_post_tx<P: PostTxExecution<TestTx>>(
        gas_config: StorageGasConfig,
        post_tx: P,
    ) -> (InMemoryStorage, CodeStore, TestStf<P>) {
        let mut storage = InMemoryStorage::default();
        let code_id = "test_account".to_string();
        storage.data.insert(
            account_code_key(TEST_ACCOUNT_ID),
            Message::new(&code_id).unwrap().into_bytes().unwrap(),
        );

        let mut codes = CodeStore::new();
        codes.add_code(TestAccount);

        let stf = Stf::new(NoopBegin, NoopEnd, Validator, post_tx, gas_config);

        (storage, codes, stf)
    }

    fn setup_default_env() -> (InMemoryStorage, CodeStore, TestStf) {
        setup_env_with_post_tx(default_gas_config(), NoopPostTx)
    }

    fn make_tx(
        sender: AccountId,
        recipient: AccountId,
        key: Vec<u8>,
        value: Vec<u8>,
        gas_limit: u64,
        fail_validate: bool,
        fail_after_write: bool,
    ) -> TestTx {
        let msg = TestMsg {
            key,
            value,
            fail_after_write,
        };
        TestTx {
            sender,
            recipient,
            request: InvokeRequest::new(&msg).unwrap(),
            gas_limit,
            funds: vec![],
            fail_validate,
        }
    }

    fn execute_and_commit<P: PostTxExecution<TestTx>>(
        stf: &TestStf<P>,
        storage: &mut InMemoryStorage,
        codes: &CodeStore,
        txs: Vec<TestTx>,
    ) -> BlockResult {
        let block = TestBlock {
            height: 1,
            time: 0,
            txs,
        };
        let (result, state) = stf.apply_block(storage, codes, &block);
        storage
            .apply_changes(state.into_changes().unwrap())
            .unwrap();
        result
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn prop_apply_block_failure_invariance(
            txs in proptest::collection::vec(
                (
                    proptest::collection::vec(any::<u8>(), 0..=16),
                    proptest::collection::vec(any::<u8>(), 0..=32),
                    0u64..=200,
                    any::<bool>(),
                    any::<bool>(),
                ),
                0..=MAX_TXS
            )
        ) {
            let test_account = AccountId::new(100);
            let sender = AccountId::new(200);

            let mut storage = InMemoryStorage::default();
            let mut codes = CodeStore::new();
            codes.add_code(TestAccount);

            let gas_config = StorageGasConfig {
                storage_get_charge: 3,
                storage_set_charge: 3,
                storage_remove_charge: 3,
            };

            let code_id = "test_account".to_string();
            storage.data.insert(
                account_code_key(test_account),
                Message::new(&code_id).unwrap().into_bytes().unwrap(),
            );

            let stf = Stf::new(
                NoopBegin,
                NoopEnd,
                Validator,
                NoopPostTx,
                gas_config.clone(),
            );

            let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
            let mut block_txs: Vec<TestTx> = Vec::new();

            for (key, value, gas_limit, fail_validate, fail_after_write) in txs {
                let msg = TestMsg {
                    key: key.clone(),
                    value: value.clone(),
                    fail_after_write,
                };
                let req = InvokeRequest::new(&msg).unwrap();
                block_txs.push(TestTx {
                    sender,
                    recipient: test_account,
                    request: req,
                    gas_limit,
                    funds: vec![],
                    fail_validate,
                });

                let storage_key = account_storage_key(test_account, &key);
                // Use storage_set_charge for set operations
                let gas_needed = gas_config.storage_set_charge.saturating_mul(
                    (storage_key.len() as u64)
                        .saturating_add(1)
                        .saturating_add(value.len() as u64)
                        .saturating_add(1),
                );
                let out_of_gas = gas_needed > gas_limit;

                if !fail_validate && !fail_after_write && !out_of_gas {
                    model.insert(storage_key, value);
                }
            }

            let block = TestBlock { height: 1, time: 0, txs: block_txs };
            let (result, state) = stf.apply_block(&storage, &codes, &block);
            let changes = state.into_changes().unwrap();
            storage.apply_changes(changes).unwrap();

            for (idx, tx_result) in result.tx_results.iter().enumerate() {
                let tx = &block.txs[idx];
                let msg: TestMsg = tx.request.get().unwrap();
                let storage_key = account_storage_key(tx.recipient, &msg.key);
                // Use storage_set_charge for set operations
                let gas_needed = gas_config.storage_set_charge.saturating_mul(
                    (storage_key.len() as u64)
                        .saturating_add(1)
                        .saturating_add(msg.value.len() as u64)
                        .saturating_add(1),
                );
                let out_of_gas = gas_needed > tx.gas_limit;

                if tx.fail_validate {
                    prop_assert!(tx_result.response.is_err());
                } else if out_of_gas {
                    match &tx_result.response {
                        Ok(_) => prop_assert!(false, "expected out of gas"),
                        Err(err) => prop_assert_eq!(*err, crate::gas::ERR_OUT_OF_GAS),
                    }
                } else if msg.fail_after_write {
                    prop_assert!(tx_result.response.is_err());
                } else {
                    prop_assert!(tx_result.response.is_ok());
                }
            }

            for (key, value) in model {
                let actual = storage.get(&key).unwrap();
                prop_assert_eq!(actual, Some(value));
            }
        }

        /// Test determinism: same block on same state produces identical results
        #[test]
        fn prop_determinism(
            txs in proptest::collection::vec(
                (
                    proptest::collection::vec(any::<u8>(), 1..=8),
                    proptest::collection::vec(any::<u8>(), 1..=16),
                ),
                1..=8
            )
        ) {
            let test_account = AccountId::new(100);
            let sender = AccountId::new(200);

            // Setup storage with account code
            let mut storage1 = InMemoryStorage::default();
            let gas_config = StorageGasConfig {
                storage_get_charge: 1,
                storage_set_charge: 1,
                storage_remove_charge: 1,
            };
            let code_id = "test_account".to_string();
            storage1.data.insert(
                account_code_key(test_account),
                Message::new(&code_id).unwrap().into_bytes().unwrap(),
            );

            // Clone storage for second run
            let storage2 = InMemoryStorage {
                data: storage1.data.clone(),
            };

            let mut codes = CodeStore::new();
            codes.add_code(TestAccount);

            let stf = Stf::new(
                NoopBegin,
                NoopEnd,
                Validator,
                NoopPostTx,
                gas_config,
            );

            // Build block
            let block_txs: Vec<TestTx> = txs.iter().map(|(key, value)| {
                let msg = TestMsg {
                    key: key.clone(),
                    value: value.clone(),
                    fail_after_write: false,
                };
                TestTx {
                    sender,
                    recipient: test_account,
                    request: InvokeRequest::new(&msg).unwrap(),
                    gas_limit: 10000,
                    funds: vec![],
                    fail_validate: false,
                }
            }).collect();

            let block = TestBlock { height: 1, time: 0, txs: block_txs };

            // Run 1
            let (result1, state1) = stf.apply_block(&storage1, &codes, &block);
            let changes1 = state1.into_changes().unwrap();

            // Run 2
            let (result2, state2) = stf.apply_block(&storage2, &codes, &block);
            let changes2 = state2.into_changes().unwrap();

            // Verify identical results
            prop_assert_eq!(result1.tx_results.len(), result2.tx_results.len());
            for (r1, r2) in result1.tx_results.iter().zip(result2.tx_results.iter()) {
                prop_assert_eq!(r1.gas_used, r2.gas_used);
                prop_assert_eq!(r1.response.is_ok(), r2.response.is_ok());
                prop_assert_eq!(r1.events.len(), r2.events.len());
            }

            // Sort changes for comparison (HashMap iteration order is non-deterministic)
            let mut sorted1: Vec<_> = changes1.iter().map(|c| match c {
                evolve_stf_traits::StateChange::Set { key, value } => (key.clone(), Some(value.clone())),
                evolve_stf_traits::StateChange::Remove { key } => (key.clone(), None),
            }).collect();
            sorted1.sort_by(|a, b| a.0.cmp(&b.0));

            let mut sorted2: Vec<_> = changes2.iter().map(|c| match c {
                evolve_stf_traits::StateChange::Set { key, value } => (key.clone(), Some(value.clone())),
                evolve_stf_traits::StateChange::Remove { key } => (key.clone(), None),
            }).collect();
            sorted2.sort_by(|a, b| a.0.cmp(&b.0));

            prop_assert_eq!(sorted1, sorted2, "State changes must be identical");
        }
    }

    // ========== Non-proptest tests for specific scenarios ==========

    /// Test that call depth limit is enforced
    #[test]
    fn test_call_depth_exhaustion() {
        use crate::errors::ERR_CALL_DEPTH_EXCEEDED;

        let recursive_account = AccountId::new(100);
        let sender = AccountId::new(200);

        let mut storage = InMemoryStorage::default();
        let gas_config = StorageGasConfig {
            storage_get_charge: 1,
            storage_set_charge: 1,
            storage_remove_charge: 1,
        };

        // RecursiveAccount calls itself until depth limit
        struct RecursiveAccount;

        #[derive(Clone, BorshSerialize, BorshDeserialize)]
        struct RecurseMsg {
            depth: u16,
        }

        impl InvokableMessage for RecurseMsg {
            const FUNCTION_IDENTIFIER: u64 = 1;
            const FUNCTION_IDENTIFIER_NAME: &'static str = "recurse";
        }

        impl AccountCode for RecursiveAccount {
            fn identifier(&self) -> String {
                "recursive".to_string()
            }

            fn schema(&self) -> evolve_core::schema::AccountSchema {
                evolve_core::schema::AccountSchema::new("RecursiveAccount", "recursive")
            }

            fn init(
                &self,
                _env: &mut dyn Environment,
                _req: &InvokeRequest,
            ) -> SdkResult<InvokeResponse> {
                InvokeResponse::new(&())
            }

            fn execute(
                &self,
                env: &mut dyn Environment,
                req: &InvokeRequest,
            ) -> SdkResult<InvokeResponse> {
                let msg: RecurseMsg = req.get()?;
                // Call self recursively
                let next_msg = RecurseMsg {
                    depth: msg.depth + 1,
                };
                env.do_exec(env.whoami(), &InvokeRequest::new(&next_msg)?, vec![])?;
                InvokeResponse::new(&msg.depth)
            }

            fn query(
                &self,
                _env: &mut dyn EnvironmentQuery,
                _req: &InvokeRequest,
            ) -> SdkResult<InvokeResponse> {
                InvokeResponse::new(&())
            }
        }

        let code_id = "recursive".to_string();
        storage.data.insert(
            account_code_key(recursive_account),
            Message::new(&code_id).unwrap().into_bytes().unwrap(),
        );

        let mut codes = CodeStore::new();
        codes.add_code(RecursiveAccount);

        let stf = Stf::new(NoopBegin, NoopEnd, Validator, NoopPostTx, gas_config);

        let msg = RecurseMsg { depth: 0 };
        let tx = TestTx {
            sender,
            recipient: recursive_account,
            request: InvokeRequest::new(&msg).unwrap(),
            gas_limit: 1_000_000, // High gas limit to ensure we hit depth, not gas
            funds: vec![],
            fail_validate: false,
        };

        let block = TestBlock {
            height: 1,
            time: 0,
            txs: vec![tx],
        };
        let (result, _state) = stf.apply_block(&storage, &codes, &block);

        assert_eq!(result.tx_results.len(), 1);
        let tx_result = &result.tx_results[0];
        assert!(tx_result.response.is_err(), "Expected call depth error");
        assert_eq!(
            tx_result.response.as_ref().unwrap_err(),
            &ERR_CALL_DEPTH_EXCEEDED,
            "Expected ERR_CALL_DEPTH_EXCEEDED"
        );
    }

    /// Test that post-tx handler can reject transactions
    #[test]
    fn test_post_tx_handler_rejection() {
        // Post-tx handler that rejects based on sender marker.
        struct RejectMarkedPostTx;
        const ERR_REJECTED_BY_POST_TX: ErrorCode = ErrorCode::new(999);

        impl PostTxExecution<TestTx> for RejectMarkedPostTx {
            fn after_tx_executed(
                tx: &TestTx,
                _gas_consumed: u64,
                tx_result: &SdkResult<InvokeResponse>,
                _env: &mut dyn Environment,
            ) -> SdkResult<()> {
                // Reject successful txs where sender has a specific pattern
                // (Using sender ID as a marker for simplicity)
                if tx_result.is_ok() && tx.sender == AccountId::new(201) {
                    return Err(ERR_REJECTED_BY_POST_TX);
                }
                Ok(())
            }
        }

        let (mut storage, codes, stf) = setup_env_with_post_tx(
            StorageGasConfig {
                storage_get_charge: 10,
                storage_set_charge: 10,
                storage_remove_charge: 10,
            },
            RejectMarkedPostTx,
        );

        let msg = TestMsg {
            key: vec![1],
            value: vec![1],
            fail_after_write: false,
        };

        // Tx from sender 200 should succeed
        let tx_normal = TestTx {
            sender: AccountId::new(200),
            recipient: TEST_ACCOUNT_ID,
            request: InvokeRequest::new(&msg).unwrap(),
            gas_limit: 10000,
            funds: vec![],
            fail_validate: false,
        };

        // Tx from sender 201 should be rejected by post-tx handler
        let tx_marked = TestTx {
            sender: AccountId::new(201),
            recipient: TEST_ACCOUNT_ID,
            request: InvokeRequest::new(&msg).unwrap(),
            gas_limit: 10000,
            funds: vec![],
            fail_validate: false,
        };

        let result = execute_and_commit(&stf, &mut storage, &codes, vec![tx_normal, tx_marked]);

        assert_eq!(result.tx_results.len(), 2);

        // Normal tx should succeed
        assert!(
            result.tx_results[0].response.is_ok(),
            "Normal tx should succeed"
        );

        // Marked tx should be rejected by post-tx handler
        assert!(
            result.tx_results[1].response.is_err(),
            "Marked tx should be rejected by post-tx handler"
        );
        assert_eq!(
            result.tx_results[1].response.as_ref().unwrap_err(),
            &ERR_REJECTED_BY_POST_TX,
            "Expected ERR_REJECTED_BY_POST_TX from post-tx handler"
        );
    }

    /// Test transaction-level rollback behavior against committed storage.
    /// Only successful txs should produce persistent state changes.
    #[test]
    fn test_only_successful_transactions_commit_state() {
        let (mut storage, codes, stf) = setup_default_env();

        let txs = vec![
            make_tx(
                DEFAULT_SENDER_ID,
                TEST_ACCOUNT_ID,
                vec![0],
                vec![100],
                10_000,
                false,
                false,
            ),
            make_tx(
                DEFAULT_SENDER_ID,
                TEST_ACCOUNT_ID,
                vec![1],
                vec![101],
                10_000,
                true,
                false,
            ),
            make_tx(
                DEFAULT_SENDER_ID,
                TEST_ACCOUNT_ID,
                vec![2],
                vec![102],
                10_000,
                false,
                true,
            ),
            make_tx(
                DEFAULT_SENDER_ID,
                TEST_ACCOUNT_ID,
                vec![3],
                vec![103],
                1,
                false,
                false,
            ),
        ];

        let result = execute_and_commit(&stf, &mut storage, &codes, txs);
        assert_eq!(result.tx_results.len(), 4);
        assert!(
            result.tx_results[0].response.is_ok(),
            "first tx should succeed"
        );
        assert!(
            result.tx_results[1].response.is_err(),
            "validation failure should fail tx"
        );
        assert!(
            result.tx_results[2].response.is_err(),
            "execution failure should fail tx"
        );
        assert_eq!(
            result.tx_results[3].response.as_ref().unwrap_err(),
            &crate::gas::ERR_OUT_OF_GAS,
            "out-of-gas tx should fail with ERR_OUT_OF_GAS"
        );

        let key_ok = account_storage_key(TEST_ACCOUNT_ID, &[0]);
        let key_validate_fail = account_storage_key(TEST_ACCOUNT_ID, &[1]);
        let key_exec_fail = account_storage_key(TEST_ACCOUNT_ID, &[2]);
        let key_out_of_gas = account_storage_key(TEST_ACCOUNT_ID, &[3]);

        assert_eq!(storage.get(&key_ok).unwrap(), Some(vec![100]));
        assert_eq!(storage.get(&key_validate_fail).unwrap(), None);
        assert_eq!(storage.get(&key_exec_fail).unwrap(), None);
        assert_eq!(storage.get(&key_out_of_gas).unwrap(), None);
    }
}

impl<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
    Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    BeginBlocker: BeginBlockerTrait<Block>,
    TxValidator: TxValidatorTrait<Tx>,
    EndBlocker: EndBlockerTrait,
    PostTx: PostTxExecution<Tx>,
{
    /// Creates a new STF instance with the provided components.
    ///
    /// # Arguments
    ///
    /// * `begin_blocker` - Handler for begin-block processing
    /// * `end_blocker` - Handler for end-block processing
    /// * `tx_validator` - Transaction validator
    /// * `post_tx_handler` - Post-transaction execution handler
    /// * `storage_gas_config` - Configuration for storage operation gas costs
    ///
    /// # Example
    ///
    /// ```text
    /// use evolve_stf::{Stf, StorageGasConfig};
    ///
    /// let stf = Stf::new(
    ///     begin_blocker,
    ///     end_blocker,
    ///     tx_validator,
    ///     post_tx_handler,
    ///     StorageGasConfig::default(),
    /// );
    /// ```
    pub const fn new(
        begin_blocker: BeginBlocker,
        end_blocker: EndBlocker,
        tx_validator: TxValidator,
        post_tx_handler: PostTx,
        storage_gas_config: StorageGasConfig,
    ) -> Self {
        Self {
            begin_blocker,
            end_blocker,
            tx_validator,
            post_tx_handler,
            storage_gas_config,
            _phantoms: PhantomData,
        }
    }
    /// Executes the given closure with system-level privileges.
    ///
    /// This method is intended for genesis initialization and system-level operations
    /// that need to bypass normal transaction flow. Unlike a traditional "sudo" model,
    /// this is not accessible during normal runtime - it can only be invoked by the
    /// consensus engine during specific lifecycle events (genesis, upgrades).
    ///
    /// # Use Cases
    ///
    /// - Genesis account creation and initialization
    /// - System parameter configuration
    /// - Protocol upgrades
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `execution_height` - Block height for execution context
    /// * `execution_time` - Block time for execution context
    /// * `action` - Closure to execute with system privileges
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the result of the action and the execution state.
    pub fn system_exec<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: BlockContext,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let mut execution_state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let resp = {
            let mut ctx = Invoker::new_for_begin_block(
                &mut execution_state,
                account_codes,
                &mut gas_counter,
                block,
            );
            action(&mut ctx)?
        };
        execution_state.pop_events();
        Ok((resp, execution_state))
    }
    /// Executes the given closure impersonating the specified account with system privileges.
    ///
    /// This method allows executing operations as if they were initiated by a specific account,
    /// bypassing normal authentication. It is only available in test builds or with the
    /// `testing` feature enabled for security reasons.
    ///
    /// # Security Note
    ///
    /// This is a testing utility and should never be exposed in production builds.
    /// It exists to facilitate integration testing where simulating user actions
    /// is necessary without going through full transaction signing.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `execution_height` - Block height for execution context
    /// * `execution_time` - Block time for execution context
    /// * `impersonate` - Account ID to impersonate
    /// * `action` - Closure to execute with the specified account privileges
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the result of the action and the execution state.
    #[cfg(any(test, feature = "testing"))]
    pub fn system_exec_as<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: BlockContext,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let mut execution_state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let resp = {
            let mut ctx = Invoker::new_for_begin_block(
                &mut execution_state,
                account_codes,
                &mut gas_counter,
                block,
            );
            ctx.whoami = impersonate;
            action(&mut ctx)?
        };
        execution_state.pop_events();
        Ok((resp, execution_state))
    }
    /// Applies a complete block to the current state.
    ///
    /// This is the main entry point for block processing. It executes the complete
    /// block lifecycle: begin-block, transaction processing, and end-block.
    ///
    /// Transactions are processed until the block gas limit is reached. Any
    /// transactions that would exceed the limit are skipped (not executed).
    /// The `BlockResult.txs_skipped` field indicates how many were skipped.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `block` - Block to process
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the block execution results and the updated execution state.
    pub fn apply_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: &Block,
    ) -> (BlockResult, ExecutionState<'a, S>) {
        let mut state = ExecutionState::new(storage);

        // run begin blocker.
        let begin_block_events = self.do_begin_block(&mut state, account_codes, block);

        // apply txs with gas limit enforcement.
        let txs = block.txs();
        let block_ctx = block.context();
        let block_gas_limit = block.gas_limit();
        let mut tx_results = Vec::with_capacity(txs.len());
        let mut cumulative_gas: u64 = 0;
        let mut txs_skipped: usize = 0;

        for tx in txs {
            // Check if adding this tx would exceed block gas limit.
            // We use tx.gas_limit() as an upper bound estimate before execution.
            // This prevents starting execution of a tx that definitely won't fit.
            if cumulative_gas.saturating_add(tx.gas_limit()) > block_gas_limit {
                txs_skipped += 1;
                continue;
            }

            // apply tx
            let tx_result: TxResult = self.apply_tx(
                &mut state,
                account_codes,
                tx,
                self.storage_gas_config.clone(),
                block_ctx,
            );

            cumulative_gas = cumulative_gas.saturating_add(tx_result.gas_used);
            tx_results.push(tx_result);

            // If we've hit the limit after execution, stop processing more txs.
            // This handles the case where actual gas used is less than gas_limit.
            if cumulative_gas >= block_gas_limit {
                txs_skipped += txs.len() - tx_results.len();
                break;
            }
        }

        let end_block_events = self.do_end_block(&mut state, account_codes, block_ctx);

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
                gas_used: cumulative_gas,
                txs_skipped,
            },
            state,
        )
    }

    pub fn apply_block_with_metrics<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: &Block,
    ) -> (BlockResult, ExecutionState<'a, S>, BlockExecutionMetrics) {
        let mut state = ExecutionState::new(storage);
        let block_start_metrics = state.metrics_snapshot();
        let block_timer = Instant::now();

        let begin_timer = Instant::now();
        let begin_block_events = self.do_begin_block(&mut state, account_codes, block);
        let begin_block_ns = begin_timer.elapsed().as_nanos() as u64;

        let txs = block.txs();
        let block_ctx = block.context();
        let block_gas_limit = block.gas_limit();
        let mut tx_results = Vec::with_capacity(txs.len());
        let mut tx_metrics = Vec::with_capacity(txs.len());
        let mut cumulative_gas: u64 = 0;
        let mut txs_skipped: usize = 0;

        for (index, tx) in txs.iter().enumerate() {
            // Check if adding this tx would exceed block gas limit.
            if cumulative_gas.saturating_add(tx.gas_limit()) > block_gas_limit {
                txs_skipped += 1;
                continue;
            }

            let tx_start_metrics = state.metrics_snapshot();
            let tx_timer = Instant::now();
            let tx_result = self.apply_tx(
                &mut state,
                account_codes,
                tx,
                self.storage_gas_config.clone(),
                block_ctx,
            );
            let duration_ns = tx_timer.elapsed().as_nanos() as u64;
            let tx_end_metrics = state.metrics_snapshot();
            let delta = tx_end_metrics.delta(tx_start_metrics);
            tx_metrics.push(TxExecutionMetrics::from_delta(
                index as u64,
                duration_ns,
                tx_result.gas_used,
                delta,
            ));

            cumulative_gas = cumulative_gas.saturating_add(tx_result.gas_used);
            tx_results.push(tx_result);

            // If we've hit the limit after execution, stop processing more txs.
            if cumulative_gas >= block_gas_limit {
                txs_skipped += txs.len() - tx_results.len();
                break;
            }
        }

        let end_timer = Instant::now();
        let end_block_events = self.do_end_block(&mut state, account_codes, block_ctx);
        let end_block_ns = end_timer.elapsed().as_nanos() as u64;

        let total_ns = block_timer.elapsed().as_nanos() as u64;
        let block_end_metrics = state.metrics_snapshot();
        let delta = block_end_metrics.delta(block_start_metrics);
        let metrics = BlockExecutionMetrics::from_delta(
            begin_block_ns,
            end_block_ns,
            total_ns,
            delta,
            tx_metrics,
        );

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
                gas_used: cumulative_gas,
                txs_skipped,
            },
            state,
            metrics,
        )
    }

    fn do_begin_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        account_codes: &'a A,
        block: &Block,
    ) -> Vec<Event> {
        let mut gas_counter = GasCounter::Infinite;
        {
            let mut ctx = Invoker::new_for_begin_block(
                state,
                account_codes,
                &mut gas_counter,
                block.context(),
            );

            self.begin_blocker.begin_block(block, &mut ctx);
        }

        state.pop_events()
    }

    fn do_end_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        codes: &'a A,
        block: BlockContext,
    ) -> Vec<Event> {
        let mut gas_counter = GasCounter::Infinite;
        {
            let mut ctx = Invoker::new_for_end_block(state, codes, &mut gas_counter, block);
            self.end_blocker.end_block(&mut ctx);
        }
        state.pop_events()
    }

    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        codes: &'a A,
        tx: &Tx,
        gas_config: StorageGasConfig,
        block: BlockContext,
    ) -> TxResult {
        // create a finite gas counter for the full tx lifecycle,
        // including optional sender bootstrap registration.
        let mut gas_counter = GasCounter::Finite {
            gas_limit: tx.gas_limit(),
            gas_used: 0,
            storage_gas_config: gas_config,
        };

        let resolved_sender = {
            let mut resolve_ctx =
                Invoker::new_for_begin_block(state, codes, &mut gas_counter, block);
            let sender = match tx.resolve_sender_account(&mut resolve_ctx) {
                Ok(sender) => sender,
                Err(err) => {
                    drop(resolve_ctx);
                    let gas_used = gas_counter.gas_used();
                    let events = state.pop_events();
                    return TxResult {
                        events,
                        gas_used,
                        response: Err(err),
                    };
                }
            };
            drop(resolve_ctx);
            sender
        };

        // Auto-register sender when transaction provides a bootstrap primitive.
        if let Some(bootstrap) = tx.sender_bootstrap() {
            let mut reg_ctx = Invoker::new_for_begin_block(state, codes, &mut gas_counter, block);
            if let Err(err) = reg_ctx.register_account_at_id(
                resolved_sender,
                bootstrap.account_code_id,
                bootstrap.init_message,
            ) {
                drop(reg_ctx);
                let gas_used = gas_counter.gas_used();
                let events = state.pop_events();
                return TxResult {
                    events,
                    gas_used,
                    response: Err(err),
                };
            }
            drop(reg_ctx);
            state.pop_events();
        }

        // NOTE: Transaction validation and execution are atomic - they share the same
        // ExecutionState throughout the process. The state cannot change between
        // validation and execution because:
        // 1. The same ExecutionState instance is used for both phases
        // 2. into_new_exec() preserves the storage, maintaining consistency
        // 3. Transactions are processed sequentially, not concurrently

        // create validation context
        let mut ctx = Invoker::new_for_validate_tx(state, codes, &mut gas_counter, tx, block);
        // do tx validation; we do not swap invoker
        match self.tx_validator.validate_tx(tx, &mut ctx) {
            Ok(()) => (),
            Err(err) => {
                drop(ctx);
                let gas_used = gas_counter.gas_used();
                let events = state.pop_events();
                return TxResult {
                    events,
                    gas_used,
                    response: Err(err),
                };
            }
        }

        // exec tx - transforms validation context to execution context
        // while preserving the same underlying state
        let mut ctx = ctx.into_new_exec(resolved_sender);

        let recipient = match tx.resolve_recipient_account(&mut ctx) {
            Ok(recipient) => recipient,
            Err(err) => {
                drop(ctx);
                let gas_used = gas_counter.gas_used();
                let events = state.pop_events();
                return TxResult {
                    events,
                    gas_used,
                    response: Err(err),
                };
            }
        };
        let response = ctx.do_exec(recipient, tx.request(), tx.funds().to_vec());

        // Run post-tx handler (e.g., for fee collection, logging, etc.)
        // The handler can observe the result and make additional state changes
        let post_tx_result = PostTx::after_tx_executed(tx, ctx.gas_used(), &response, &mut ctx);

        drop(ctx);
        let gas_used = gas_counter.gas_used();
        let events = state.pop_events();

        // If post-tx handler fails, the tx response becomes the error
        // This allows the handler to reject transactions after execution
        let final_response = match post_tx_result {
            Ok(()) => response,
            Err(post_tx_err) => Err(post_tx_err),
        };

        TxResult {
            events,
            gas_used,
            response: final_response,
        }
    }

    /// Executes a query against a specific account.
    ///
    /// Queries are read-only operations that don't modify state. They can be used
    /// to retrieve information from accounts without consuming gas or affecting
    /// the blockchain state.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `to` - Target account ID for the query
    /// * `req` - Query request message
    /// * `gc` - Gas counter for query execution
    ///
    /// # Returns
    ///
    /// Returns the query response from the target account.
    pub fn query<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R: InvokableMessage>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        to: AccountId,
        req: &R,
        gc: GasCounter,
    ) -> SdkResult<InvokeResponse> {
        let mut state = ExecutionState::new(storage);
        let mut gas_counter = gc;
        let mut ctx = Invoker::new_for_query(&mut state, account_codes, &mut gas_counter, to);
        ctx.do_query(to, &InvokeRequest::new(req)?)
    }
    /// Executes a closure with read-only access to a specific account's code.
    ///
    /// This method provides a developer-friendly way to extract data from account state
    /// after execution. It's particularly useful for consensus engines that need to
    /// extract validator set changes or other state information after block execution.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with account code access
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
    pub fn run_with_code<T: AccountCode + Default, R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(&T, &mut dyn EnvironmentQuery) -> SdkResult<R>,
    ) -> SdkResult<R> {
        self.run(storage, account_storage, account_id, move |t| {
            handle(&T::default(), t)
        })
    }

    /// Executes a closure with a reference-based account handle.
    ///
    /// Similar to `run_with_code`, but uses a reference type that can be constructed
    /// from the account ID. This is useful for account handles that don't need
    /// to maintain state but need access to the account ID.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with account reference
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
    pub fn run_with_ref<T: From<AccountId>, R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(T, &mut dyn EnvironmentQuery) -> SdkResult<R>,
    ) -> SdkResult<R> {
        self.run(storage, account_storage, account_id, move |t| {
            handle(T::from(account_id), t)
        })
    }

    /// Executes a closure with read-only access to an account's environment.
    ///
    /// This is the base method for executing read-only operations against accounts.
    /// It provides an environment interface that can be used to query account state
    /// and perform other read-only operations.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with environment access
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
    pub fn run<R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(&mut dyn EnvironmentQuery) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let mut state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let mut invoker =
            Invoker::new_for_query(&mut state, account_storage, &mut gas_counter, account_id);
        handle(&mut invoker)
    }
}

impl<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx> Clone
    for Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
where
    BeginBlocker: Clone,
    TxValidator: Clone,
    EndBlocker: Clone,
    PostTx: Clone,
{
    fn clone(&self) -> Self {
        Self {
            begin_blocker: self.begin_blocker.clone(),
            end_blocker: self.end_blocker.clone(),
            tx_validator: self.tx_validator.clone(),
            post_tx_handler: self.post_tx_handler.clone(),
            storage_gas_config: self.storage_gas_config.clone(),
            _phantoms: PhantomData,
        }
    }
}

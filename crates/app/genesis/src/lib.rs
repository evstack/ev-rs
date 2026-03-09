//! # Genesis Module
//!
//! This module provides genesis file handling for the Evolve blockchain.
//! Genesis is represented as an ordered list of unsigned transactions that
//! are run before block 1, populating initial state.
//!
//! ## Design
//!
//! Instead of having a separate genesis initialization code path, genesis
//! reuses the normal transaction handling infrastructure with:
//! - Signature verification disabled
//! - Infinite gas
//! - Block height 0
//!
//! This ensures genesis state is created through the same code paths as
//! runtime state transitions.

pub mod error;
pub mod file;
pub mod registry;
pub mod types;

pub use error::GenesisError;
pub use file::{GenesisFile, GenesisTxJson};
pub use registry::MessageRegistry;
pub use types::{ConsensusParams, GenesisTx, DEFAULT_BLOCK_GAS_LIMIT};

use evolve_core::runtime_api::CONSENSUS_PARAMS_KEY;
use evolve_core::{AccountId, BlockContext, Environment, Message, ReadonlyKV, SdkResult};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::Stf;
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker, Block as BlockTrait, EndBlocker, PostTxExecution,
    Transaction, TxValidator,
};
use std::cell::RefCell;

/// System account ID used as sender for genesis transactions.
/// This is a sentinel value that indicates the transaction originates
/// from the genesis process rather than a real account.
pub const SYSTEM_ACCOUNT_ID: AccountId = AccountId::from_u64(0);

/// Applies genesis transactions to initialize chain state.
///
/// This function runs a list of genesis transactions with:
/// - No signature verification
/// - Infinite gas (no metering)
/// - Block height 0
///
/// # Arguments
///
/// * `stf` - The state transition function
/// * `storage` - Read-only storage backend
/// * `codes` - Account code storage
/// * `consensus_params` - Consensus parameters to store in state
/// * `txs` - Ordered list of genesis transactions
///
/// # Returns
///
/// Returns the result state containing all state changes from genesis,
/// along with results for each transaction. The consensus parameters are
/// stored at a well-known key so syncing nodes can read them.
pub fn apply_genesis<'a, S, A, Tx, Block, BB, TV, EB, PT>(
    stf: &Stf<Tx, Block, BB, TV, EB, PT>,
    storage: &'a S,
    codes: &'a A,
    consensus_params: &ConsensusParams,
    txs: Vec<GenesisTx>,
) -> Result<(ExecutionState<'a, S>, Vec<GenesisResult>), GenesisError>
where
    S: ReadonlyKV + 'a,
    A: AccountsCodeStorage + 'a,
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    BB: BeginBlocker<Block>,
    TV: TxValidator<Tx>,
    EB: EndBlocker,
    PT: PostTxExecution<Tx>,
{
    let genesis_block = BlockContext::new(0, 0);
    let results = RefCell::new(Vec::with_capacity(txs.len()));

    // Run all genesis transactions in a single system_exec call
    // to ensure they share the same state
    let (_, mut state) = stf
        .system_exec(
            storage,
            codes,
            genesis_block,
            |env: &mut dyn Environment| -> SdkResult<()> {
                for (idx, tx) in txs.iter().enumerate() {
                    let result = env.do_exec(tx.recipient, &tx.request, vec![]);
                    let success = result.is_ok();
                    let error = result.err().map(|e| format!("{:?}", e));

                    results.borrow_mut().push(GenesisResult {
                        index: idx,
                        id: tx.id.clone(),
                        success,
                        error,
                    });

                    // Stop on first error - genesis must be fully successful
                    if !success {
                        break;
                    }
                }
                Ok(())
            },
        )
        .map_err(|e| GenesisError::Failed(format!("{:?}", e)))?;

    let results = results.into_inner();

    // Check if any transaction failed
    if let Some(failed) = results.iter().find(|r| !r.success) {
        return Err(GenesisError::TransactionFailed {
            index: failed.index,
            id: failed.id.clone(),
            error: failed.error.clone().unwrap_or_default(),
        });
    }

    // Store consensus params in state at well-known key.
    // This allows syncing nodes to read block gas limit for validation.
    let params_bytes = borsh::to_vec(consensus_params).map_err(|e| {
        GenesisError::Failed(format!("failed to serialize consensus params: {}", e))
    })?;
    state
        .set(CONSENSUS_PARAMS_KEY, Message::from_bytes(params_bytes))
        .map_err(|e| GenesisError::Failed(format!("failed to store consensus params: {:?}", e)))?;

    Ok((state, results))
}

/// Result of a single genesis transaction.
#[derive(Debug, Clone)]
pub struct GenesisResult {
    /// Index of the transaction in the genesis file
    pub index: usize,
    /// Optional ID from the genesis file
    pub id: Option<String>,
    /// Whether the transaction succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Reads consensus parameters from storage.
///
/// This is used by syncing nodes to read the block gas limit and other
/// consensus parameters that were set during genesis.
///
/// # Arguments
///
/// * `storage` - Read-only storage backend
///
/// # Returns
///
/// Returns the consensus parameters if found, or an error if not present
/// or if deserialization fails.
pub fn read_consensus_params<S: ReadonlyKV>(storage: &S) -> Result<ConsensusParams, GenesisError> {
    use borsh::BorshDeserialize;

    let bytes = storage
        .get(CONSENSUS_PARAMS_KEY)
        .map_err(|e| GenesisError::Failed(format!("failed to read consensus params: {:?}", e)))?
        .ok_or_else(|| GenesisError::Failed("consensus params not found in state".to_string()))?;

    ConsensusParams::try_from_slice(&bytes)
        .map_err(|e| GenesisError::Failed(format!("failed to deserialize consensus params: {}", e)))
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    use super::*;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::{
        AccountCode, Environment, EnvironmentQuery, ErrorCode, FungibleAsset, InvokableMessage,
        InvokeRequest, InvokeResponse,
    };
    use evolve_stf::StorageGasConfig;
    use evolve_stf_traits::{AccountsCodeStorage, WritableKV};
    use std::collections::HashMap;

    // Test message that the test account can handle
    #[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
    struct TestInitMsg {
        value: u64,
    }

    impl InvokableMessage for TestInitMsg {
        const FUNCTION_IDENTIFIER: u64 = 0; // init
        const FUNCTION_IDENTIFIER_NAME: &'static str = "init";
    }

    // Test account that tracks initialization
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
            _env: &mut dyn Environment,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
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

    // Account that always fails
    struct FailingAccount;

    impl AccountCode for FailingAccount {
        fn identifier(&self) -> String {
            "failing_account".to_string()
        }

        fn schema(&self) -> evolve_core::schema::AccountSchema {
            evolve_core::schema::AccountSchema::new("FailingAccount", "failing_account")
        }

        fn init(
            &self,
            _env: &mut dyn Environment,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            Err(ErrorCode::new(999))
        }

        fn execute(
            &self,
            _env: &mut dyn Environment,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            Err(ErrorCode::new(999))
        }

        fn query(
            &self,
            _env: &mut dyn EnvironmentQuery,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            Err(ErrorCode::new(999))
        }
    }

    // Minimal test infrastructure
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

    impl evolve_core::ReadonlyKV for InMemoryStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
            Ok(self.data.get(key).cloned())
        }
    }

    impl WritableKV for InMemoryStorage {
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

    // Test transaction type
    #[derive(Clone)]
    struct TestTx {
        sender: AccountId,
        recipient: AccountId,
        request: InvokeRequest,
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
            1_000_000
        }
        fn funds(&self) -> &[FungibleAsset] {
            &[]
        }
        fn compute_identifier(&self) -> [u8; 32] {
            [0u8; 32]
        }
    }

    // Test block type
    struct TestBlock {
        txs: Vec<TestTx>,
    }

    impl BlockTrait<TestTx> for TestBlock {
        fn context(&self) -> BlockContext {
            BlockContext::new(0, 0)
        }
        fn txs(&self) -> &[TestTx] {
            &self.txs
        }
    }

    // Noop components
    #[derive(Default)]
    struct NoopBegin;
    impl BeginBlocker<TestBlock> for NoopBegin {
        fn begin_block(&self, _: &TestBlock, _: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct NoopEnd;
    impl EndBlocker for NoopEnd {
        fn end_block(&self, _: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct NoopValidator;
    impl TxValidator<TestTx> for NoopValidator {
        fn validate_tx(&self, _: &TestTx, _: &mut dyn Environment) -> SdkResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopPostTx;
    impl PostTxExecution<TestTx> for NoopPostTx {
        fn after_tx_executed(
            _: &TestTx,
            _: u64,
            _: &SdkResult<InvokeResponse>,
            _: &mut dyn Environment,
        ) -> SdkResult<()> {
            Ok(())
        }
    }

    type TestStf = Stf<TestTx, TestBlock, NoopBegin, NoopValidator, NoopEnd, NoopPostTx>;

    fn setup_stf() -> TestStf {
        TestStf::new(
            NoopBegin,
            NoopEnd,
            NoopValidator,
            NoopPostTx,
            StorageGasConfig::default(),
        )
    }

    fn setup_storage_with_account(account_id: AccountId, code_identifier: &str) -> InMemoryStorage {
        use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
        use evolve_core::Message;

        let mut storage = InMemoryStorage::default();
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account_id.as_bytes());
        storage.data.insert(
            key,
            Message::new(&code_identifier.to_string())
                .unwrap()
                .into_bytes()
                .unwrap(),
        );
        storage
    }

    #[test]
    fn test_apply_genesis_success() {
        let stf = setup_stf();
        let mut codes = CodeStore::new();
        codes.add_code(TestAccount);

        let test_account_id = AccountId::from_u64(100);
        let storage = setup_storage_with_account(test_account_id, "test_account");
        let consensus_params = ConsensusParams::default();

        let txs = vec![GenesisTx::with_id(
            "test",
            SYSTEM_ACCOUNT_ID,
            test_account_id,
            InvokeRequest::new(&TestInitMsg { value: 42 }).unwrap(),
        )];

        let result = apply_genesis(&stf, &storage, &codes, &consensus_params, txs);
        assert!(result.is_ok());

        let (_, results) = result.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].id, Some("test".to_string()));
    }

    #[test]
    fn test_apply_genesis_multiple_transactions() {
        let stf = setup_stf();
        let mut codes = CodeStore::new();
        codes.add_code(TestAccount);

        let account1 = AccountId::from_u64(100);
        let account2 = AccountId::from_u64(101);
        let mut storage = setup_storage_with_account(account1, "test_account");
        let consensus_params = ConsensusParams::default();
        // Add second account
        {
            use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
            use evolve_core::Message;
            let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
            key.extend_from_slice(&account2.as_bytes());
            storage.data.insert(
                key,
                Message::new(&"test_account".to_string())
                    .unwrap()
                    .into_bytes()
                    .unwrap(),
            );
        }

        let txs = vec![
            GenesisTx::with_id(
                "first",
                SYSTEM_ACCOUNT_ID,
                account1,
                InvokeRequest::new(&TestInitMsg { value: 1 }).unwrap(),
            ),
            GenesisTx::with_id(
                "second",
                SYSTEM_ACCOUNT_ID,
                account2,
                InvokeRequest::new(&TestInitMsg { value: 2 }).unwrap(),
            ),
        ];

        let result = apply_genesis(&stf, &storage, &codes, &consensus_params, txs);
        assert!(result.is_ok());

        let (_, results) = result.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(results[1].success);
    }

    #[test]
    fn test_apply_genesis_stops_on_failure() {
        let stf = setup_stf();
        let mut codes = CodeStore::new();
        codes.add_code(TestAccount);
        codes.add_code(FailingAccount);

        let good_account = AccountId::from_u64(100);
        let bad_account = AccountId::from_u64(101);
        let after_bad = AccountId::from_u64(102);
        let consensus_params = ConsensusParams::default();

        let mut storage = setup_storage_with_account(good_account, "test_account");
        {
            use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
            use evolve_core::Message;
            let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
            key.extend_from_slice(&bad_account.as_bytes());
            storage.data.insert(
                key,
                Message::new(&"failing_account".to_string())
                    .unwrap()
                    .into_bytes()
                    .unwrap(),
            );
            let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
            key.extend_from_slice(&after_bad.as_bytes());
            storage.data.insert(
                key,
                Message::new(&"test_account".to_string())
                    .unwrap()
                    .into_bytes()
                    .unwrap(),
            );
        }

        let txs = vec![
            GenesisTx::with_id(
                "good",
                SYSTEM_ACCOUNT_ID,
                good_account,
                InvokeRequest::new(&TestInitMsg { value: 1 }).unwrap(),
            ),
            GenesisTx::with_id(
                "bad",
                SYSTEM_ACCOUNT_ID,
                bad_account,
                InvokeRequest::new(&TestInitMsg { value: 2 }).unwrap(),
            ),
            GenesisTx::with_id(
                "never_reached",
                SYSTEM_ACCOUNT_ID,
                after_bad,
                InvokeRequest::new(&TestInitMsg { value: 3 }).unwrap(),
            ),
        ];

        let result = apply_genesis(&stf, &storage, &codes, &consensus_params, txs);
        assert!(result.is_err());

        match result {
            Err(GenesisError::TransactionFailed { index, id, .. }) => {
                assert_eq!(index, 1);
                assert_eq!(id, Some("bad".to_string()));
            }
            _ => panic!("expected TransactionFailed error"),
        }
    }

    #[test]
    fn test_apply_genesis_failure_without_id_propagates_none_id() {
        let stf = setup_stf();
        let mut codes = CodeStore::new();
        codes.add_code(FailingAccount);

        let bad_account = AccountId::from_u64(100);
        let storage = setup_storage_with_account(bad_account, "failing_account");
        let consensus_params = ConsensusParams::default();

        let txs = vec![GenesisTx::new(
            SYSTEM_ACCOUNT_ID,
            bad_account,
            InvokeRequest::new(&TestInitMsg { value: 1 }).unwrap(),
        )];

        let result = apply_genesis(&stf, &storage, &codes, &consensus_params, txs);
        assert!(result.is_err());

        assert!(matches!(
            result,
            Err(GenesisError::TransactionFailed {
                index: 0,
                id: None,
                ..
            })
        ));
    }

    #[test]
    fn test_apply_genesis_empty_transactions() {
        let stf = setup_stf();
        let codes = CodeStore::new();
        let storage = InMemoryStorage::default();
        let consensus_params = ConsensusParams::default();

        let txs: Vec<GenesisTx> = vec![];

        let result = apply_genesis(&stf, &storage, &codes, &consensus_params, txs);
        assert!(result.is_ok());

        let (_, results) = result.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_consensus_params_stored_in_state() {
        use evolve_stf_traits::WritableKV;

        let stf = setup_stf();
        let codes = CodeStore::new();
        let mut storage = InMemoryStorage::default();
        let consensus_params = ConsensusParams::new(50_000_000); // 50M gas limit

        let txs: Vec<GenesisTx> = vec![];

        let (state, _) = apply_genesis(&stf, &storage, &codes, &consensus_params, txs).unwrap();

        // Apply the state changes to storage
        let changes = state.into_changes().unwrap();
        storage.apply_changes(changes).unwrap();

        // Read back the consensus params
        let read_params = read_consensus_params(&storage).unwrap();
        assert_eq!(read_params.block_gas_limit, 50_000_000);
    }
}

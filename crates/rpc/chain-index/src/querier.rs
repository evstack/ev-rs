//! State querier for reading account balances, nonces, and call simulations.
//!
//! This module provides direct storage reads for simple RPC state queries and
//! STF-backed simulations for `eth_call` / `eth_estimateGas`.

use std::sync::Arc;

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use async_trait::async_trait;
use evolve_core::encoding::Encodable;
use evolve_core::{
    runtime_api::ACCOUNT_IDENTIFIER_PREFIX, AccountId, BlockContext, FungibleAsset, InvokeRequest,
    InvokeResponse, Message, ReadonlyKV, ERR_UNKNOWN_FUNCTION,
};
use evolve_eth_jsonrpc::error::RpcError;
use evolve_rpc_types::CallRequest;
use evolve_stf::results::TxResult;
use evolve_stf::{QueryContext, Stf, ERR_OUT_OF_GAS};
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, TxValidator,
};
use evolve_tx_eth::{
    derive_eth_eoa_account_id, lookup_account_id_in_storage, lookup_contract_account_id_in_storage,
    sender_types, TxContext, TxContextMeta, TxPayload, ETH_EOA_CODE_ID,
};

const DEFAULT_RPC_GAS_LIMIT: u64 = 30_000_000;

/// Trait for querying on-chain state (balance, nonce, simulated calls).
#[async_trait]
pub trait StateQuerier: Send + Sync {
    /// Get the token balance for an Ethereum address.
    async fn get_balance(&self, address: Address) -> Result<U256, RpcError>;

    /// Get the transaction count (nonce) for an Ethereum address.
    async fn get_transaction_count(&self, address: Address) -> Result<u64, RpcError>;

    /// Get code bytes for an Ethereum address.
    async fn get_code(&self, address: Address, block: Option<u64>) -> Result<Bytes, RpcError>;

    /// Get a storage word for an Ethereum address.
    async fn get_storage_at(
        &self,
        address: Address,
        position: U256,
        block: Option<u64>,
    ) -> Result<B256, RpcError>;

    /// Execute a read-only call.
    async fn call(&self, request: &CallRequest, block: Option<u64>) -> Result<Bytes, RpcError>;

    /// Estimate gas for a transaction.
    async fn estimate_gas(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<u64, RpcError>;
}

/// STF execution hooks required by the RPC querier.
pub trait RpcExecutionContext: Send + Sync {
    fn simulate_call_tx<S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_codes: &A,
        tx: &TxContext,
        block: BlockContext,
    ) -> TxResult;

    fn execute_query<S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_codes: &A,
        to: AccountId,
        request: &InvokeRequest,
        context: QueryContext,
    ) -> TxResult;
}

impl<B, Begin, ValidatorT, End, Post> RpcExecutionContext
    for Stf<TxContext, B, Begin, ValidatorT, End, Post>
where
    B: BlockTrait<TxContext> + Send + Sync,
    Begin: BeginBlockerTrait<B> + Send + Sync,
    ValidatorT: TxValidator<TxContext> + Send + Sync,
    End: EndBlockerTrait + Send + Sync,
    Post: PostTxExecution<TxContext> + Send + Sync,
{
    fn simulate_call_tx<S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_codes: &A,
        tx: &TxContext,
        block: BlockContext,
    ) -> TxResult {
        self.simulate_transaction(storage, account_codes, tx, block)
    }

    fn execute_query<S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_codes: &A,
        to: AccountId,
        request: &InvokeRequest,
        context: QueryContext,
    ) -> TxResult {
        self.query_invoke_request(storage, account_codes, to, request, context)
    }
}

/// State querier that reads directly from storage and uses STF dry-runs for RPC calls.
///
/// Uses the known storage key layout to read token balances and nonces without
/// invoking the full STF for simple state reads:
/// - Nonce: `account_id_bytes ++ [0]` (EthEoaAccount storage prefix 0)
/// - Balance: `token_id_bytes ++ [1] ++ encode(account_id)` (Token storage prefix 1)
pub struct StorageStateQuerier<S, A, E> {
    storage: S,
    token_account_id: AccountId,
    account_codes: Arc<A>,
    executor: Arc<E>,
    default_gas_limit: u64,
}

impl<S: ReadonlyKV + Send + Sync, A: AccountsCodeStorage + Send + Sync, E: RpcExecutionContext>
    StorageStateQuerier<S, A, E>
{
    pub fn new(
        storage: S,
        token_account_id: AccountId,
        account_codes: Arc<A>,
        executor: Arc<E>,
    ) -> Self {
        Self {
            storage,
            token_account_id,
            account_codes,
            executor,
            default_gas_limit: DEFAULT_RPC_GAS_LIMIT,
        }
    }

    fn read_nonce(&self, account_id: AccountId) -> Result<u64, RpcError> {
        let mut key = account_id.as_bytes().to_vec();
        key.push(0u8); // EthEoaAccount::nonce storage prefix
        match self
            .storage
            .get(&key)
            .map_err(|e| RpcError::InternalError(format!("storage read: {:?}", e)))?
        {
            Some(value) => Message::from_bytes(value)
                .get::<u64>()
                .map_err(|e| RpcError::InternalError(format!("decode nonce: {:?}", e))),
            None => Ok(0),
        }
    }

    fn read_balance(&self, account_id: AccountId) -> Result<u128, RpcError> {
        let mut key = self.token_account_id.as_bytes().to_vec();
        key.push(1u8); // Token::balances storage prefix
        key.extend(
            account_id
                .encode()
                .map_err(|e| RpcError::InternalError(format!("encode account id: {:?}", e)))?,
        );
        match self
            .storage
            .get(&key)
            .map_err(|e| RpcError::InternalError(format!("storage read: {:?}", e)))?
        {
            Some(value) => Message::from_bytes(value)
                .get::<u128>()
                .map_err(|e| RpcError::InternalError(format!("decode balance: {:?}", e))),
            None => Ok(0),
        }
    }

    fn read_raw_storage(&self, key: &[u8]) -> Result<Option<Vec<u8>>, RpcError> {
        self.storage
            .get(key)
            .map_err(|e| RpcError::InternalError(format!("storage read: {:?}", e)))
    }

    fn read_account_storage(
        &self,
        account_id: AccountId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, RpcError> {
        let mut full_key = account_id.as_bytes().to_vec();
        full_key.extend_from_slice(key);
        self.read_raw_storage(&full_key)
    }

    fn read_account_code_identifier(
        &self,
        account_id: AccountId,
    ) -> Result<Option<String>, RpcError> {
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account_id.as_bytes());
        match self.read_raw_storage(&key)? {
            Some(value) => Message::from_bytes(value)
                .get::<String>()
                .map(Some)
                .map_err(|e| RpcError::InternalError(format!("decode account code id: {:?}", e))),
            None => Ok(None),
        }
    }

    fn resolve_contract_account_id(&self, address: Address) -> Result<Option<AccountId>, RpcError> {
        lookup_contract_account_id_in_storage(&self.storage, address)
            .map_err(|e| RpcError::InternalError(format!("lookup contract account id: {:?}", e)))
    }

    fn resolve_account_id(&self, address: Address) -> Result<Option<AccountId>, RpcError> {
        if let Some(account_id) = lookup_account_id_in_storage(&self.storage, address)
            .map_err(|e| RpcError::InternalError(format!("lookup account id: {:?}", e)))?
        {
            return Ok(Some(account_id));
        }

        self.resolve_contract_account_id(address)
    }

    fn resolve_sender_account_id(&self, address: Address) -> Result<AccountId, RpcError> {
        Ok(self
            .resolve_account_id(address)?
            .unwrap_or_else(|| derive_eth_eoa_account_id(address)))
    }

    fn sender_nonce(&self, address: Address) -> Result<u64, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(0);
        };
        self.read_nonce(account_id)
    }

    fn call_request_gas_limit(&self, request: &CallRequest) -> u64 {
        request
            .gas
            .map(|value| value.to::<u64>())
            .unwrap_or(self.default_gas_limit)
    }

    fn call_request_effective_gas_price(&self, request: &CallRequest) -> u128 {
        let effective = request
            .max_fee_per_gas
            .or(request.gas_price)
            .unwrap_or(U256::ZERO);
        u128::try_from(effective).unwrap_or(u128::MAX)
    }

    fn call_request_funds(&self, request: &CallRequest) -> Result<Vec<FungibleAsset>, RpcError> {
        let Some(value) = request.value else {
            return Ok(vec![]);
        };
        if value.is_zero() {
            return Ok(vec![]);
        }

        let amount = u128::try_from(value).map_err(|_| {
            RpcError::InvalidParams("call value exceeds supported native token range".to_string())
        })?;

        Ok(vec![FungibleAsset {
            asset_id: self.token_account_id,
            amount,
        }])
    }

    fn block_context(block: Option<u64>) -> BlockContext {
        BlockContext::new(block.unwrap_or_default(), 0)
    }

    fn call_request_to_invoke_request(request: &CallRequest) -> InvokeRequest {
        let input = request.input_data().cloned().unwrap_or_else(Bytes::new);
        let bytes = input.as_ref();
        let (function_id, args) = if bytes.len() >= 4 {
            (
                u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
                &bytes[4..],
            )
        } else {
            (0u64, bytes)
        };

        InvokeRequest::new_from_message(
            "eth_dispatch",
            function_id,
            Message::from_bytes(args.to_vec()),
        )
    }

    fn synthetic_tx_hash(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<B256, RpcError> {
        let payload = serde_json::to_vec(&(request, block))
            .map_err(|e| RpcError::InternalError(format!("encode synthetic tx: {e}")))?;
        Ok(B256::from(keccak256(payload)))
    }

    fn build_synthetic_tx(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<TxContext, RpcError> {
        let sender = request.from.unwrap_or(Address::ZERO);
        let invoke_request = Self::call_request_to_invoke_request(request);
        let funds = self.call_request_funds(request)?;
        let sender_account = self.resolve_sender_account_id(sender)?;
        let authentication_payload = Message::new(&sender.into_array()).map_err(|e| {
            RpcError::InternalError(format!("encode synthetic auth payload: {:?}", e))
        })?;

        TxContext::from_payload(
            TxPayload::Custom(
                request
                    .input_data()
                    .cloned()
                    .unwrap_or_else(Bytes::new)
                    .to_vec(),
            ),
            sender_types::EOA_SECP256K1,
            TxContextMeta {
                tx_hash: self.synthetic_tx_hash(request, block)?,
                gas_limit: self.call_request_gas_limit(request),
                nonce: self.sender_nonce(sender)?,
                chain_id: None,
                effective_gas_price: self.call_request_effective_gas_price(request),
                invoke_request,
                funds,
                sender_account,
                recipient_account: self.resolve_account_id(request.to)?,
                sender_key: sender.as_slice().to_vec(),
                authentication_payload,
                sender_eth_address: Some(sender.into()),
                recipient_eth_address: Some(request.to.into()),
            },
        )
        .ok_or_else(|| RpcError::InternalError("failed to build synthetic tx context".to_string()))
    }

    fn run_query(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<Option<TxResult>, RpcError> {
        let Some(target_account) = self.resolve_account_id(request.to)? else {
            return Ok(None);
        };
        let sender = self.resolve_sender_account_id(request.from.unwrap_or(Address::ZERO))?;
        let funds = self.call_request_funds(request)?;
        let invoke_request = Self::call_request_to_invoke_request(request);

        Ok(Some(self.executor.execute_query(
            &self.storage,
            self.account_codes.as_ref(),
            target_account,
            &invoke_request,
            QueryContext {
                gas_limit: Some(self.call_request_gas_limit(request)),
                sender,
                funds,
                block: Self::block_context(block),
            },
        )))
    }

    fn run_exec(&self, request: &CallRequest, block: Option<u64>) -> Result<TxResult, RpcError> {
        let synthetic_tx = self.build_synthetic_tx(request, block)?;
        Ok(self.executor.simulate_call_tx(
            &self.storage,
            self.account_codes.as_ref(),
            &synthetic_tx,
            Self::block_context(block),
        ))
    }

    fn response_bytes(response: InvokeResponse) -> Result<Bytes, RpcError> {
        let bytes = response
            .into_inner()
            .into_bytes()
            .map_err(|e| RpcError::InternalError(format!("encode response bytes: {:?}", e)))?;
        Ok(Bytes::from(bytes))
    }

    fn storage_slot_candidates(position: U256) -> Vec<Vec<u8>> {
        let mut candidates = vec![position.to_be_bytes::<32>().to_vec()];
        if position <= U256::from(u8::MAX) {
            candidates.push(vec![position.to::<u8>()]);
        }
        candidates
    }

    fn storage_word(value: &[u8]) -> B256 {
        let mut word = [0u8; 32];
        let word_len = word.len();
        if value.len() >= word_len {
            word.copy_from_slice(&value[value.len() - word_len..]);
        } else {
            let start = word_len - value.len();
            word[start..].copy_from_slice(value);
        }
        B256::from(word)
    }

    fn map_call_error(err: evolve_core::ErrorCode) -> RpcError {
        if err == ERR_OUT_OF_GAS {
            return RpcError::ExecutionReverted("out of gas".to_string());
        }
        RpcError::ExecutionReverted(format!("{:?}", err))
    }

    fn map_estimate_error(err: evolve_core::ErrorCode) -> RpcError {
        if err == ERR_OUT_OF_GAS {
            return RpcError::GasEstimationFailed("out of gas".to_string());
        }
        RpcError::GasEstimationFailed(format!("{:?}", err))
    }
}

#[async_trait]
impl<
        S: ReadonlyKV + Send + Sync,
        A: AccountsCodeStorage + Send + Sync + 'static,
        E: RpcExecutionContext + 'static,
    > StateQuerier for StorageStateQuerier<S, A, E>
{
    async fn get_balance(&self, address: Address) -> Result<U256, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(U256::ZERO);
        };
        let balance = self.read_balance(account_id)?;
        Ok(U256::from(balance))
    }

    async fn get_transaction_count(&self, address: Address) -> Result<u64, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(0);
        };
        self.read_nonce(account_id)
    }

    async fn get_code(&self, address: Address, _block: Option<u64>) -> Result<Bytes, RpcError> {
        let Some(account_id) = self.resolve_contract_account_id(address)? else {
            return Ok(Bytes::new());
        };

        let code_id = self
            .read_account_code_identifier(account_id)?
            .ok_or_else(|| {
                RpcError::InternalError("missing account code identifier".to_string())
            })?;

        if code_id == ETH_EOA_CODE_ID {
            return Ok(Bytes::new());
        }

        Ok(Bytes::from(code_id.into_bytes()))
    }

    async fn get_storage_at(
        &self,
        address: Address,
        position: U256,
        _block: Option<u64>,
    ) -> Result<B256, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(B256::ZERO);
        };

        for key in Self::storage_slot_candidates(position) {
            if let Some(value) = self.read_account_storage(account_id, &key)? {
                return Ok(Self::storage_word(&value));
            }
        }

        Ok(B256::ZERO)
    }

    async fn call(&self, request: &CallRequest, block: Option<u64>) -> Result<Bytes, RpcError> {
        if self.resolve_account_id(request.to)?.is_none() {
            return Ok(Bytes::new());
        }

        let exec_result = self.run_exec(request, block)?;
        match exec_result.response {
            Ok(response) => return Self::response_bytes(response),
            Err(err) if err == ERR_UNKNOWN_FUNCTION => {}
            Err(err) => return Err(Self::map_call_error(err)),
        }

        let Some(query_result) = self.run_query(request, block)? else {
            return Ok(Bytes::new());
        };
        match query_result.response {
            Ok(response) => Self::response_bytes(response),
            Err(err) => Err(Self::map_call_error(err)),
        }
    }

    async fn estimate_gas(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<u64, RpcError> {
        let exec_result = self.run_exec(request, block)?;
        match exec_result.response {
            Ok(_) => return Ok(exec_result.gas_used),
            Err(err) if err == ERR_UNKNOWN_FUNCTION => {}
            Err(err) => return Err(Self::map_estimate_error(err)),
        }

        let Some(query_result) = self.run_query(request, block)? else {
            return Err(RpcError::GasEstimationFailed(
                "target account not found".to_string(),
            ));
        };
        match query_result.response {
            Ok(_) => Ok(query_result.gas_used),
            Err(err) => Err(Self::map_estimate_error(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::encoding::Decodable;
    use evolve_core::low_level::{exec_account, query_account};
    use evolve_core::runtime_api::{ACCOUNT_IDENTIFIER_PREFIX, RUNTIME_ACCOUNT_ID};
    use evolve_core::schema::AccountSchema;
    use evolve_core::storage_api::{
        StorageGetRequest, StorageGetResponse, StorageSetRequest, StorageSetResponse,
        STORAGE_ACCOUNT_ID,
    };
    use evolve_core::{AccountCode, Environment, EnvironmentQuery, SdkResult};
    use evolve_stf_traits::Block as BlockTrait;

    const QUERY_SELECTOR: u32 = 0x0102_0304;
    const EXEC_SELECTOR: u32 = 0x0506_0708;
    const QUERY_FUNCTION_ID: u64 = QUERY_SELECTOR as u64;
    const EXEC_FUNCTION_ID: u64 = EXEC_SELECTOR as u64;
    const EOA_ADDR_TO_ID_PREFIX: &[u8] = b"registry/eoa/eth/a2i/";
    const EOA_ID_TO_ADDR_PREFIX: &[u8] = b"registry/eoa/eth/i2a/";
    const CONTRACT_ADDR_TO_ID_PREFIX: &[u8] = b"registry/contract/runtime/a2i/";
    const CONTRACT_ID_TO_ADDR_PREFIX: &[u8] = b"registry/contract/runtime/i2a/";

    #[derive(Clone, Default)]
    struct TestStorage {
        state: BTreeMap<Vec<u8>, Vec<u8>>,
    }

    impl ReadonlyKV for TestStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, evolve_core::ErrorCode> {
            Ok(self.state.get(key).cloned())
        }
    }

    #[derive(Default)]
    struct TestCodeStorage {
        codes: BTreeMap<String, Box<dyn AccountCode>>,
    }

    impl TestCodeStorage {
        fn add(&mut self, code: impl AccountCode + 'static) {
            self.codes.insert(code.identifier(), Box::new(code));
        }
    }

    impl AccountsCodeStorage for TestCodeStorage {
        fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, evolve_core::ErrorCode>
        where
            F: FnOnce(Option<&dyn AccountCode>) -> R,
        {
            Ok(f(self.codes.get(identifier).map(|code| code.as_ref())))
        }

        fn list_identifiers(&self) -> Vec<String> {
            self.codes.keys().cloned().collect()
        }
    }

    #[derive(Clone, Default)]
    struct TestBlock;

    impl BlockTrait<TxContext> for TestBlock {
        fn context(&self) -> BlockContext {
            BlockContext::new(0, 0)
        }

        fn txs(&self) -> &[TxContext] {
            &[]
        }
    }

    #[derive(Default)]
    struct NoopBegin;

    impl BeginBlockerTrait<TestBlock> for NoopBegin {
        fn begin_block(&self, _block: &TestBlock, _env: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct NoopEnd;

    impl EndBlockerTrait for NoopEnd {
        fn end_block(&self, _env: &mut dyn Environment) {}
    }

    #[derive(Default)]
    struct NoopValidator;

    impl TxValidator<TxContext> for NoopValidator {
        fn validate_tx(&self, _tx: &TxContext, _env: &mut dyn Environment) -> SdkResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopPostTx;

    impl PostTxExecution<TxContext> for NoopPostTx {
        fn after_tx_executed(
            _tx: &TxContext,
            _gas_consumed: u64,
            _tx_result: &SdkResult<InvokeResponse>,
            _env: &mut dyn Environment,
        ) -> SdkResult<()> {
            Ok(())
        }
    }

    type TestStf = Stf<TxContext, TestBlock, NoopBegin, NoopValidator, NoopEnd, NoopPostTx>;

    #[derive(Clone, BorshSerialize, BorshDeserialize)]
    struct ReadValue {
        key: u8,
    }

    #[derive(Clone, BorshSerialize, BorshDeserialize)]
    struct WriteValue {
        key: u8,
        value: u64,
    }

    struct DummyEoaAccount;

    impl AccountCode for DummyEoaAccount {
        fn identifier(&self) -> String {
            "EthEoaAccount".to_string()
        }

        fn schema(&self) -> AccountSchema {
            AccountSchema::new("EthEoaAccount", "EthEoaAccount")
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
            Err(ERR_UNKNOWN_FUNCTION)
        }

        fn query(
            &self,
            _env: &mut dyn EnvironmentQuery,
            _request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            Err(ERR_UNKNOWN_FUNCTION)
        }
    }

    struct TestContract;

    impl AccountCode for TestContract {
        fn identifier(&self) -> String {
            "TestContract".to_string()
        }

        fn schema(&self) -> AccountSchema {
            AccountSchema::new("TestContract", "TestContract")
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
            match request.function() {
                EXEC_FUNCTION_ID => {
                    let payload: WriteValue = request.get()?;
                    let _: StorageSetResponse = exec_account(
                        STORAGE_ACCOUNT_ID,
                        &StorageSetRequest {
                            key: vec![payload.key],
                            value: Message::new(&payload.value)?,
                        },
                        vec![],
                        env,
                    )?;
                    InvokeResponse::new(&payload.value)
                }
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }

        fn query(
            &self,
            env: &mut dyn EnvironmentQuery,
            request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            match request.function() {
                QUERY_FUNCTION_ID => {
                    let payload: ReadValue = request.get()?;
                    let response: StorageGetResponse = query_account(
                        STORAGE_ACCOUNT_ID,
                        &StorageGetRequest {
                            account_id: env.whoami(),
                            key: vec![payload.key],
                        },
                        env,
                    )?;
                    let value = response
                        .value
                        .map(|message| message.get::<u64>())
                        .transpose()?
                        .unwrap_or_default();
                    InvokeResponse::new(&value)
                }
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }
    }

    fn selector_bytes(selector: u32) -> [u8; 4] {
        selector.to_be_bytes()
    }

    fn calldata<T: Encodable>(selector: u32, payload: &T) -> Bytes {
        let mut data = selector_bytes(selector).to_vec();
        data.extend(payload.encode().expect("payload should encode"));
        Bytes::from(data)
    }

    fn code_id_key(account_id: AccountId) -> Vec<u8> {
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account_id.as_bytes());
        key
    }

    fn eoa_forward_key(address: Address) -> Vec<u8> {
        let mut key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
        key.extend_from_slice(EOA_ADDR_TO_ID_PREFIX);
        key.extend_from_slice(address.as_slice());
        key
    }

    fn eoa_reverse_key(account_id: AccountId) -> Vec<u8> {
        let mut key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
        key.extend_from_slice(EOA_ID_TO_ADDR_PREFIX);
        key.extend_from_slice(&account_id.as_bytes());
        key
    }

    fn contract_forward_key(address: Address) -> Vec<u8> {
        let mut key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
        key.extend_from_slice(CONTRACT_ADDR_TO_ID_PREFIX);
        key.extend_from_slice(address.as_slice());
        key
    }

    fn contract_reverse_key(account_id: AccountId) -> Vec<u8> {
        let mut key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
        key.extend_from_slice(CONTRACT_ID_TO_ADDR_PREFIX);
        key.extend_from_slice(&account_id.as_bytes());
        key
    }

    fn account_slot_key(account_id: AccountId, slot: u8) -> Vec<u8> {
        let mut key = account_id.as_bytes().to_vec();
        key.push(slot);
        key
    }

    fn setup_querier() -> (
        StorageStateQuerier<TestStorage, TestCodeStorage, TestStf>,
        Address,
        Address,
    ) {
        let token_account_id = AccountId::from_u64(50);
        let contract_account_id = AccountId::from_u64(100);
        let sender_account_id = AccountId::from_u64(200);
        let contract_address = Address::repeat_byte(0x11);
        let sender_address = Address::repeat_byte(0x22);

        let mut storage = TestStorage::default();
        storage.state.insert(
            code_id_key(contract_account_id),
            Message::new(&"TestContract".to_string())
                .expect("contract code id should encode")
                .into_bytes()
                .expect("contract code id bytes"),
        );
        storage.state.insert(
            code_id_key(sender_account_id),
            Message::new(&"EthEoaAccount".to_string())
                .expect("sender code id should encode")
                .into_bytes()
                .expect("sender code id bytes"),
        );
        storage.state.insert(
            contract_forward_key(contract_address),
            Message::new(&contract_account_id)
                .expect("contract account id should encode")
                .into_bytes()
                .expect("contract account id bytes"),
        );
        storage.state.insert(
            contract_reverse_key(contract_account_id),
            Message::new(&contract_address.into_array())
                .expect("contract address should encode")
                .into_bytes()
                .expect("contract address bytes"),
        );
        storage.state.insert(
            eoa_forward_key(sender_address),
            Message::new(&sender_account_id)
                .expect("sender account id should encode")
                .into_bytes()
                .expect("sender account id bytes"),
        );
        storage.state.insert(
            eoa_reverse_key(sender_account_id),
            Message::new(&sender_address.into_array())
                .expect("sender address should encode")
                .into_bytes()
                .expect("sender address bytes"),
        );
        storage.state.insert(
            account_slot_key(sender_account_id, 0),
            Message::new(&0u64)
                .expect("nonce should encode")
                .into_bytes()
                .expect("nonce bytes"),
        );
        storage.state.insert(
            account_slot_key(contract_account_id, 7),
            Message::new(&77u64)
                .expect("query value should encode")
                .into_bytes()
                .expect("query value bytes"),
        );

        let mut codes = TestCodeStorage::default();
        codes.add(DummyEoaAccount);
        codes.add(TestContract);
        let codes = Arc::new(codes);
        let executor = Arc::new(TestStf::new(
            NoopBegin,
            NoopEnd,
            NoopValidator,
            NoopPostTx,
            evolve_stf::StorageGasConfig::default(),
        ));

        (
            StorageStateQuerier::new(storage, token_account_id, codes, executor),
            contract_address,
            sender_address,
        )
    }

    #[tokio::test]
    async fn eth_call_falls_back_to_query_dispatch() {
        let (querier, contract_address, sender_address) = setup_querier();
        let request = CallRequest {
            from: Some(sender_address),
            to: contract_address,
            data: Some(calldata(QUERY_SELECTOR, &ReadValue { key: 7 })),
            ..Default::default()
        };

        let result = querier
            .call(&request, Some(12))
            .await
            .expect("query call should succeed");

        let decoded = u64::decode(result.as_ref()).expect("result should decode");
        assert_eq!(decoded, 77);
    }

    #[tokio::test]
    async fn eth_call_executes_state_changing_selector_in_dry_run_mode() {
        let (querier, contract_address, sender_address) = setup_querier();
        let request = CallRequest {
            from: Some(sender_address),
            to: contract_address,
            data: Some(calldata(EXEC_SELECTOR, &WriteValue { key: 9, value: 55 })),
            ..Default::default()
        };

        let result = querier
            .call(&request, Some(15))
            .await
            .expect("exec call should succeed");

        let decoded = u64::decode(result.as_ref()).expect("result should decode");
        assert_eq!(decoded, 55);
    }

    #[tokio::test]
    async fn eth_estimate_gas_uses_execution_pipeline_when_available() {
        let (querier, contract_address, sender_address) = setup_querier();
        let request = CallRequest {
            from: Some(sender_address),
            to: contract_address,
            data: Some(calldata(EXEC_SELECTOR, &WriteValue { key: 3, value: 99 })),
            ..Default::default()
        };

        let gas = querier
            .estimate_gas(&request, Some(20))
            .await
            .expect("gas estimate should succeed");

        assert!(gas > 0);
    }

    #[tokio::test]
    async fn eth_call_returns_empty_bytes_for_unknown_target() {
        let (querier, _contract_address, sender_address) = setup_querier();
        let request = CallRequest {
            from: Some(sender_address),
            to: Address::repeat_byte(0xAA),
            data: Some(calldata(QUERY_SELECTOR, &ReadValue { key: 1 })),
            ..Default::default()
        };

        let result = querier
            .call(&request, Some(1))
            .await
            .expect("unknown target should not error");

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn eth_get_code_returns_runtime_code_identifier_bytes() {
        let (querier, contract_address, _sender_address) = setup_querier();

        let code = querier
            .get_code(contract_address, Some(3))
            .await
            .expect("code lookup should succeed");

        assert_eq!(code, Bytes::from("TestContract".as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn eth_get_code_returns_empty_for_eoa_addresses() {
        let (querier, _contract_address, sender_address) = setup_querier();

        let code = querier
            .get_code(sender_address, Some(3))
            .await
            .expect("code lookup should succeed");

        assert!(code.is_empty());
    }

    #[tokio::test]
    async fn eth_get_storage_at_reads_runtime_storage_slots() {
        let (querier, contract_address, _sender_address) = setup_querier();

        let value = querier
            .get_storage_at(contract_address, U256::from(7u64), Some(3))
            .await
            .expect("storage lookup should succeed");

        let raw = Message::new(&77u64)
            .expect("value should encode")
            .into_bytes()
            .expect("value bytes");
        let mut expected = [0u8; 32];
        let start = expected.len() - raw.len();
        expected[start..].copy_from_slice(&raw);

        assert_eq!(value, B256::from(expected));
    }
}

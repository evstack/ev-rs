#![allow(dead_code)]

use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, InvokableMessage, InvokeResponse, ReadonlyKV,
    SdkResult,
};
use evolve_stf::gas::StorageGasConfig;
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker, EndBlocker, PostTxExecution, StateChange, TxValidator,
    WritableKV,
};
use hashbrown::HashMap;

// ---------------------------------------------------------------------------
// Shared test message (used by core and post-tx conformance tests)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct TestMsg {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub fail_after_write: bool,
}

impl InvokableMessage for TestMsg {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "test_msg";
}

// ---------------------------------------------------------------------------
// Noop STF components
// ---------------------------------------------------------------------------

pub struct NoopBegin<B>(std::marker::PhantomData<B>);

impl<B> Default for NoopBegin<B> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<B> BeginBlocker<B> for NoopBegin<B> {
    fn begin_block(&self, _block: &B, _env: &mut dyn Environment) {}
}

#[derive(Default)]
pub struct NoopEnd;

impl EndBlocker for NoopEnd {
    fn end_block(&self, _env: &mut dyn Environment) {}
}

pub struct NoopValidator<T>(std::marker::PhantomData<T>);

impl<T> Default for NoopValidator<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> TxValidator<T> for NoopValidator<T> {
    fn validate_tx(&self, _tx: &T, _env: &mut dyn Environment) -> SdkResult<()> {
        Ok(())
    }
}

pub struct NoopPostTx<T>(std::marker::PhantomData<T>);

impl<T> Default for NoopPostTx<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> PostTxExecution<T> for NoopPostTx<T> {
    fn after_tx_executed(
        _tx: &T,
        _gas_consumed: u64,
        _tx_result: &SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// In-memory storage and code store
// ---------------------------------------------------------------------------

pub struct CodeStore {
    codes: HashMap<String, Box<dyn AccountCode>>,
}

impl Default for CodeStore {
    fn default() -> Self {
        Self {
            codes: HashMap::new(),
        }
    }
}

impl CodeStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_code(&mut self, code: impl AccountCode + 'static) {
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
pub struct InMemoryStorage {
    pub data: HashMap<Vec<u8>, Vec<u8>>,
}

impl ReadonlyKV for InMemoryStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        Ok(self.data.get(key).cloned())
    }
}

impl WritableKV for InMemoryStorage {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        for change in changes {
            match change {
                StateChange::Set { key, value } => {
                    self.data.insert(key, value);
                }
                StateChange::Remove { key } => {
                    self.data.remove(&key);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn account_code_key(account: AccountId) -> Vec<u8> {
    let mut out = vec![ACCOUNT_IDENTIFIER_PREFIX];
    out.extend_from_slice(&account.as_bytes());
    out
}

pub fn default_gas_config() -> StorageGasConfig {
    StorageGasConfig {
        storage_get_charge: 1,
        storage_set_charge: 1,
        storage_remove_charge: 1,
    }
}

pub fn register_account(storage: &mut InMemoryStorage, account: AccountId, code_id: &str) {
    use evolve_core::Message;
    let id = code_id.to_string();
    storage.data.insert(
        account_code_key(account),
        Message::new(&id).unwrap().into_bytes().unwrap(),
    );
}

pub fn extract_account_storage(
    storage: &InMemoryStorage,
    account: AccountId,
) -> HashMap<Vec<u8>, Vec<u8>> {
    let prefix = account.as_bytes();
    let mut result = HashMap::new();
    for (key, value) in &storage.data {
        if key.len() >= prefix.len() && key[..prefix.len()] == prefix {
            result.insert(key[prefix.len()..].to_vec(), value.clone());
        }
    }
    result
}

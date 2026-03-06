#![allow(dead_code)]

use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, InvokeResponse, ReadonlyKV, SdkResult,
};
use evolve_stf::gas::StorageGasConfig;
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker, EndBlocker, PostTxExecution, StateChange, TxValidator,
    WritableKV,
};
use hashbrown::HashMap;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::io::BufReader;
use std::path::Path;

// ---------------------------------------------------------------------------
// ITF deserialization types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
pub struct ItfBigInt {
    #[serde(rename = "#bigint")]
    value: String,
}

impl ItfBigInt {
    pub fn as_i64(&self) -> i64 {
        self.value.parse().unwrap()
    }
    pub fn as_u64(&self) -> u64 {
        self.value.parse().unwrap()
    }
}

#[derive(Deserialize)]
pub struct ItfMap<K, V> {
    #[serde(rename = "#map")]
    pub entries: Vec<(K, V)>,
}

#[derive(Deserialize)]
pub struct ItfBlockResult {
    pub gas_used: ItfBigInt,
    pub tx_results: Vec<ItfTxResult>,
    pub txs_skipped: Option<ItfBigInt>,
}

#[derive(Deserialize)]
pub struct ItfTxResult {
    pub gas_used: ItfBigInt,
    pub result: ItfResult,
}

#[derive(Deserialize)]
pub struct ItfResult {
    pub ok: bool,
    pub err_code: ItfBigInt,
}

// ---------------------------------------------------------------------------
// Spec error constants (shared across conformance tests)
// ---------------------------------------------------------------------------

pub const SPEC_ERR_OUT_OF_GAS: i64 = 0x01;
pub const SPEC_ERR_EXECUTION: i64 = 200;

// ---------------------------------------------------------------------------
// Trace file utilities
// ---------------------------------------------------------------------------

pub fn find_single_trace_file(traces_dir: &Path, test_name: &str) -> std::path::PathBuf {
    let prefix = format!("out_{}_", test_name);
    let trace_files: Vec<_> = std::fs::read_dir(traces_dir)
        .unwrap_or_else(|e| {
            panic!(
                "Cannot read traces directory {}: {e}. \
                 Regenerate traces with: quint test specs/stf_*.qnt \
                 --out-itf \"specs/traces/out_{{test}}_{{seq}}.itf.json\"",
                traces_dir.display()
            )
        })
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".itf.json")
        })
        .collect();

    assert_eq!(
        trace_files.len(),
        1,
        "{}: expected exactly 1 trace file, found {}",
        test_name,
        trace_files.len()
    );
    trace_files[0].path()
}

pub fn read_itf_trace<T: DeserializeOwned>(trace_path: &Path) -> T {
    let file = std::fs::File::open(trace_path).unwrap();
    serde_json::from_reader(BufReader::new(file)).unwrap()
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

pub fn expected_storage_from_itf(
    itf_storage: &ItfMap<ItfBigInt, ItfMap<Vec<ItfBigInt>, Vec<ItfBigInt>>>,
    account: AccountId,
) -> HashMap<Vec<u8>, Vec<u8>> {
    let mut expected = HashMap::new();
    for (account_id_itf, account_store_itf) in &itf_storage.entries {
        if AccountId::new(account_id_itf.as_u64() as u128) != account {
            continue;
        }
        for (key_itf, value_itf) in &account_store_itf.entries {
            let key: Vec<u8> = key_itf.iter().map(|b| b.as_u64() as u8).collect();
            let value: Vec<u8> = value_itf.iter().map(|b| b.as_u64() as u8).collect();
            expected.insert(key, value);
        }
    }
    expected
}

pub fn assert_storage_matches(
    storage: &InMemoryStorage,
    itf_storage: &ItfMap<ItfBigInt, ItfMap<Vec<ItfBigInt>, Vec<ItfBigInt>>>,
    account: AccountId,
    test_name: &str,
) {
    let expected = expected_storage_from_itf(itf_storage, account);
    let actual = extract_account_storage(storage, account);
    assert_eq!(
        actual, expected,
        "{}: storage mismatch for account {:?}",
        test_name, account
    );
}

#![allow(dead_code)]

use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::{AccountCode, AccountId, ErrorCode, ReadonlyKV};
use evolve_stf_traits::{AccountsCodeStorage, BeginBlocker, EndBlocker, StateChange, WritableKV};
use hashbrown::HashMap;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::fs;
use std::path::Path;

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

pub fn find_single_trace_file(traces_dir: &Path, test_name: &str) -> std::path::PathBuf {
    let trace_files: Vec<_> = fs::read_dir(traces_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&format!("out_{}_", test_name)) && name.ends_with(".itf.json")
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
    let trace_json = fs::read_to_string(trace_path).unwrap();
    serde_json::from_str(&trace_json).unwrap()
}

pub struct NoopBegin<B>(std::marker::PhantomData<B>);

impl<B> Default for NoopBegin<B> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<B> BeginBlocker<B> for NoopBegin<B> {
    fn begin_block(&self, _block: &B, _env: &mut dyn evolve_core::Environment) {}
}

#[derive(Default)]
pub struct NoopEnd;

impl EndBlocker for NoopEnd {
    fn end_block(&self, _env: &mut dyn evolve_core::Environment) {}
}

pub struct CodeStore {
    codes: HashMap<String, Box<dyn AccountCode>>,
}

impl CodeStore {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new(),
        }
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

pub fn account_code_key(account: AccountId) -> Vec<u8> {
    let mut out = vec![ACCOUNT_IDENTIFIER_PREFIX];
    out.extend_from_slice(&account.as_bytes());
    out
}

//! `quint_connect` pilot for `specs/stf_post_tx.qnt`.
//!
//! This uses `quint run --mbt` rather than `quint test`: with Quint 0.30.0, the
//! plain `test` traces do not include `mbt::actionTaken`/`mbt::nondetPicks`,
//! which `quint_connect` needs in order to replay steps.

use anyhow::anyhow;
use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::storage_api::{StorageSetRequest, STORAGE_ACCOUNT_ID};
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, ErrorCode, FungibleAsset,
    InvokeRequest, InvokeResponse, Message, SdkResult,
};
use evolve_stf::Stf;
use evolve_stf_traits::{Block as BlockTrait, PostTxExecution, Transaction, WritableKV};
use quint_connect::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

mod quint_common;
use quint_common::{
    default_gas_config, extract_account_storage, register_account, CodeStore, InMemoryStorage,
    NoopBegin, NoopEnd, NoopValidator, TestMsg,
};

const SPEC_ERR_EXECUTION: u16 = 200;
const SPEC_ERR_POST_TX: u16 = 999;
const TEST_SENDER: u64 = 200;
const WRITE_KEY: &[u8] = &[1];
const WRITE_VALUE: &[u8] = &[11];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SpecState {
    storage: BTreeMap<u64, BTreeMap<Vec<u64>, Vec<u64>>>,
    accounts: BTreeSet<u64>,
    last_result: SpecBlockResult,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
struct SpecBlockResult {
    tx_results: Vec<SpecTxResult>,
    gas_used: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SpecTxResult {
    result: SpecResult,
    gas_used: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SpecResult {
    ok: bool,
    err_code: u64,
}

impl State<PostTxDriver> for SpecState {
    fn from_driver(driver: &PostTxDriver) -> Result<Self> {
        let accounts = driver.accounts();
        let storage = accounts
            .iter()
            .copied()
            .map(|account| {
                (
                    account,
                    extract_account_storage(&driver.storage, AccountId::from_u64(account))
                        .into_iter()
                        .map(|(key, value)| {
                            (
                                key.into_iter().map(u64::from).collect(),
                                value.into_iter().map(u64::from).collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect();

        Ok(Self {
            storage,
            accounts,
            last_result: driver.last_result.clone(),
        })
    }
}

#[derive(Clone, Debug)]
struct TestTx {
    sender: AccountId,
    recipient: AccountId,
    request: InvokeRequest,
    gas_limit: u64,
    funds: Vec<FungibleAsset>,
    reject_post_tx: bool,
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
        &self.funds
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
struct RejectingPostTx;

impl PostTxExecution<TestTx> for RejectingPostTx {
    fn after_tx_executed(
        tx: &TestTx,
        _gas_consumed: u64,
        tx_result: &SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        if tx.reject_post_tx && tx_result.is_ok() {
            return Err(ErrorCode::new(SPEC_ERR_POST_TX));
        }
        Ok(())
    }
}

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
            return Err(ErrorCode::new(SPEC_ERR_EXECUTION));
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

#[derive(Default)]
struct PostTxDriver {
    storage: InMemoryStorage,
    last_result: SpecBlockResult,
}

impl Driver for PostTxDriver {
    type State = SpecState;

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => { self.reset()?; },
            register_account(recipient: u64) => { self.register_account(recipient)?; },
            apply_registered_tx(
                recipient: u64,
                gas_limit: u64,
                fail_e: bool,
                reject_pt: bool
            ) => { self.apply_registered_tx(recipient, gas_limit, fail_e, reject_pt)?; }
        })
    }
}

impl PostTxDriver {
    fn reset(&mut self) -> Result {
        self.storage = InMemoryStorage::default();
        self.last_result = SpecBlockResult::default();
        Ok(())
    }

    fn register_account(&mut self, recipient: u64) -> Result {
        register_account(
            &mut self.storage,
            AccountId::from_u64(recipient),
            "test_account",
        );
        Ok(())
    }

    fn apply_registered_tx(
        &mut self,
        recipient: u64,
        gas_limit: u64,
        fail_e: bool,
        reject_pt: bool,
    ) -> Result {
        let msg = TestMsg {
            key: WRITE_KEY.to_vec(),
            value: WRITE_VALUE.to_vec(),
            fail_after_write: fail_e,
        };
        let tx = TestTx {
            sender: AccountId::from_u64(TEST_SENDER),
            recipient: AccountId::from_u64(recipient),
            request: InvokeRequest::new(&msg)
                .map_err(|err| anyhow!("failed to serialize test request: {err:?}"))?,
            gas_limit,
            funds: vec![],
            reject_post_tx: reject_pt,
        };
        let block = TestBlock {
            height: 1,
            time: 0,
            txs: vec![tx],
        };

        let stf = build_stf();
        let codes = build_codes();
        let (result, exec_state) = stf.apply_block(&self.storage, &codes, &block);
        let changes = exec_state
            .into_changes()
            .map_err(|err| anyhow!("failed to extract execution changes: {err:?}"))?;
        self.storage
            .apply_changes(changes)
            .map_err(|err| anyhow!("failed to apply execution changes: {err:?}"))?;
        self.last_result = SpecBlockResult::from_real(&result);

        Ok(())
    }

    fn accounts(&self) -> BTreeSet<u64> {
        self.storage
            .data
            .iter()
            .filter(|(key, value)| {
                key.first() == Some(&ACCOUNT_IDENTIFIER_PREFIX)
                    && Message::from_bytes((*value).clone())
                        .get::<String>()
                        .ok()
                        .as_deref()
                        == Some("test_account")
            })
            .filter_map(|(key, _)| {
                let bytes = key.get(1..33)?;
                let account_bytes: [u8; 32] = bytes.try_into().ok()?;
                Some(account_id_to_u64(AccountId::from_bytes(account_bytes)))
            })
            .collect()
    }
}

fn build_codes() -> CodeStore {
    let mut codes = CodeStore::new();
    codes.add_code(TestAccount);
    codes
}

fn build_stf(
) -> Stf<TestTx, TestBlock, NoopBegin<TestBlock>, NoopValidator<TestTx>, NoopEnd, RejectingPostTx> {
    Stf::new(
        NoopBegin::<TestBlock>::default(),
        NoopEnd,
        NoopValidator::<TestTx>::default(),
        RejectingPostTx,
        default_gas_config(),
    )
}

fn account_id_to_u64(account: AccountId) -> u64 {
    let bytes = account.as_bytes();
    u64::from_be_bytes(
        bytes[24..32]
            .try_into()
            .expect("account id must be 32 bytes"),
    )
}

impl SpecBlockResult {
    fn from_real(result: &evolve_stf::results::BlockResult) -> Self {
        Self {
            tx_results: result
                .tx_results
                .iter()
                .map(SpecTxResult::from_real)
                .collect(),
            gas_used: result.gas_used,
        }
    }
}

impl SpecTxResult {
    fn from_real(result: &evolve_stf::results::TxResult) -> Self {
        Self {
            result: SpecResult::from_real(&result.response),
            gas_used: result.gas_used,
        }
    }
}

impl SpecResult {
    fn from_real(result: &SdkResult<InvokeResponse>) -> Self {
        match result {
            Ok(_) => Self {
                ok: true,
                err_code: 0,
            },
            Err(err) => Self {
                ok: false,
                err_code: normalize_error_code(err.id),
            },
        }
    }
}

fn normalize_error_code(err_id: u16) -> u64 {
    if err_id == evolve_stf::ERR_OUT_OF_GAS.id {
        0x01
    } else {
        err_id as u64
    }
}

#[quint_run(
    spec = "../../../specs/stf_post_tx.qnt",
    main = "stf_post_tx",
    init = "init",
    step = "qc_step",
    max_samples = 8,
    max_steps = 8,
    seed = "0x42"
)]
fn quint_post_tx_qc_run() -> impl Driver {
    PostTxDriver::default()
}

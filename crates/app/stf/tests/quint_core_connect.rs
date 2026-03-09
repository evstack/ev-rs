//! `quint_connect` pilot for `specs/stf_core.qnt`.
//!
//! This uses `quint run --mbt` rather than `quint test`: with Quint 0.30.0, the
//! plain `test` traces do not include `mbt::actionTaken`/`mbt::nondetPicks`,
//! which `quint_connect` needs in order to replay steps.

use anyhow::anyhow;
use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::storage_api::{StorageSetRequest, STORAGE_ACCOUNT_ID};
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, ErrorCode, FungibleAsset,
    InvokeRequest, InvokeResponse, Message, SdkResult,
};
use evolve_stf::Stf;
use evolve_stf_traits::{
    Block as BlockTrait, SenderBootstrap, Transaction, TxValidator, WritableKV,
};
use quint_connect::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

mod quint_common;
use quint_common::{
    default_gas_config, extract_account_storage, register_account, CodeStore, InMemoryStorage,
    NoopBegin, NoopEnd, NoopPostTx, TestMsg,
};

const SPEC_ERR_VALIDATION: u16 = 100;
const SPEC_ERR_EXECUTION: u16 = 200;
const SPEC_ERR_BOOTSTRAP: u16 = 300;
const WRITE_KEY: &[u8] = &[1];
const WRITE_VALUE: &[u8] = &[11];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SpecState {
    storage: BTreeMap<u64, BTreeMap<Vec<u64>, Vec<u64>>>,
    accounts: BTreeSet<u64>,
    block_height: u64,
    last_result: SpecBlockResult,
    last_block_tx_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
struct SpecBlockResult {
    tx_results: Vec<SpecTxResult>,
    gas_used: u64,
    txs_skipped: u64,
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

impl State<CoreDriver> for SpecState {
    fn from_driver(driver: &CoreDriver) -> Result<Self> {
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
            block_height: driver.block_height,
            last_result: driver.last_result.clone(),
            last_block_tx_count: driver.last_block_tx_count,
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
    fail_validate: bool,
    needs_bootstrap: bool,
    fail_bootstrap: bool,
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
    fn sender_bootstrap(&self) -> Option<SenderBootstrap> {
        if !self.needs_bootstrap {
            return None;
        }
        let init = BootstrapInit {
            fail: self.fail_bootstrap,
        };
        let init_message =
            Message::new(&init).expect("bootstrap init serialization must succeed in tests");
        Some(SenderBootstrap {
            account_code_id: "test_account",
            init_message,
        })
    }
}

#[derive(Clone)]
struct TestBlock {
    height: u64,
    time: u64,
    txs: Vec<TestTx>,
    gas_limit: u64,
}

impl BlockTrait<TestTx> for TestBlock {
    fn context(&self) -> BlockContext {
        BlockContext::new(self.height, self.time)
    }
    fn txs(&self) -> &[TestTx] {
        &self.txs
    }
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
}

#[derive(Default)]
struct Validator;

impl TxValidator<TestTx> for Validator {
    fn validate_tx(&self, tx: &TestTx, env: &mut dyn Environment) -> SdkResult<()> {
        if tx.fail_validate {
            return Err(ErrorCode::new(SPEC_ERR_VALIDATION));
        }
        let probe = TestMsg {
            key: vec![],
            value: vec![],
            fail_after_write: false,
        };
        env.do_query(tx.sender(), &InvokeRequest::new(&probe).unwrap())
            .map_err(|_| ErrorCode::new(SPEC_ERR_VALIDATION))?;
        Ok(())
    }
}

#[derive(Default)]
struct TestAccount;

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
struct BootstrapInit {
    fail: bool,
}

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
        request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let init: BootstrapInit = request.get()?;
        if init.fail {
            return Err(ErrorCode::new(SPEC_ERR_BOOTSTRAP));
        }
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
struct CoreDriver {
    storage: InMemoryStorage,
    block_height: u64,
    last_result: SpecBlockResult,
    last_block_tx_count: u64,
}

impl Driver for CoreDriver {
    type State = SpecState;

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => { self.reset()?; },
            register_account(id: u64) => { self.register(id)?; },
            apply_registered_block(
                sender: u64,
                recipient: u64,
                gas_limit: u64,
                block_gas: u64,
                fail_v: bool,
                fail_e: bool,
                needs_b: bool,
                fail_b: bool
            ) => {
                self.apply_registered_block(
                    sender, recipient, gas_limit, block_gas, fail_v, fail_e, needs_b, fail_b
                )?;
            }
        })
    }
}

impl CoreDriver {
    fn reset(&mut self) -> Result {
        self.storage = InMemoryStorage::default();
        self.block_height = 0;
        self.last_result = SpecBlockResult::default();
        self.last_block_tx_count = 0;
        Ok(())
    }

    fn register(&mut self, account_id: u64) -> Result {
        register_account(
            &mut self.storage,
            AccountId::from_u64(account_id),
            "test_account",
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_registered_block(
        &mut self,
        sender: u64,
        recipient: u64,
        gas_limit: u64,
        block_gas: u64,
        fail_v: bool,
        fail_e: bool,
        needs_b: bool,
        fail_b: bool,
    ) -> Result {
        let msg = TestMsg {
            key: WRITE_KEY.to_vec(),
            value: WRITE_VALUE.to_vec(),
            fail_after_write: fail_e,
        };
        let tx = TestTx {
            sender: AccountId::from_u64(sender),
            recipient: AccountId::from_u64(recipient),
            request: InvokeRequest::new(&msg)
                .map_err(|err| anyhow!("failed to serialize test request: {err:?}"))?,
            gas_limit,
            funds: vec![],
            fail_validate: fail_v,
            needs_bootstrap: needs_b,
            fail_bootstrap: fail_b,
        };
        let block = TestBlock {
            height: self.block_height + 1,
            time: 0,
            txs: vec![tx],
            gas_limit: block_gas,
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
        self.block_height = block.height;
        self.last_result = SpecBlockResult::from_real(&result);
        self.last_block_tx_count = block.txs.len() as u64;

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
) -> Stf<TestTx, TestBlock, NoopBegin<TestBlock>, Validator, NoopEnd, NoopPostTx<TestTx>> {
    Stf::new(
        NoopBegin::<TestBlock>::default(),
        NoopEnd,
        Validator,
        NoopPostTx::<TestTx>::default(),
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
            txs_skipped: result.txs_skipped as u64,
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
    spec = "../../../specs/stf_core.qnt",
    main = "stf",
    init = "init",
    step = "qc_step",
    max_samples = 8,
    max_steps = 10,
    seed = "0x42"
)]
fn quint_core_qc_run() -> impl Driver {
    CoreDriver::default()
}

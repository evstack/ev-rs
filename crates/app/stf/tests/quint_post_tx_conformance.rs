//! Conformance tests: replay Quint ITF traces for stf_post_tx.qnt.
//!
//! Run:
//! `quint test specs/stf_post_tx.qnt --out-itf "specs/traces/out_{test}_{seq}.itf.json"`
//! `cargo test -p evolve_stf --test quint_post_tx_conformance`

use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::storage_api::{StorageSetRequest, STORAGE_ACCOUNT_ID};
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, ErrorCode, FungibleAsset,
    InvokableMessage, InvokeRequest, InvokeResponse, Message, SdkResult,
};
use evolve_stf::Stf;
use evolve_stf_traits::{Block as BlockTrait, PostTxExecution, Transaction, WritableKV};
use serde::Deserialize;
use std::path::Path;

mod quint_common;
use quint_common::{
    assert_storage_matches, find_single_trace_file, read_itf_trace, register_account, CodeStore,
    InMemoryStorage, ItfBigInt, ItfBlockResult, ItfMap, NoopBegin, NoopEnd, NoopValidator,
    SPEC_ERR_EXECUTION, SPEC_ERR_OUT_OF_GAS,
};

#[derive(Deserialize)]
struct ItfTrace {
    states: Vec<ItfState>,
}

#[derive(Deserialize)]
struct ItfState {
    last_result: ItfBlockResult,
    storage: ItfMap<ItfBigInt, ItfMap<Vec<ItfBigInt>, Vec<ItfBigInt>>>,
}

const SPEC_ERR_POST_TX: i64 = 999;

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
            return Err(ErrorCode::new(SPEC_ERR_POST_TX as u16));
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
            return Err(ErrorCode::new(SPEC_ERR_EXECUTION as u16));
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

const TEST_ACCOUNT_ID: u128 = 100;
const TEST_SENDER: u128 = 200;

struct TxCase {
    key: Vec<u8>,
    value: Vec<u8>,
    gas_limit: u64,
    fail_execute: bool,
    reject_post_tx: bool,
}

fn make_tx(tc: TxCase) -> TestTx {
    let msg = TestMsg {
        key: tc.key,
        value: tc.value,
        fail_after_write: tc.fail_execute,
    };
    TestTx {
        sender: AccountId::new(TEST_SENDER),
        recipient: AccountId::new(TEST_ACCOUNT_ID),
        request: InvokeRequest::new(&msg).unwrap(),
        gas_limit: tc.gas_limit,
        funds: vec![],
        reject_post_tx: tc.reject_post_tx,
    }
}

struct ConformanceCase {
    test_name: &'static str,
    block: TestBlock,
}

fn known_test_cases() -> Vec<ConformanceCase> {
    vec![
        ConformanceCase {
            test_name: "postTxRejectsButKeepsStateTest",
            block: TestBlock {
                height: 1,
                time: 0,
                txs: vec![make_tx(TxCase {
                    key: vec![1],
                    value: vec![11],
                    gas_limit: 10000,
                    fail_execute: false,
                    reject_post_tx: true,
                })],
            },
        },
        ConformanceCase {
            test_name: "postTxDoesNotMaskExecFailureTest",
            block: TestBlock {
                height: 1,
                time: 0,
                txs: vec![make_tx(TxCase {
                    key: vec![1],
                    value: vec![11],
                    gas_limit: 10000,
                    fail_execute: true,
                    reject_post_tx: true,
                })],
            },
        },
        ConformanceCase {
            test_name: "happyPathTest",
            block: TestBlock {
                height: 1,
                time: 0,
                txs: vec![make_tx(TxCase {
                    key: vec![1],
                    value: vec![11],
                    gas_limit: 10000,
                    fail_execute: false,
                    reject_post_tx: false,
                })],
            },
        },
        ConformanceCase {
            test_name: "mixedPostTxTest",
            block: TestBlock {
                height: 1,
                time: 0,
                txs: vec![
                    make_tx(TxCase {
                        key: vec![1],
                        value: vec![11],
                        gas_limit: 10000,
                        fail_execute: false,
                        reject_post_tx: true,
                    }),
                    make_tx(TxCase {
                        key: vec![2],
                        value: vec![12],
                        gas_limit: 10000,
                        fail_execute: false,
                        reject_post_tx: false,
                    }),
                ],
            },
        },
        ConformanceCase {
            test_name: "outOfGasIgnoresPostTxTest",
            block: TestBlock {
                height: 1,
                time: 0,
                txs: vec![make_tx(TxCase {
                    key: vec![1],
                    value: vec![11],
                    gas_limit: 1,
                    fail_execute: false,
                    reject_post_tx: true,
                })],
            },
        },
    ]
}

#[test]
fn quint_itf_post_tx_conformance() {
    let traces_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../specs/traces");

    let test_cases = known_test_cases();
    for case in &test_cases {
        let trace_file = find_single_trace_file(&traces_dir, case.test_name);
        let trace: ItfTrace = read_itf_trace(&trace_file);
        let spec_state = trace
            .states
            .last()
            .expect("trace must have at least one state");
        let spec_result = &spec_state.last_result;

        let stf = Stf::new(
            NoopBegin::<TestBlock>::default(),
            NoopEnd,
            NoopValidator::<TestTx>::default(),
            RejectingPostTx,
            quint_common::default_gas_config(),
        );

        let mut storage = InMemoryStorage::default();
        let mut codes = CodeStore::new();
        codes.add_code(TestAccount);
        register_account(
            &mut storage,
            AccountId::new(TEST_ACCOUNT_ID),
            "test_account",
        );

        let (real_result, exec_state) = stf.apply_block(&storage, &codes, &case.block);
        storage
            .apply_changes(exec_state.into_changes().unwrap())
            .unwrap();

        assert_eq!(
            real_result.tx_results.len(),
            spec_result.tx_results.len(),
            "{}: tx_results count mismatch",
            case.test_name
        );

        for (i, (real_tx, spec_tx)) in real_result
            .tx_results
            .iter()
            .zip(spec_result.tx_results.iter())
            .enumerate()
        {
            let spec_ok = spec_tx.result.ok;
            let real_ok = real_tx.response.is_ok();
            assert_eq!(real_ok, spec_ok, "{} tx[{i}]: ok mismatch", case.test_name);

            if !spec_ok {
                let spec_err = spec_tx.result.err_code.as_i64();
                let real_err = real_tx.response.as_ref().unwrap_err().id;
                match spec_err {
                    SPEC_ERR_OUT_OF_GAS => assert_eq!(
                        real_err,
                        evolve_stf::ERR_OUT_OF_GAS.id,
                        "{} tx[{i}]: expected OOG",
                        case.test_name
                    ),
                    SPEC_ERR_EXECUTION => assert_eq!(
                        real_err, SPEC_ERR_EXECUTION as u16,
                        "{} tx[{i}]: expected execution error",
                        case.test_name
                    ),
                    SPEC_ERR_POST_TX => assert_eq!(
                        real_err, SPEC_ERR_POST_TX as u16,
                        "{} tx[{i}]: expected post-tx error",
                        case.test_name
                    ),
                    _ => panic!(
                        "{} tx[{i}]: unknown spec error code {spec_err}",
                        case.test_name
                    ),
                }
            }

            assert_eq!(
                real_tx.gas_used,
                spec_tx.gas_used.as_u64(),
                "{} tx[{i}]: gas_used mismatch",
                case.test_name
            );
        }

        assert_eq!(
            real_result.gas_used,
            spec_result.gas_used.as_u64(),
            "{}: block gas mismatch",
            case.test_name
        );

        assert_storage_matches(
            &storage,
            &spec_state.storage,
            AccountId::new(TEST_ACCOUNT_ID),
            case.test_name,
        );
    }
}

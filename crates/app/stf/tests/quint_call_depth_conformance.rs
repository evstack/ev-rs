//! Conformance tests: replay Quint ITF traces for stf_call_depth.qnt.
//!
//! The Quint spec models nested do_exec calls with a call_stack. This
//! conformance test uses a RecursiveAccount that calls itself N times,
//! verifying that the real STF matches the spec's depth enforcement.
//!
//! Run:
//! `quint test --main=stf_call_depth specs/stf_call_depth.qnt --out-itf "specs/traces/out_{test}_{seq}.itf.json"`
//! `cargo test -p evolve_stf --test quint_call_depth_conformance`

use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, FungibleAsset,
    InvokableMessage, InvokeRequest, InvokeResponse, Message, SdkResult,
};
use evolve_stf::gas::StorageGasConfig;
use evolve_stf::Stf;
use evolve_stf_traits::{Block as BlockTrait, PostTxExecution, Transaction, TxValidator};
use serde::Deserialize;
use std::path::Path;

mod quint_common;
use quint_common::{
    account_code_key, find_single_trace_file, read_itf_trace, CodeStore, InMemoryStorage,
    ItfBigInt, NoopBegin, NoopEnd,
};

#[derive(Deserialize)]
struct ItfTrace {
    states: Vec<ItfState>,
}

#[derive(Deserialize)]
struct ItfState {
    #[allow(dead_code)]
    call_stack: Vec<ItfBigInt>,
    last_result: ItfResult,
}

#[derive(Deserialize)]
struct ItfResult {
    ok: bool,
    #[allow(dead_code)]
    err_code: ItfBigInt,
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
struct RecurseMsg {
    remaining: u16,
}

impl InvokableMessage for RecurseMsg {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "recurse";
}

#[derive(Clone, Debug)]
struct TestTx {
    sender: AccountId,
    recipient: AccountId,
    request: InvokeRequest,
    gas_limit: u64,
    funds: Vec<FungibleAsset>,
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
    fn gas_limit(&self) -> u64 {
        1_000_000
    }
}

#[derive(Default)]
struct NoopValidator;
impl TxValidator<TestTx> for NoopValidator {
    fn validate_tx(&self, _tx: &TestTx, _env: &mut dyn Environment) -> SdkResult<()> {
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

#[derive(Default)]
struct RecursiveAccount;

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
        _request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        InvokeResponse::new(&())
    }
    fn execute(
        &self,
        env: &mut dyn Environment,
        request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let msg: RecurseMsg = request.get()?;
        if msg.remaining == 0 {
            return InvokeResponse::new(&());
        }
        let next = RecurseMsg {
            remaining: msg.remaining - 1,
        };
        env.do_exec(env.whoami(), &InvokeRequest::new(&next)?, vec![])?;
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

const RECURSIVE_ACCOUNT: u128 = 100;
const TEST_SENDER: u128 = 200;

/// Maps Quint test names to the recursive depth the Rust test should exercise.
///
/// The Quint spec models individual do_exec/return_from_call steps. The Rust
/// conformance test uses a single recursive account that calls itself N times.
struct ConformanceCase {
    test_name: &'static str,
    requested_depth: u16,
    expect_ok: bool,
}

fn known_test_cases() -> Vec<ConformanceCase> {
    vec![
        // singleCallTest: one do_exec, stack=[1], OK
        ConformanceCase {
            test_name: "singleCallTest",
            requested_depth: 1,
            expect_ok: true,
        },
        // nestedCallsTest: 3 nested calls, stack=[1,2,3], OK
        ConformanceCase {
            test_name: "nestedCallsTest",
            requested_depth: 3,
            expect_ok: true,
        },
        // returnUnwindsStackTest: 2 calls + 1 return, OK (depth 2 succeeds)
        ConformanceCase {
            test_name: "returnUnwindsStackTest",
            requested_depth: 2,
            expect_ok: true,
        },
        // fullUnwindTest: 2 calls + 2 returns, OK (depth 2 succeeds)
        ConformanceCase {
            test_name: "fullUnwindTest",
            requested_depth: 2,
            expect_ok: true,
        },
        // recursiveCallsTest: 3 recursive self-calls, OK
        ConformanceCase {
            test_name: "recursiveCallsTest",
            requested_depth: 3,
            expect_ok: true,
        },
    ]
}

#[test]
fn quint_itf_call_depth_conformance() {
    let traces_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../specs/traces");
    if !traces_dir.exists() {
        panic!(
            "ITF traces not found at {}. Run: quint test --main=stf_call_depth \
             specs/stf_call_depth.qnt \
             --out-itf \"specs/traces/out_{{test}}_{{seq}}.itf.json\"",
            traces_dir.display()
        );
    }

    let test_cases = known_test_cases();
    for case in &test_cases {
        let trace_file = find_single_trace_file(&traces_dir, case.test_name);
        let trace: ItfTrace = read_itf_trace(&trace_file);
        let spec_state = trace
            .states
            .last()
            .expect("trace must have at least one state");
        let spec_result = &spec_state.last_result;

        assert_eq!(
            spec_result.ok, case.expect_ok,
            "{}: expected ok={} but trace says ok={}",
            case.test_name, case.expect_ok, spec_result.ok
        );

        let gas_config = StorageGasConfig {
            storage_get_charge: 1,
            storage_set_charge: 1,
            storage_remove_charge: 1,
        };
        let stf = Stf::new(
            NoopBegin::<TestBlock>::default(),
            NoopEnd,
            NoopValidator,
            NoopPostTx,
            gas_config,
        );

        let mut storage = InMemoryStorage::default();
        let mut codes = CodeStore::new();
        codes.add_code(RecursiveAccount);

        let recursive_account = AccountId::new(RECURSIVE_ACCOUNT);
        let code_id = "recursive".to_string();
        storage.data.insert(
            account_code_key(recursive_account),
            Message::new(&code_id).unwrap().into_bytes().unwrap(),
        );

        let msg = RecurseMsg {
            remaining: case.requested_depth,
        };
        let tx = TestTx {
            sender: AccountId::new(TEST_SENDER),
            recipient: recursive_account,
            request: InvokeRequest::new(&msg).unwrap(),
            gas_limit: 1_000_000,
            funds: vec![],
        };
        let block = TestBlock {
            height: 1,
            time: 0,
            txs: vec![tx],
        };

        let (real_result, _) = stf.apply_block(&storage, &codes, &block);
        assert_eq!(
            real_result.tx_results.len(),
            1,
            "{}: expected one tx result",
            case.test_name
        );

        let real_ok = real_result.tx_results[0].response.is_ok();
        assert_eq!(
            real_ok, case.expect_ok,
            "{}: ok mismatch (real={real_ok}, expected={})",
            case.test_name, case.expect_ok
        );

        if !case.expect_ok {
            let real_err = real_result.tx_results[0]
                .response
                .as_ref()
                .unwrap_err()
                .id;
            assert_eq!(
                real_err,
                evolve_stf::errors::ERR_CALL_DEPTH_EXCEEDED.id,
                "{}: expected call depth error",
                case.test_name
            );
        }

        eprintln!("PASS: {}", case.test_name);
    }
}

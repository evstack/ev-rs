use evolve_authentication::ERR_NOT_EOA;
use evolve_core::runtime_api::{CreateAccountRequest, RUNTIME_ACCOUNT_ID};
use evolve_core::{AccountId, InvokeRequest, Message};
use evolve_fungible_asset::TransferMsg;
use evolve_ns::account::ResolveNameMsg;
use evolve_server_core::WritableKV;
use evolve_stf::gas::GasCounter;
use evolve_testapp::{do_genesis, install_account_codes, TestTx, STF};
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};

const ALICE: AccountId = AccountId::new(65536);
const BOB: AccountId = AccountId::new(65537);

// Helper function to set up common test state
fn setup_test_environment() -> (StorageMock, AccountId, AccountId) {
    let mut codes = AccountStorageMock::new();
    let mut storage = StorageMock::new();

    install_account_codes(&mut codes);

    // do genesis
    let state = do_genesis(&STF, &codes, &storage).unwrap();
    let state_changes = state.into_changes().unwrap();
    storage.apply_changes(state_changes).unwrap();

    // query atom
    let atom_id = STF
        .query(
            &storage,
            &mut codes,
            evolve_ns::GLOBAL_NAME_SERVICE_REF.0,
            &ResolveNameMsg {
                name: "atom".to_string(),
            },
            GasCounter::infinite(),
        )
        .unwrap()
        .get::<Option<AccountId>>()
        .unwrap()
        .unwrap();

    let poa_id = STF
        .query(
            &storage,
            &mut codes,
            evolve_ns::GLOBAL_NAME_SERVICE_REF.0,
            &ResolveNameMsg {
                name: "poa".to_string(),
            },
            GasCounter::infinite(),
        )
        .unwrap()
        .get::<Option<AccountId>>()
        .unwrap()
        .unwrap();

    (storage, atom_id, poa_id)
}

#[test]
fn test_successful_transaction() {
    let (storage, atom_id, _poa_id) = setup_test_environment();
    let codes = {
        let mut codes = AccountStorageMock::new();
        install_account_codes(&mut codes);
        codes
    };

    // create tx of alice sending money to bob
    let ok_tx = TestTx {
        sender: ALICE,
        recipient: atom_id,
        request: InvokeRequest::new(&TransferMsg {
            to: BOB,
            amount: 200,
        })
        .unwrap(),
        gas_limit: 100_000,
        funds: vec![],
    };

    // execute block with successful transaction
    let block = evolve_testapp::block::TestBlock::make_for_testing(vec![ok_tx]);
    let (mut block_results, _new_state) = STF.apply_block(&storage, &codes, &block);

    // extract and verify result
    let ok_result = block_results.tx_results.pop().unwrap();
    assert!(ok_result.response.is_ok(), "{:?}", ok_result.response);
}

#[test]
fn test_not_eoa_transaction() {
    let (storage, atom_id, _poa_id) = setup_test_environment();
    let codes = {
        let mut codes = AccountStorageMock::new();
        install_account_codes(&mut codes);
        codes
    };

    // create a TX failing because not EOA account
    let not_eoa = TestTx {
        // we pretend sender is runtime; which is not EOA, so it cannot send Txs.
        sender: RUNTIME_ACCOUNT_ID,
        recipient: atom_id,
        request: InvokeRequest::new(&CreateAccountRequest {
            code_id: "".to_string(),
            init_message: Message::new(&0).unwrap(),
        })
        .expect("REASON"),
        gas_limit: 500_000,
        funds: vec![],
    };

    // execute block with not EOA transaction
    let block = evolve_testapp::block::TestBlock::make_for_testing(vec![not_eoa]);
    let (mut block_results, _new_state) = STF.apply_block(&storage, &codes, &block);

    // extract and verify result
    let not_eoa_result = block_results.tx_results.pop().unwrap();
    assert_eq!(
        not_eoa_result.response.expect_err("expected an error"),
        ERR_NOT_EOA,
    );
}

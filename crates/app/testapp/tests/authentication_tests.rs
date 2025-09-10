use evolve_authentication::ERR_NOT_EOA;
use evolve_core::runtime_api::{CreateAccountRequest, RUNTIME_ACCOUNT_ID};
use evolve_core::{AccountId, InvokeRequest, Message};
use evolve_fungible_asset::TransferMsg;
use evolve_server_core::WritableKV;
use evolve_stf::SystemAccounts;
use evolve_testapp::{
    build_stf, do_genesis, install_account_codes, GenesisAccounts, TestTx, PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};

const ALICE: AccountId = AccountId::new(65536);
const BOB: AccountId = AccountId::new(65537);

// Helper function to set up common test state
fn setup_test_environment() -> (
    StorageMock,
    AccountStorageMock,
    evolve_testapp::CustomStf,
    GenesisAccounts,
) {
    let mut codes = AccountStorageMock::new();
    let mut storage = StorageMock::new();

    install_account_codes(&mut codes);

    // do genesis
    let bootstrap_stf = build_stf(SystemAccounts::placeholder(), PLACEHOLDER_ACCOUNT);
    let (state, accounts) = do_genesis(&bootstrap_stf, &codes, &storage).unwrap();
    let state_changes = state.into_changes().unwrap();
    storage.apply_changes(state_changes).unwrap();

    let stf = build_stf(
        SystemAccounts::new(accounts.gas_service),
        accounts.scheduler,
    );
    (storage, codes, stf, accounts)
}

#[test]
fn test_successful_transaction() {
    let (storage, codes, stf, accounts) = setup_test_environment();
    let atom_id = accounts.atom;

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
    let (mut block_results, _new_state) = stf.apply_block(&storage, &codes, &block);

    // extract and verify result
    let ok_result = block_results.tx_results.pop().unwrap();
    assert!(ok_result.response.is_ok(), "{:?}", ok_result.response);
}

#[test]
fn test_not_eoa_transaction() {
    let (storage, codes, stf, accounts) = setup_test_environment();
    let atom_id = accounts.atom;

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
    let (mut block_results, _new_state) = stf.apply_block(&storage, &codes, &block);

    // extract and verify result
    let not_eoa_result = block_results.tx_results.pop().unwrap();
    assert_eq!(
        not_eoa_result.response.expect_err("expected an error"),
        ERR_NOT_EOA,
    );
}

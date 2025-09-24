use evolve_core::{AccountId, EnvironmentQuery, InvokeRequest, SdkResult};
use evolve_fungible_asset::TransferMsg;
use evolve_poa::account::Poa;
use evolve_stf::ERR_OUT_OF_GAS;
use evolve_stf_traits::WritableKV;
use evolve_testapp::{
    build_stf, default_gas_config, do_genesis, install_account_codes, GenesisAccounts, TestTx,
    PLACEHOLDER_ACCOUNT,
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

    let gas_config = default_gas_config();
    // do genesis
    let bootstrap_stf = build_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
    let (state, accounts) = do_genesis(&bootstrap_stf, &codes, &storage).unwrap();
    let state_changes = state.into_changes().unwrap();
    storage.apply_changes(state_changes).unwrap();

    let stf = build_stf(gas_config, accounts.scheduler);
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
    let block = evolve_server::Block::for_testing(1, vec![ok_tx]);
    let (mut block_results, _new_state) = stf.apply_block(&storage, &codes, &block);

    // extract and verify result
    let ok_result = block_results.tx_results.pop().unwrap();
    assert!(ok_result.response.is_ok(), "{:?}", ok_result.response);
}

#[test]
fn test_out_of_gas_transaction() {
    let (storage, codes, stf, accounts) = setup_test_environment();
    let atom_id = accounts.atom;

    // create a TX failing because of gas limit
    let out_of_gas_tx = TestTx {
        sender: ALICE,
        recipient: atom_id,
        request: InvokeRequest::new(&TransferMsg {
            to: BOB,
            amount: 200,
        })
        .unwrap(),
        gas_limit: 1500,
        funds: vec![],
    };

    // execute block with out of gas transaction
    let block = evolve_server::Block::for_testing(1, vec![out_of_gas_tx]);
    let (mut block_results, _new_state) = stf.apply_block(&storage, &codes, &block);

    // extract and verify result
    let out_of_gas_result = block_results.tx_results.pop().unwrap();
    assert_eq!(
        out_of_gas_result.response.expect_err("expected an error"),
        ERR_OUT_OF_GAS
    );
}

#[test]
fn test_poa_validator_query() {
    let (storage, codes, stf, accounts) = setup_test_environment();
    let poa_id = accounts.poa;

    // test run as
    stf.run_with_code(
        &storage,
        &codes,
        poa_id,
        |x: &Poa, env: &mut dyn EnvironmentQuery| -> SdkResult<()> {
            let validators = x.get_validator_set(env)?;
            assert_eq!(validators.len(), 0); // TODO: add validators
            Ok(())
        },
    )
    .unwrap();
}

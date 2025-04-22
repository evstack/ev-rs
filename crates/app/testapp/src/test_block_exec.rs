use crate::{do_genesis, install_account_codes, Block, TestAppStf, Tx, ALICE, BOB, STF};
use evolve_core::{AccountId, Environment, InvokeRequest, SdkResult};
use evolve_fungible_asset::TransferMsg;
use evolve_gas::account::ERR_OUT_OF_GAS;
use evolve_ns::account::ResolveNameMsg;
use evolve_poa::account::Poa;
use evolve_server_core::WritableKV;
use evolve_stf::gas::GasCounter;
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};

#[test]
fn test_block_exec() {
    let mut codes = AccountStorageMock::new();
    let mut storage = StorageMock::new();

    install_account_codes(&mut codes);

    // do genesis
    let state = do_genesis(&STF, &mut storage, &mut codes).unwrap();
    let state_changes = state.into_changes().unwrap();
    storage.apply_changes(state_changes).unwrap();

    // query atom
    let atom_id = TestAppStf::query(
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

    let poa_id = TestAppStf::query(
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

    // create tx of alice sending money to bob
    let ok_tx = Tx {
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

    // create a TX failing because of gas limit
    let out_of_gas_tx = Tx {
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

    // execute first block
    let block = Block {
        height: 0,
        txs: vec![ok_tx, out_of_gas_tx],
    };

    let (mut block_results, new_state) = STF.apply_block(&storage, &codes, &block);

    let out_of_gas_result = block_results.tx_results.pop().unwrap();
    let ok_result = block_results.tx_results.pop().unwrap();

    assert!(ok_result.response.is_ok());
    assert_eq!(
        out_of_gas_result.response.expect_err("expected an error"),
        ERR_OUT_OF_GAS
    );

    // test run as
    STF.run_with_code(
        &new_state,
        &codes,
        poa_id,
        |x: &Poa, env: &dyn Environment| -> SdkResult<()> {
            let validators = x.get_validator_set(env)?;
            assert_eq!(validators.len(), 0); // TODO: add validators
            Ok(())
        },
    )
    .unwrap();
}

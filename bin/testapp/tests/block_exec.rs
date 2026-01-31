use evolve_stf::ERR_OUT_OF_GAS;
use evolve_testapp::sim_testing::SimTestApp;

#[test]
fn test_successful_transaction() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    let result = app.submit_transfer_and_produce_block(accounts.alice, accounts.bob, 200, 100_000);
    let ok_result = result.tx_results.first().expect("tx result");
    assert!(ok_result.response.is_ok(), "{:?}", ok_result.response);
}

#[test]
fn test_out_of_gas_transaction() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    let raw_tx = app
        .build_token_transfer_tx(accounts.alice, accounts.atom, accounts.bob, 200, 1500)
        .expect("build tx");
    app.submit_raw_tx(&raw_tx).expect("submit tx");
    let result = app.produce_block_from_mempool(1);
    let out_of_gas_result = result.tx_results.first().expect("tx result");
    match &out_of_gas_result.response {
        Err(err) => assert_eq!(*err, ERR_OUT_OF_GAS),
        Ok(resp) => panic!("expected error, got {:?}", resp),
    }
}

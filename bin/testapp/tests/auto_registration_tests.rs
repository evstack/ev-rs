#![allow(clippy::indexing_slicing)]

use evolve_stf::ERR_OUT_OF_GAS;
use evolve_testapp::sim_testing::SimTestApp;

#[test]
fn test_auto_registration_allows_unregistered_signer() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    // Create a signer without registering the EOA account in storage
    let charlie = app.create_signer_without_account();

    // Fund charlie via minting (minter can send to any account)
    app.mint_atom(charlie, 500);

    // Submit a transfer from charlie (unregistered) to bob
    // Auto-registration should transparently register charlie's EOA
    let result = app.submit_transfer_and_produce_block(charlie, accounts.bob, 100, 100_000);
    let tx_result = result.tx_results.first().expect("tx result");
    assert!(
        tx_result.response.is_ok(),
        "auto-registered EOA should succeed: {:?}",
        tx_result.response
    );
}

#[test]
fn test_pre_registered_accounts_still_work() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    // Alice and Bob are pre-registered in genesis — they should work as before
    let result = app.submit_transfer_and_produce_block(accounts.alice, accounts.bob, 100, 100_000);
    let tx_result = result.tx_results.first().expect("tx result");
    assert!(
        tx_result.response.is_ok(),
        "pre-registered account should still work: {:?}",
        tx_result.response
    );
}

#[test]
fn test_auto_registration_second_tx_works() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    let charlie = app.create_signer_without_account();
    app.mint_atom(charlie, 500);

    // First tx: triggers auto-registration
    let result = app.submit_transfer_and_produce_block(charlie, accounts.bob, 50, 100_000);
    assert!(
        result.tx_results[0].response.is_ok(),
        "first tx should succeed"
    );

    // Second tx: account already registered, should still work
    let result = app.submit_transfer_and_produce_block(charlie, accounts.bob, 50, 100_000);
    assert!(
        result.tx_results[0].response.is_ok(),
        "second tx should succeed with already-registered account"
    );
}

#[test]
fn test_auto_registration_charges_gas() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    let charlie = app.create_signer_without_account();
    app.mint_atom(charlie, 500);

    // Low gas should fail because bootstrap registration now consumes tx gas.
    let raw_tx = app
        .build_token_transfer_tx(charlie, accounts.atom, accounts.bob, 50, 100)
        .expect("build tx");
    app.submit_raw_tx(&raw_tx).expect("submit tx");
    let result = app.produce_block_from_mempool(1);
    let tx_result = result.tx_results.first().expect("tx result");
    match &tx_result.response {
        Err(err) => assert_eq!(*err, ERR_OUT_OF_GAS),
        Ok(resp) => panic!("expected out of gas, got {:?}", resp),
    }
}

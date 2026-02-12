use evolve_authentication::auth_interface::AuthenticationInterfaceRef;
use evolve_authentication::ERR_NOT_EOA;
use evolve_core::Message;
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
fn test_not_eoa_transaction() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    let sender = app.create_signer_with_code("Token");
    let raw_tx = app
        .build_token_transfer_tx(sender, accounts.atom, accounts.bob, 10, 100_000)
        .expect("build tx");
    app.submit_raw_tx(&raw_tx).expect("submit tx");
    let result = app.produce_block_from_mempool(1);
    let not_eoa_result = result.tx_results.first().expect("tx result");
    match &not_eoa_result.response {
        Err(err) => assert_eq!(*err, ERR_NOT_EOA),
        Ok(resp) => panic!("expected error, got {:?}", resp),
    }
}

#[test]
fn test_forged_sender_account_id_payload_rejected() {
    let mut app = SimTestApp::default();
    let accounts = app.accounts();

    // Attempt to authenticate Alice's EOA account with Bob's account ID payload.
    // This simulates a forged auth payload for the fast path and must fail.
    let res = app.system_exec_as(accounts.alice, |env| {
        AuthenticationInterfaceRef::new(accounts.alice)
            .authenticate(Message::new(&accounts.bob)?, env)
    });

    match res {
        Err(err) => assert_eq!(err.id, 0x51),
        Ok(_) => panic!("expected forged sender payload to be rejected"),
    }
}

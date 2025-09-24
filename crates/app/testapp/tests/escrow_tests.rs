use evolve_core::unique_api::UNIQUE_HANDLER_ACCOUNT_ID;
use evolve_core::{AccountId, ERR_UNAUTHORIZED};
use evolve_escrow::escrow::{EscrowRef, ERR_LOCK_NOT_READY};
use evolve_testapp::testing::TestApp;

#[test]
fn test_escrow() {
    let mut app = TestApp::new();
    let escrow_creator = AccountId::new(1000);
    let escrow_money_recipient = AccountId::new(1001);
    let escrow_unauthorized = AccountId::new(1002);
    let unlock_height = 100;

    let minted_money = app.mint_atom(escrow_creator, 1000);
    // create escrow
    let (lock_id, escrow) = app
        .system_exec_as(escrow_creator, |env| {
            // create escrow
            let escrow_ref = EscrowRef::initialize(UNIQUE_HANDLER_ACCOUNT_ID, env)?.0;

            let lock_id = escrow_ref.create_lock(
                vec![minted_money.clone()],
                escrow_money_recipient,
                unlock_height,
                env,
            )?;

            Ok((lock_id, escrow_ref))
        })
        .unwrap();

    // now if we try to withdraw from the lock, it will fail
    // since the height of unlocking has not been reached yet.
    let err = app
        .system_exec_as(escrow_money_recipient, |env| {
            escrow.withdraw_funds(lock_id, env)
        })
        .expect_err("Should fail");
    assert_eq!(err, ERR_LOCK_NOT_READY);

    // try unauthorized
    let err = app
        .system_exec_as(escrow_unauthorized, |env| {
            escrow.withdraw_funds(lock_id, env)
        })
        .expect_err("Should fail");
    assert_eq!(err, ERR_UNAUTHORIZED);

    // now jumping to unlock height all will be good.
    app.go_to_height(unlock_height);
    app.system_exec_as(escrow_money_recipient, |env| {
        escrow.withdraw_funds(lock_id, env)?;
        Ok(())
    })
    .unwrap();
}

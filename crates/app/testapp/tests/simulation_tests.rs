//! Simulation tests that verify system invariants across multiple blocks.

use evolve_fungible_asset::{GetBalanceMsg, TotalSupplyMsg};
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::{
    BalanceAwareTransferGenerator, SimTestApp, TokenTransferGenerator, TxGeneratorRegistry,
};

/// Verifies that transfers preserve total token supply.
/// This is a critical invariant: sum of all balances must equal total supply.
#[test]
fn test_transfer_preserves_total_supply() {
    let mut app = SimTestApp::with_config(SimConfig::default(), 42);
    let accounts = app.accounts();

    // Setup initial balances
    app.mint_atom(accounts.alice, 10_000);
    app.mint_atom(accounts.bob, 10_000);

    // Query initial total supply
    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    // Run transfers
    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        BalanceAwareTransferGenerator::new(
            accounts.atom,
            vec![(accounts.alice, 10_000), (accounts.bob, 10_000)],
            10,
            500,
            100_000,
        ),
    );

    let _ = app.run_blocks_with_registry(50, &mut registry);

    // Verify total supply unchanged
    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    assert_eq!(
        initial_supply, final_supply,
        "Total supply must be preserved across transfers"
    );

    // Verify sum of balances equals total supply
    let alice_balance: Option<u128> = app
        .query(
            accounts.atom,
            &GetBalanceMsg {
                account: accounts.alice,
            },
        )
        .expect("query alice balance");
    let bob_balance: Option<u128> = app
        .query(
            accounts.atom,
            &GetBalanceMsg {
                account: accounts.bob,
            },
        )
        .expect("query bob balance");

    let sum = alice_balance.unwrap_or(0) + bob_balance.unwrap_or(0);
    assert_eq!(sum, final_supply, "Sum of balances must equal total supply");
}

/// Verifies deterministic execution - same seed produces identical state.
#[test]
fn test_deterministic_execution() {
    let seed = 12345u64;

    // First run
    let final_hash_1 = {
        let mut app = SimTestApp::with_config(SimConfig::default(), seed);
        let accounts = app.accounts();
        app.mint_atom(accounts.alice, 5_000);
        app.mint_atom(accounts.bob, 5_000);

        let mut registry = TxGeneratorRegistry::new();
        registry.register(
            100,
            TokenTransferGenerator::new(
                accounts.atom,
                vec![accounts.alice, accounts.bob],
                vec![accounts.alice, accounts.bob],
                1,
                100,
                100_000,
            ),
        );
        let _ = app.run_blocks_with_registry(20, &mut registry);
        app.simulator().storage().state_hash()
    };

    // Second run with same seed
    let final_hash_2 = {
        let mut app = SimTestApp::with_config(SimConfig::default(), seed);
        let accounts = app.accounts();
        app.mint_atom(accounts.alice, 5_000);
        app.mint_atom(accounts.bob, 5_000);

        let mut registry = TxGeneratorRegistry::new();
        registry.register(
            100,
            TokenTransferGenerator::new(
                accounts.atom,
                vec![accounts.alice, accounts.bob],
                vec![accounts.alice, accounts.bob],
                1,
                100,
                100_000,
            ),
        );
        let _ = app.run_blocks_with_registry(20, &mut registry);
        app.simulator().storage().state_hash()
    };

    assert_eq!(
        final_hash_1, final_hash_2,
        "Same seed must produce identical final state"
    );
}

/// Verifies failed transactions don't mutate state.
#[test]
fn test_failed_tx_no_state_mutation() {
    let mut app = SimTestApp::with_config(SimConfig::default(), 99);
    let accounts = app.accounts();

    // Give alice minimal balance
    app.mint_atom(accounts.alice, 100);

    // Capture balance before failed tx
    let alice_balance_before: Option<u128> = app
        .query(
            accounts.atom,
            &GetBalanceMsg {
                account: accounts.alice,
            },
        )
        .expect("query");

    // Attempt transfer exceeding balance
    let request = evolve_core::InvokeRequest::new(&evolve_fungible_asset::TransferMsg {
        to: accounts.bob,
        amount: 1_000_000, // Way more than alice has
    })
    .unwrap();

    let tx = evolve_testapp::TestTx {
        sender: accounts.alice,
        recipient: accounts.atom,
        request,
        gas_limit: 100_000,
        funds: vec![],
    };

    let block = evolve_server::Block::for_testing(app.simulator().time().block_height(), vec![tx]);
    let result = app.apply_block(&block);

    // Transaction should have failed
    assert!(
        result.tx_results[0].response.is_err(),
        "Transfer should fail"
    );

    // State should be unchanged (except for gas/nonce if applicable)
    let alice_balance_after: Option<u128> = app
        .query(
            accounts.atom,
            &GetBalanceMsg {
                account: accounts.alice,
            },
        )
        .expect("query");

    assert_eq!(
        alice_balance_before, alice_balance_after,
        "Failed tx should not change balance"
    );
}

/// Verifies accounts can be created and participate in transfers.
#[test]
fn test_new_accounts_can_receive_and_send() {
    let mut app = SimTestApp::with_config(SimConfig::default(), 777);
    let accounts = app.accounts();

    // Create new EOA
    let charlie = app.create_eoa();

    // Charlie should be able to receive
    app.mint_atom(charlie, 1_000);

    let charlie_balance: Option<u128> = app
        .query(accounts.atom, &GetBalanceMsg { account: charlie })
        .expect("query");
    assert_eq!(charlie_balance, Some(1_000));

    // Charlie should be able to send
    let request = evolve_core::InvokeRequest::new(&evolve_fungible_asset::TransferMsg {
        to: accounts.alice,
        amount: 500,
    })
    .unwrap();

    let tx = evolve_testapp::TestTx {
        sender: charlie,
        recipient: accounts.atom,
        request,
        gas_limit: 100_000,
        funds: vec![],
    };

    let block = evolve_server::Block::for_testing(app.simulator().time().block_height(), vec![tx]);
    let result = app.apply_block(&block);

    assert!(
        result.tx_results[0].response.is_ok(),
        "Charlie should be able to send: {:?}",
        result.tx_results[0].response
    );

    let charlie_after: Option<u128> = app
        .query(accounts.atom, &GetBalanceMsg { account: charlie })
        .expect("query");
    assert_eq!(charlie_after, Some(500));
}

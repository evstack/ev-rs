//! Simulation tests that verify system invariants across multiple blocks.

use evolve_fungible_asset::{GetBalanceMsg, TotalSupplyMsg};
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::{BalanceAwareTransferGenerator, SimTestApp, TxGeneratorRegistry};

/// Seeds for short-lived CI simulations.
const CI_SEED: u64 = 42;

/// Verifies that transfers preserve total token supply.
/// This is a critical invariant: sum of all balances must equal total supply.
#[test]
fn test_transfer_preserves_total_supply() {
    let config = SimConfig {
        max_txs_per_block: 10,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, 42);
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

    let _ = app.run_blocks_with_registry(5, &mut registry);

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

/// Verifies failed transactions don't mutate state.
#[test]
fn test_failed_tx_no_state_mutation() {
    let config = SimConfig {
        max_txs_per_block: 10,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, 99);
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
    let raw_tx = app
        .build_token_transfer_tx(
            accounts.alice,
            accounts.atom,
            accounts.bob,
            1_000_000,
            100_000,
        )
        .expect("build tx");
    app.submit_raw_tx(&raw_tx).expect("submit tx");
    let result = app.produce_block_from_mempool(1);

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
    let config = SimConfig {
        max_txs_per_block: 10,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, 777);
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
    let result = app.submit_transfer_and_produce_block(charlie, accounts.alice, 500, 100_000);

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

/// Tests transfer chains: A->B, B->C, C->D, D->E in sequence.
#[test]
fn test_sequential_transfer_chain() {
    let config = SimConfig {
        max_txs_per_block: 10,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, CI_SEED);
    let accounts = app.accounts();

    let charlie = app.create_eoa();
    let dave = app.create_eoa();
    let eve = app.create_eoa();

    let chain = [accounts.alice, accounts.bob, charlie, dave, eve];

    // Give first account all the funds
    app.mint_atom(chain[0], 10_000);

    let initial_supply: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");

    // Transfer along the chain
    let transfer_amount = 1_000u128;
    for window in chain.windows(2) {
        let from = window[0];
        let to = window[1];

        let result = app.submit_transfer_and_produce_block(from, to, transfer_amount, 100_000);
        assert!(
            result.tx_results[0].response.is_ok(),
            "chain transfer {from:?}->{to:?} should succeed"
        );
    }

    // Verify final state
    let final_supply: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");
    assert_eq!(
        initial_supply, final_supply,
        "chain transfers preserve supply"
    );

    // Last account should have received the transfer
    let eve_balance: Option<u128> = app
        .query(accounts.atom, &GetBalanceMsg { account: eve })
        .expect("query");
    assert_eq!(
        eve_balance,
        Some(transfer_amount),
        "eve should have {transfer_amount}"
    );
}

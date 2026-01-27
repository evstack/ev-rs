//! Simulation tests that verify system invariants across multiple blocks.

use evolve_core::{AccountId, InvokeRequest};
use evolve_fungible_asset::{GetBalanceMsg, TotalSupplyMsg, TransferMsg};
use evolve_server::Block;
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::{
    BalanceAwareTransferGenerator, RoundRobinTransferGenerator, SimTestApp, TokenTransferGenerator,
    TxGeneratorRegistry,
};
use evolve_testapp::TestTx;

/// Seeds used for simulation tests to ensure broad coverage.
const TEST_SEEDS: [u64; 5] = [42, 12345, 99999, 0xDEADBEEF, 1337];

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

/// Tests transfers between Alice, Bob, and multiple dynamically created accounts
/// across multiple seeds for broader coverage.
#[test]
fn test_multi_party_transfers_across_seeds() {
    for seed in TEST_SEEDS {
        run_multi_party_transfer_test(seed);
    }
}

fn run_multi_party_transfer_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
    let accounts = app.accounts();

    // Create additional participants
    let charlie = app.create_eoa();
    let dave = app.create_eoa();
    let eve = app.create_eoa();

    // Setup initial balances
    let initial_amount = 10_000u128;
    app.mint_atom(accounts.alice, initial_amount);
    app.mint_atom(accounts.bob, initial_amount);
    app.mint_atom(charlie, initial_amount);
    app.mint_atom(dave, initial_amount);
    app.mint_atom(eve, initial_amount);

    let all_participants = [accounts.alice, accounts.bob, charlie, dave, eve];
    let initial_balances: Vec<(AccountId, u128)> = all_participants
        .iter()
        .map(|&a| (a, initial_amount))
        .collect();

    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    // Run transfers with balance-aware generator
    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        BalanceAwareTransferGenerator::new(accounts.atom, initial_balances, 10, 1_000, 100_000),
    );

    let results = app.run_blocks_with_registry(100, &mut registry);

    // Verify some transactions were processed
    let total_txs: usize = results.iter().map(|r| r.tx_results.len()).sum();
    assert!(total_txs > 0, "seed {seed}: expected some transactions");

    // Verify total supply preserved
    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(
        initial_supply, final_supply,
        "seed {seed}: total supply must be preserved"
    );

    // Verify sum of balances equals total supply
    let sum: u128 = all_participants
        .iter()
        .map(|&acc| {
            app.query::<_, Option<u128>>(accounts.atom, &GetBalanceMsg { account: acc })
                .expect("query balance")
                .unwrap_or(0)
        })
        .sum();
    assert_eq!(
        sum, final_supply,
        "seed {seed}: sum of balances must equal total supply"
    );
}

/// Tests circular transfer chains (A->B->C->D->E->A) preserve balances.
#[test]
fn test_circular_transfer_chain() {
    for seed in TEST_SEEDS {
        run_circular_transfer_test(seed);
    }
}

fn run_circular_transfer_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
    let accounts = app.accounts();

    // Create chain participants
    let charlie = app.create_eoa();
    let dave = app.create_eoa();
    let eve = app.create_eoa();

    let participants = vec![accounts.alice, accounts.bob, charlie, dave, eve];

    // Give everyone initial balance
    for &p in &participants {
        app.mint_atom(p, 5_000);
    }

    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    // Use round-robin generator for predictable circular transfers
    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        RoundRobinTransferGenerator::new(accounts.atom, participants.clone(), 100, 100_000),
    );

    let _ = app.run_blocks_with_registry(50, &mut registry);

    // Verify supply preserved
    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(
        initial_supply, final_supply,
        "seed {seed}: circular transfers must preserve supply"
    );
}

/// Tests mixed transfer patterns with both successful and failing transactions.
#[test]
fn test_mixed_transfer_patterns() {
    for seed in TEST_SEEDS {
        run_mixed_transfer_test(seed);
    }
}

fn run_mixed_transfer_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
    let accounts = app.accounts();

    let charlie = app.create_eoa();
    let dave = app.create_eoa();

    // Asymmetric balances to create mix of success/failure
    app.mint_atom(accounts.alice, 10_000);
    app.mint_atom(accounts.bob, 5_000);
    app.mint_atom(charlie, 1_000);
    app.mint_atom(dave, 500);

    let all_accounts = vec![accounts.alice, accounts.bob, charlie, dave];

    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    // Random transfer generator (may fail due to insufficient funds)
    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        TokenTransferGenerator::new(
            accounts.atom,
            all_accounts.clone(),
            all_accounts.clone(),
            1,
            2_000, // Can exceed some balances
            100_000,
        ),
    );

    let results = app.run_blocks_with_registry(50, &mut registry);

    // Some transactions should succeed, some may fail
    let successful: usize = results
        .iter()
        .flat_map(|r| &r.tx_results)
        .filter(|tx| tx.response.is_ok())
        .count();
    let failed: usize = results
        .iter()
        .flat_map(|r| &r.tx_results)
        .filter(|tx| tx.response.is_err())
        .count();

    // With asymmetric balances we expect a mix
    assert!(
        successful > 0,
        "seed {seed}: expected some successful transfers"
    );

    // Regardless of failures, supply must be preserved
    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(
        initial_supply, final_supply,
        "seed {seed}: supply preserved despite failures (ok={successful}, fail={failed})"
    );
}

/// Tests that dynamically created accounts can participate in complex transfer flows.
#[test]
fn test_dynamic_account_transfer_flow() {
    for seed in TEST_SEEDS {
        run_dynamic_account_flow_test(seed);
    }
}

fn run_dynamic_account_flow_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
    let accounts = app.accounts();

    // Create several new accounts
    let new_accounts: Vec<AccountId> = (0..5).map(|_| app.create_eoa()).collect();

    // Fund Alice and Bob
    app.mint_atom(accounts.alice, 50_000);
    app.mint_atom(accounts.bob, 50_000);

    // Alice distributes to new accounts
    for &new_acc in &new_accounts {
        let request = InvokeRequest::new(&TransferMsg {
            to: new_acc,
            amount: 5_000,
        })
        .unwrap();

        let tx = TestTx {
            sender: accounts.alice,
            recipient: accounts.atom,
            request,
            gas_limit: 100_000,
            funds: vec![],
        };

        let block = Block::for_testing(app.simulator().time().block_height(), vec![tx]);
        let result = app.apply_block(&block);
        assert!(
            result.tx_results[0].response.is_ok(),
            "seed {seed}: alice->new_acc transfer should succeed"
        );
        app.simulator_mut().advance_block();
    }

    // Verify new accounts received funds
    for &new_acc in &new_accounts {
        let balance: Option<u128> = app
            .query(accounts.atom, &GetBalanceMsg { account: new_acc })
            .expect("query");
        assert_eq!(
            balance,
            Some(5_000),
            "seed {seed}: new account should have 5000"
        );
    }

    // New accounts transfer among themselves and to Bob
    let mut all_participants = new_accounts.clone();
    all_participants.push(accounts.bob);

    let initial_balances: Vec<(AccountId, u128)> = new_accounts
        .iter()
        .map(|&a| (a, 5_000))
        .chain(std::iter::once((accounts.bob, 50_000)))
        .collect();

    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        BalanceAwareTransferGenerator::new(accounts.atom, initial_balances, 10, 500, 100_000),
    );

    let _ = app.run_blocks_with_registry(30, &mut registry);

    // Verify all balances are non-negative and sum correctly
    let total: u128 = all_participants
        .iter()
        .chain(std::iter::once(&accounts.alice))
        .map(|&acc| {
            app.query::<_, Option<u128>>(accounts.atom, &GetBalanceMsg { account: acc })
                .expect("query")
                .unwrap_or(0)
        })
        .sum();

    let expected_total: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");

    assert_eq!(
        total, expected_total,
        "seed {seed}: all balances must sum to total supply"
    );
}

/// Tests deterministic execution across multiple seeds.
#[test]
fn test_deterministic_execution_multi_seed() {
    for seed in TEST_SEEDS {
        run_determinism_check(seed);
    }
}

fn run_determinism_check(seed: u64) {
    // First run
    let hash1 = {
        let mut app = SimTestApp::with_config(SimConfig::default(), seed);
        let accounts = app.accounts();
        let charlie = app.create_eoa();
        let dave = app.create_eoa();

        app.mint_atom(accounts.alice, 10_000);
        app.mint_atom(accounts.bob, 10_000);
        app.mint_atom(charlie, 5_000);
        app.mint_atom(dave, 5_000);

        let mut registry = TxGeneratorRegistry::new();
        registry.register(
            100,
            TokenTransferGenerator::new(
                accounts.atom,
                vec![accounts.alice, accounts.bob, charlie, dave],
                vec![accounts.alice, accounts.bob, charlie, dave],
                1,
                500,
                100_000,
            ),
        );
        let _ = app.run_blocks_with_registry(30, &mut registry);
        app.simulator().storage().state_hash()
    };

    // Second run with identical seed
    let hash2 = {
        let mut app = SimTestApp::with_config(SimConfig::default(), seed);
        let accounts = app.accounts();
        let charlie = app.create_eoa();
        let dave = app.create_eoa();

        app.mint_atom(accounts.alice, 10_000);
        app.mint_atom(accounts.bob, 10_000);
        app.mint_atom(charlie, 5_000);
        app.mint_atom(dave, 5_000);

        let mut registry = TxGeneratorRegistry::new();
        registry.register(
            100,
            TokenTransferGenerator::new(
                accounts.atom,
                vec![accounts.alice, accounts.bob, charlie, dave],
                vec![accounts.alice, accounts.bob, charlie, dave],
                1,
                500,
                100_000,
            ),
        );
        let _ = app.run_blocks_with_registry(30, &mut registry);
        app.simulator().storage().state_hash()
    };

    assert_eq!(hash1, hash2, "seed {seed}: must produce identical state");
}

/// Tests transfer chains: A->B, B->C, C->D, D->E in sequence.
#[test]
fn test_sequential_transfer_chain() {
    for seed in TEST_SEEDS {
        run_sequential_chain_test(seed);
    }
}

fn run_sequential_chain_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
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

        let request = InvokeRequest::new(&TransferMsg {
            to,
            amount: transfer_amount,
        })
        .unwrap();

        let tx = TestTx {
            sender: from,
            recipient: accounts.atom,
            request,
            gas_limit: 100_000,
            funds: vec![],
        };

        let block = Block::for_testing(app.simulator().time().block_height(), vec![tx]);
        let result = app.apply_block(&block);
        assert!(
            result.tx_results[0].response.is_ok(),
            "seed {seed}: chain transfer {from:?}->{to:?} should succeed"
        );
        app.simulator_mut().advance_block();
    }

    // Verify final state
    let final_supply: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");
    assert_eq!(
        initial_supply, final_supply,
        "seed {seed}: chain transfers preserve supply"
    );

    // Last account should have received the transfer
    let eve_balance: Option<u128> = app
        .query(accounts.atom, &GetBalanceMsg { account: eve })
        .expect("query");
    assert_eq!(
        eve_balance,
        Some(transfer_amount),
        "seed {seed}: eve should have {transfer_amount}"
    );
}

/// Tests high-volume transfers with many participants.
#[test]
fn test_high_volume_multi_participant() {
    // Use subset of seeds for this expensive test
    for &seed in &[TEST_SEEDS[0], TEST_SEEDS[2]] {
        run_high_volume_test(seed);
    }
}

fn run_high_volume_test(seed: u64) {
    let mut app = SimTestApp::with_config(SimConfig::default(), seed);
    let accounts = app.accounts();

    // Create many participants
    let extra_accounts: Vec<AccountId> = (0..10).map(|_| app.create_eoa()).collect();

    let mut all_accounts = vec![accounts.alice, accounts.bob];
    all_accounts.extend(&extra_accounts);

    // Fund all accounts
    let initial_balance = 5_000u128;
    for &acc in &all_accounts {
        app.mint_atom(acc, initial_balance);
    }

    let initial_supply: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");

    let initial_balances: Vec<(AccountId, u128)> =
        all_accounts.iter().map(|&a| (a, initial_balance)).collect();

    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        BalanceAwareTransferGenerator::new(accounts.atom, initial_balances, 1, 200, 100_000),
    );

    // Run many blocks
    let results = app.run_blocks_with_registry(200, &mut registry);

    let total_txs: usize = results.iter().map(|r| r.tx_results.len()).sum();
    let successful: usize = results
        .iter()
        .flat_map(|r| &r.tx_results)
        .filter(|tx| tx.response.is_ok())
        .count();

    assert!(
        total_txs > 100,
        "seed {seed}: expected many transactions, got {total_txs}"
    );
    assert!(
        successful > 50,
        "seed {seed}: expected many successful txs, got {successful}"
    );

    // Supply invariant
    let final_supply: u128 = app.query(accounts.atom, &TotalSupplyMsg {}).expect("query");
    assert_eq!(
        initial_supply, final_supply,
        "seed {seed}: high-volume test must preserve supply"
    );
}

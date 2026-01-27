//! Long-running simulations for local verification.
//! These are intentionally ignored for CI runtime.

use evolve_core::AccountId;
use evolve_fungible_asset::{GetBalanceMsg, TotalSupplyMsg};
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::{
    BalanceAwareTransferGenerator, RoundRobinTransferGenerator, SimTestApp, TokenTransferGenerator,
    TxGeneratorRegistry,
};

const LONG_SEED: u64 = 2026;
const LONG_BLOCKS: u64 = 300;
const LONG_ACCOUNTS: usize = 12;
const LONG_GAS_LIMIT: u64 = 100_000;

#[test]
#[ignore]
fn test_multi_party_transfers_long() {
    let config = SimConfig {
        max_txs_per_block: 50,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, LONG_SEED);
    let accounts = app.accounts();

    let mut participants = Vec::with_capacity(LONG_ACCOUNTS);
    participants.push(accounts.alice);
    participants.push(accounts.bob);
    for _ in 0..(LONG_ACCOUNTS.saturating_sub(2)) {
        participants.push(app.create_eoa());
    }

    for account in &participants {
        app.mint_atom(*account, 10_000);
    }

    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    let initial_balances: Vec<(AccountId, u128)> = participants
        .iter()
        .map(|account| (*account, 10_000))
        .collect();

    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        70,
        BalanceAwareTransferGenerator::new(accounts.atom, initial_balances, 1, 200, LONG_GAS_LIMIT),
    );
    registry.register(
        30,
        TokenTransferGenerator::new(
            accounts.atom,
            participants.clone(),
            participants.clone(),
            1,
            50,
            LONG_GAS_LIMIT,
        ),
    );

    let _ = app.run_blocks_with_registry(LONG_BLOCKS, &mut registry);

    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(initial_supply, final_supply, "total supply preserved");

    let mut sum = 0u128;
    for account in &participants {
        let balance: Option<u128> = app
            .query(accounts.atom, &GetBalanceMsg { account: *account })
            .expect("query balance");
        sum = sum.saturating_add(balance.unwrap_or(0));
    }
    assert_eq!(sum, final_supply, "sum of balances matches total supply");
}

#[test]
#[ignore]
fn test_round_robin_circular_transfers_long() {
    let config = SimConfig {
        max_txs_per_block: 50,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, LONG_SEED + 1);
    let accounts = app.accounts();

    let mut participants = Vec::with_capacity(6);
    participants.push(accounts.alice);
    participants.push(accounts.bob);
    for _ in 0..4 {
        participants.push(app.create_eoa());
    }

    for account in &participants {
        app.mint_atom(*account, 5_000);
    }

    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        100,
        RoundRobinTransferGenerator::new(accounts.atom, participants.clone(), 25, LONG_GAS_LIMIT),
    );

    let _ = app.run_blocks_with_registry(200, &mut registry);

    let total_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    let mut sum = 0u128;
    for account in &participants {
        let balance: Option<u128> = app
            .query(accounts.atom, &GetBalanceMsg { account: *account })
            .expect("query balance");
        sum = sum.saturating_add(balance.unwrap_or(0));
    }
    assert_eq!(sum, total_supply, "round-robin preserves supply");
}

#[test]
#[ignore]
fn test_mixed_workload_long() {
    let config = SimConfig {
        max_txs_per_block: 50,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, LONG_SEED + 2);
    let accounts = app.accounts();

    let mut participants = Vec::with_capacity(LONG_ACCOUNTS);
    participants.push(accounts.alice);
    participants.push(accounts.bob);
    for _ in 0..(LONG_ACCOUNTS.saturating_sub(2)) {
        participants.push(app.create_eoa());
    }

    for account in &participants {
        app.mint_atom(*account, 20_000);
    }

    let initial_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");

    let initial_balances: Vec<(AccountId, u128)> = participants
        .iter()
        .map(|account| (*account, 20_000))
        .collect();

    let mut registry = TxGeneratorRegistry::new();
    registry.register(
        50,
        BalanceAwareTransferGenerator::new(accounts.atom, initial_balances, 1, 500, LONG_GAS_LIMIT),
    );
    registry.register(
        30,
        TokenTransferGenerator::new(
            accounts.atom,
            participants.clone(),
            participants.clone(),
            1,
            100,
            LONG_GAS_LIMIT,
        ),
    );
    registry.register(
        20,
        RoundRobinTransferGenerator::new(accounts.atom, participants.clone(), 10, LONG_GAS_LIMIT),
    );

    let _ = app.run_blocks_with_registry(LONG_BLOCKS, &mut registry);

    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(
        initial_supply, final_supply,
        "mixed workload preserves supply"
    );
}

#[test]
#[ignore]
fn test_dynamic_account_creation_long() {
    let config = SimConfig {
        max_txs_per_block: 20,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, LONG_SEED + 3);
    let accounts = app.accounts();

    app.mint_atom(accounts.alice, 50_000);

    const NEW_ACCOUNTS: usize = 8;
    let mut created = Vec::with_capacity(NEW_ACCOUNTS);

    for _ in 0..NEW_ACCOUNTS {
        let account = app.create_eoa();
        created.push(account);
        let result =
            app.submit_transfer_and_produce_block(accounts.alice, account, 1_000, LONG_GAS_LIMIT);
        assert!(
            result.tx_results[0].response.is_ok(),
            "funding new account should succeed"
        );
    }

    for account in &created {
        let result =
            app.submit_transfer_and_produce_block(*account, accounts.bob, 250, LONG_GAS_LIMIT);
        assert!(
            result.tx_results[0].response.is_ok(),
            "new account should be able to send"
        );
    }

    let total_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    let mut sum = 0u128;
    for account in created.iter().chain([accounts.alice, accounts.bob].iter()) {
        let balance: Option<u128> = app
            .query(accounts.atom, &GetBalanceMsg { account: *account })
            .expect("query balance");
        sum = sum.saturating_add(balance.unwrap_or(0));
    }
    assert_eq!(sum, total_supply, "dynamic creation preserves supply");
}

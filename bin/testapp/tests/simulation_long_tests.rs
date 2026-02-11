//! Long-running simulations for local verification.
//! These are intentionally ignored for CI runtime.

use evolve_core::AccountId;
use evolve_fungible_asset::TotalSupplyMsg;
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::{
    DynamicAccountTransferGenerator, SimTestApp, TxGeneratorRegistry,
};

const LONG_SEED: u64 = 2026;
const LONG_BLOCKS: u64 = 300;
const LONG_ACCOUNTS: usize = 12;
const LONG_GAS_LIMIT: u64 = 100_000;

#[test]
#[ignore]
fn test_mixed_workload_long() {
    let config = SimConfig {
        max_txs_per_block: 50,
        ..SimConfig::default()
    };
    let mut app = SimTestApp::with_config(config, LONG_SEED);
    let accounts = app.accounts();
    let initial_signers = app.signer_account_count();

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
        100,
        DynamicAccountTransferGenerator::new(
            accounts.atom,
            accounts.alice,
            initial_balances.clone(),
            1,
            500,
            1_000,
            LONG_GAS_LIMIT,
            10,
            6,
        ),
    );

    let _ = app.run_blocks_with_registry(LONG_BLOCKS, &mut registry);

    let final_supply: u128 = app
        .query(accounts.atom, &TotalSupplyMsg {})
        .expect("query total supply");
    assert_eq!(
        initial_supply, final_supply,
        "mixed workload preserves supply"
    );
    assert!(
        app.signer_account_count() > initial_signers,
        "mixed workload should create additional signer accounts"
    );
}

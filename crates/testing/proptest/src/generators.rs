//! Proptest generators for Evolve SDK types.
//!
//! This module provides composable generators for creating random but valid
//! test inputs like account IDs, transactions, and blocks.

use evolve_core::{AccountId, FungibleAsset};
use proptest::prelude::*;

/// Strategy for generating valid account IDs.
///
/// Account IDs are 128-bit values, but we typically want to test with
/// a smaller range to increase collision probability for relationship testing.
pub fn arb_account_id() -> impl Strategy<Value = AccountId> {
    any::<u128>().prop_map(AccountId::new)
}

/// Strategy for generating account IDs from a fixed pool.
///
/// This is useful for testing interactions between a known set of accounts.
pub fn arb_account_id_from_pool(pool_size: u128) -> impl Strategy<Value = AccountId> {
    (0..pool_size).prop_map(AccountId::new)
}

/// Strategy for generating a vector of unique account IDs.
pub fn arb_account_ids(count: usize) -> impl Strategy<Value = Vec<AccountId>> {
    proptest::collection::vec(any::<u128>(), count).prop_map(|ids| {
        ids.into_iter()
            .enumerate()
            .map(|(i, id)| AccountId::new(id.wrapping_add(i as u128)))
            .collect()
    })
}

/// Strategy for generating token amounts.
///
/// Uses a biased distribution to test both small and large amounts.
pub fn arb_amount() -> impl Strategy<Value = u128> {
    prop_oneof![
        10 => 0u128..1000,           // Small amounts (common case)
        5 => 1000u128..1_000_000,    // Medium amounts
        3 => 1_000_000u128..1_000_000_000, // Large amounts
        2 => Just(u128::MAX),        // Maximum
    ]
}

/// Strategy for generating non-zero amounts.
pub fn arb_nonzero_amount() -> impl Strategy<Value = u128> {
    prop_oneof![
        10 => 1u128..1000,
        5 => 1000u128..1_000_000,
        3 => 1_000_000u128..1_000_000_000,
    ]
}

/// Strategy for generating fungible assets.
pub fn arb_fungible_asset() -> impl Strategy<Value = FungibleAsset> {
    (arb_account_id(), arb_amount())
        .prop_map(|(asset_id, amount)| FungibleAsset { asset_id, amount })
}

/// Strategy for generating fungible assets with a specific asset ID.
pub fn arb_fungible_asset_with_id(asset_id: AccountId) -> impl Strategy<Value = FungibleAsset> {
    arb_amount().prop_map(move |amount| FungibleAsset { asset_id, amount })
}

/// Strategy for generating raw storage keys.
pub fn arb_storage_key() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 1..64)
}

/// Strategy for generating raw storage values.
pub fn arb_storage_value() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..256)
}

/// A transfer operation for testing.
#[derive(Debug, Clone)]
pub struct TestTransfer {
    pub from: AccountId,
    pub to: AccountId,
    pub asset_id: AccountId,
    pub amount: u128,
}

/// Strategy for generating transfer operations.
pub fn arb_transfer(
    accounts: Vec<AccountId>,
    asset_id: AccountId,
) -> impl Strategy<Value = TestTransfer> {
    let accounts_clone = accounts.clone();
    (
        proptest::sample::select(accounts),
        proptest::sample::select(accounts_clone),
        arb_nonzero_amount(),
    )
        .prop_map(move |(from, to, amount)| TestTransfer {
            from,
            to,
            asset_id,
            amount,
        })
}

/// A test transaction wrapper.
#[derive(Debug, Clone)]
pub struct TestTx {
    /// Unique transaction ID.
    pub id: [u8; 32],
    /// Sender account.
    pub sender: AccountId,
    /// Recipient account.
    pub recipient: AccountId,
    /// Transaction type/operation.
    pub operation: TestOperation,
    /// Gas limit for this transaction.
    pub gas_limit: u64,
    /// Nonce (should be monotonically increasing per sender).
    pub nonce: u64,
}

/// Test operation types.
#[derive(Debug, Clone)]
pub enum TestOperation {
    /// Transfer tokens.
    Transfer { asset_id: AccountId, amount: u128 },
    /// Store arbitrary data.
    Store { key: Vec<u8>, value: Vec<u8> },
    /// Delete data.
    Delete { key: Vec<u8> },
    /// No-op (for testing gas only).
    Noop,
}

/// Strategy for generating test operations.
pub fn arb_operation(accounts: Vec<AccountId>) -> impl Strategy<Value = TestOperation> {
    let accounts_for_transfer = accounts.clone();
    prop_oneof![
        5 => (proptest::sample::select(accounts_for_transfer), arb_nonzero_amount())
            .prop_map(|(asset_id, amount)| TestOperation::Transfer { asset_id, amount }),
        3 => (arb_storage_key(), arb_storage_value())
            .prop_map(|(key, value)| TestOperation::Store { key, value }),
        1 => arb_storage_key().prop_map(|key| TestOperation::Delete { key }),
        1 => Just(TestOperation::Noop),
    ]
}

/// Strategy for generating test transactions.
pub fn arb_tx(accounts: Vec<AccountId>) -> impl Strategy<Value = TestTx> {
    let accounts_for_sender = accounts.clone();
    let accounts_for_recipient = accounts.clone();
    let accounts_for_op = accounts;

    (
        proptest::array::uniform32(any::<u8>()),
        proptest::sample::select(accounts_for_sender),
        proptest::sample::select(accounts_for_recipient),
        arb_operation(accounts_for_op),
        1000u64..1_000_000,
        0u64..1000,
    )
        .prop_map(
            |(id, sender, recipient, operation, gas_limit, nonce)| TestTx {
                id,
                sender,
                recipient,
                operation,
                gas_limit,
                nonce,
            },
        )
}

/// A test block containing multiple transactions.
#[derive(Debug, Clone)]
pub struct TestBlock {
    /// Block height.
    pub height: u64,
    /// Block timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Transactions in this block.
    pub txs: Vec<TestTx>,
}

/// Strategy for generating test blocks.
pub fn arb_block(
    accounts: Vec<AccountId>,
    height: u64,
    max_txs: usize,
) -> impl Strategy<Value = TestBlock> {
    let timestamp = height * 1000; // Simple timestamp calculation

    proptest::collection::vec(arb_tx(accounts), 0..=max_txs).prop_map(move |txs| TestBlock {
        height,
        timestamp_ms: timestamp,
        txs,
    })
}

/// Strategy for generating a sequence of blocks.
pub fn arb_block_sequence(
    accounts: Vec<AccountId>,
    num_blocks: usize,
    max_txs_per_block: usize,
) -> impl Strategy<Value = Vec<TestBlock>> {
    let strategies: Vec<_> = (0..num_blocks)
        .map(|i| arb_block(accounts.clone(), i as u64, max_txs_per_block))
        .collect();

    strategies
        .into_iter()
        .fold(Just(Vec::new()).boxed(), |acc, block_strategy| {
            (acc, block_strategy)
                .prop_map(|(mut blocks, block)| {
                    blocks.push(block);
                    blocks
                })
                .boxed()
        })
}

/// Configuration for test scenario generation.
#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    /// Number of accounts in the test.
    pub num_accounts: usize,
    /// Number of blocks to generate.
    pub num_blocks: usize,
    /// Maximum transactions per block.
    pub max_txs_per_block: usize,
    /// Token asset ID for transfers.
    pub token_asset_id: AccountId,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        Self {
            num_accounts: 10,
            num_blocks: 10,
            max_txs_per_block: 20,
            token_asset_id: AccountId::new(1),
        }
    }
}

/// A complete test scenario.
#[derive(Debug, Clone)]
pub struct TestScenario {
    /// Seed for reproducibility.
    pub seed: u64,
    /// All accounts in the test.
    pub accounts: Vec<AccountId>,
    /// Initial balances for each account.
    pub initial_balances: Vec<(AccountId, u128)>,
    /// Blocks to execute.
    pub blocks: Vec<TestBlock>,
    /// Configuration used.
    pub config: ScenarioConfig,
}

/// Strategy for generating complete test scenarios.
pub fn arb_scenario(config: ScenarioConfig) -> impl Strategy<Value = TestScenario> {
    let num_accounts = config.num_accounts;
    let num_blocks = config.num_blocks;
    let max_txs = config.max_txs_per_block;
    let token = config.token_asset_id;

    (
        any::<u64>(),
        arb_account_ids(num_accounts),
        proptest::collection::vec(arb_amount(), num_accounts),
    )
        .prop_flat_map(move |(seed, accounts, balances)| {
            let initial_balances: Vec<_> = accounts.iter().cloned().zip(balances).collect();

            let accounts_clone = accounts.clone();
            arb_block_sequence(accounts_clone, num_blocks, max_txs).prop_map(move |blocks| {
                TestScenario {
                    seed,
                    accounts: accounts.clone(),
                    initial_balances: initial_balances.clone(),
                    blocks,
                    config: ScenarioConfig {
                        num_accounts,
                        num_blocks,
                        max_txs_per_block: max_txs,
                        token_asset_id: token,
                    },
                }
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::strategy::ValueTree;

    proptest! {
        #[test]
        fn test_account_id_generation(id in arb_account_id()) {
            // Account IDs should always be valid
            let _ = id.as_bytes();
        }

        #[test]
        fn test_tx_generation(tx in arb_tx(vec![AccountId::new(1), AccountId::new(2), AccountId::new(3)])) {
            prop_assert!(tx.gas_limit >= 1000);
            prop_assert!(tx.gas_limit < 1_000_000);
        }

        #[test]
        fn test_block_generation(block in arb_block(
            vec![AccountId::new(1), AccountId::new(2)],
            5,
            10
        )) {
            prop_assert_eq!(block.height, 5);
            prop_assert!(block.txs.len() <= 10);
        }
    }

    #[test]
    fn test_scenario_generation() {
        let config = ScenarioConfig {
            num_accounts: 3,
            num_blocks: 2,
            max_txs_per_block: 5,
            ..Default::default()
        };

        let strategy = arb_scenario(config);

        // Generate a few scenarios and verify structure
        for _ in 0..3 {
            let scenario = strategy
                .new_tree(&mut proptest::test_runner::TestRunner::default())
                .unwrap()
                .current();

            assert_eq!(scenario.accounts.len(), 3);
            assert_eq!(scenario.blocks.len(), 2);
            assert!(scenario.blocks.iter().all(|b| b.txs.len() <= 5));
        }
    }
}

---
name: evolve-proptest
description: Write property-based tests for Evolve SDK using proptest. Use when writing property tests, invariant testing, fuzz testing, test generators, or finding edge cases.
---


# Evolve Property-Based Testing

The `evolve_proptest` crate provides property-based testing infrastructure for Evolve SDK, including transaction generators, system invariants, and test case shrinking.

## Key Features

1. **Transaction generators**: Generate random valid transactions for testing
2. **System invariants**: Define rules that must always hold
3. **Shrinking**: Automatically find minimal failing test cases
4. **Scenario testing**: Generate complex multi-block test scenarios

## Basic Usage

```rust
use evolve_proptest::{
    generators::{arb_account_id, arb_tx, arb_block, arb_scenario},
    invariants::{Invariant, InvariantChecker, BalanceConservation, NonNegativeBalances},
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_balance_conservation(
        block in arb_block(test_accounts(), 0, 10)
    ) {
        // Execute block
        let state = execute_block(block);

        // Check invariant
        let checker = InvariantChecker::new()
            .with(BalanceConservation);

        let results = checker.check_all(&state);
        for result in results {
            prop_assert!(result.passed, "Invariant violated: {}", result.name);
        }
    }
}
```

## Transaction Generators

### Account IDs

```rust
use evolve_proptest::generators::arb_account_id;

// Generate random account IDs
proptest! {
    #[test]
    fn test_with_account(account in arb_account_id()) {
        assert!(account.as_bytes().len() == 16);
    }
}
```

### Transactions

```rust
use evolve_proptest::generators::arb_tx;

let accounts = vec![AccountId::new(1), AccountId::new(2), AccountId::new(3)];

proptest! {
    #[test]
    fn test_with_tx(tx in arb_tx(accounts.clone())) {
        // tx has valid sender from accounts
        // tx has valid amount > 0
        // tx has valid gas limit
    }
}
```

### Blocks

```rust
use evolve_proptest::generators::arb_block;

// Generate block at height 5 with up to 10 transactions
proptest! {
    #[test]
    fn test_with_block(
        block in arb_block(accounts.clone(), 5, 10)
    ) {
        assert_eq!(block.height, 5);
        assert!(block.transactions.len() <= 10);
    }
}
```

### Scenarios

```rust
use evolve_proptest::generators::{arb_scenario, ScenarioConfig};

let config = ScenarioConfig {
    num_accounts: 5,
    num_blocks: 10,
    max_txs_per_block: 5,
    initial_balance: 1_000_000,
};

proptest! {
    #[test]
    fn test_scenario(scenario in arb_scenario(config.clone())) {
        // scenario.accounts - list of accounts
        // scenario.initial_balances - account -> balance map
        // scenario.blocks - list of blocks to execute
    }
}
```

## System Invariants

### Built-in Invariants

```rust
use evolve_proptest::invariants::{
    BalanceConservation,
    NonNegativeBalances,
    NonceMonotonicity,
};

let checker = InvariantChecker::new()
    .with(BalanceConservation)
    .with(NonNegativeBalances)
    .with(NonceMonotonicity);

let results = checker.check_all(&state);
```

### Custom Invariants

```rust
use evolve_proptest::invariants::{Invariant, InvariantResult};
use evolve_core::ReadonlyKV;

struct NoEmptyAccounts;

impl Invariant for NoEmptyAccounts {
    fn name(&self) -> &str {
        "no_empty_accounts"
    }

    fn check(&self, state: &dyn ReadonlyKV) -> InvariantResult {
        // Check that no account has zero balance
        // Return InvariantResult::passed() or InvariantResult::violated(msg)
        InvariantResult::passed("no_empty_accounts")
    }
}

let checker = InvariantChecker::new()
    .with(NoEmptyAccounts);
```

## Test Case Shrinking

When a property test fails, the shrinker finds the minimal failing case:

```rust
use evolve_proptest::shrinker::FailureShrink;

fn run_test(scenario: &TestScenario) -> Result<(), String> {
    // Run scenario and check invariants
}

let shrink = FailureShrink::new(
    initial_seed,
    failing_scenario,
    "balance_conservation",
    |scenario| run_test(scenario).is_err(),
);

let minimal = shrink.shrink();
println!("Minimal failing case:");
println!("  Seed: {}", minimal.seed);
println!("  Blocks: {}", minimal.blocks.len());
println!("  Transactions: {}", minimal.total_transactions());
```

## Property Test Runner

```rust
use evolve_proptest::PropertyTestRunner;
use evolve_simulator::Simulator;

let runner = PropertyTestRunner::new()
    .with_config(proptest_config)
    .with_invariants(checker);

let result = runner.run(|scenario| {
    let mut sim = Simulator::new(scenario.seed, SimConfig::default());

    // Execute scenario
    for block in &scenario.blocks {
        execute_block(&mut sim, block)?;
    }

    Ok(())
});

if let Err(failure) = result {
    println!("Property violated!");
    println!("Minimal case: {:?}", failure.minimal);
    println!("Reproduce: evolve-sim run --seed {}", failure.seed);
}
```

## Best Practices

1. **Start with simple invariants**: Balance conservation, no negative values
2. **Use realistic generators**: Generate transactions that could occur in production
3. **Keep test cases small**: Smaller cases are easier to debug
4. **Save failing seeds**: Always capture seeds for reproduction
5. **Run many iterations**: Property tests find edge cases with volume

## Environment-Based Configuration

Configure test iterations via environment variables:

```rust
fn get_proptest_cases() -> u32 {
    if std::env::var("CI").is_ok() {
        1000  // More cases in CI
    } else if let Ok(cases) = std::env::var("EVOLVE_PROPTEST_CASES") {
        cases.parse().unwrap_or(100)
    } else {
        100  // Default for local development
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(get_proptest_cases()))]
    #[test]
    fn my_property_test(input in arb_input()) {
        // ...
    }
}
```

## Model-Based Testing

Test implementation against a simplified model:

```rust
use std::collections::HashMap;

struct TokenModel {
    balances: HashMap<AccountId, u128>,
    total_supply: u128,
}

impl TokenModel {
    fn transfer(&mut self, from: AccountId, to: AccountId, amount: u128) -> bool {
        if let Some(balance) = self.balances.get_mut(&from) {
            if *balance >= amount {
                *balance -= amount;
                *self.balances.entry(to).or_insert(0) += amount;
                return true;
            }
        }
        false
    }
}

proptest! {
    #[test]
    fn token_matches_model(ops in prop::collection::vec(arb_transfer_op(), 0..100)) {
        let mut model = TokenModel::new();
        let mut app = TestApp::new();

        for op in ops {
            let model_result = model.transfer(op.from, op.to, op.amount);
            let impl_result = app.system_exec_as(op.from, |env| {
                TokenRef::from(token).transfer(op.to, op.amount, env)
            });

            prop_assert_eq!(model_result, impl_result.is_ok());
        }

        // Verify final state matches
        for (account, expected_balance) in model.balances {
            let actual = app.system_exec_as(account, |env| {
                TokenRef::from(token).get_balance(account, env)
            }).unwrap();
            prop_assert_eq!(actual, Some(expected_balance));
        }
    }
}
```

## Token-Specific Invariants

```rust
let runner = PropertyTestRunner::new()
    .with_token_invariants(asset_id, initial_supply);

// This adds:
// - BalanceConservation: sum(balances) == total_supply
// - NonNegativeBalances: all balances >= 0
// - TotalSupplyMatch: total_supply storage == sum(balances)
```

## Integration with Simulator

```rust
use evolve_proptest::generators::arb_scenario;
use evolve_simulator::{Simulator, SimConfig};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    #[test]
    fn fuzz_test(scenario in arb_scenario(config.clone())) {
        let mut sim = Simulator::new(scenario.seed, SimConfig::default());

        // Apply initial state
        for (account, balance) in &scenario.initial_balances {
            // Set up account balance
        }

        // Execute blocks
        for block in &scenario.blocks {
            // Execute each transaction
        }

        // Check invariants
        let results = checker.check_all(sim.storage());
        for result in results {
            prop_assert!(result.passed);
        }
    }
}
```

## Files

- `crates/testing/proptest/src/lib.rs` - Main exports
- `crates/testing/proptest/src/generators.rs` - Test data generators
- `crates/testing/proptest/src/invariants.rs` - System invariants
- `crates/testing/proptest/src/shrinker.rs` - Failing case minimization

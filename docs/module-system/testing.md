# Testing Modules

Evolve provides a `MockEnv` for unit testing modules without running a full blockchain.

## Basic Setup

```rust
use evolve_core::{AccountId, Environment, EnvironmentQuery};
use evolve_testing::MockEnv;
use crate::account::MyModule;

#[test]
fn test_basic_operation() {
    // Create mock environment
    // - First arg: contract's account ID (whoami)
    // - Second arg: sender's account ID
    let contract_id = AccountId::new(1);
    let sender_id = AccountId::new(100);
    let mut env = MockEnv::new(contract_id, sender_id);

    // Create module instance
    let module = MyModule::default();

    // Initialize
    module.initialize(42, &mut env).expect("init failed");

    // Test operations
    let result = module.increment(&mut env).expect("increment failed");
    assert_eq!(result, 43);
}
```

## MockEnv Builder Pattern

```rust
let mut env = MockEnv::new(contract_id, sender_id)
    .with_sender(new_sender)           // Change sender
    .with_funds(vec![fungible_asset])  // Attach funds
    .with_account_balance(account, token, amount);  // Set balances
```

## Testing Authorization

```rust
#[test]
fn test_unauthorized_access() {
    let contract_id = AccountId::new(1);
    let owner_id = AccountId::new(100);
    let attacker_id = AccountId::new(999);

    // Initialize as owner
    let mut env = MockEnv::new(contract_id, owner_id);
    let module = MyModule::default();
    module.initialize(&mut env).unwrap();

    // Try to call as attacker
    env = env.with_sender(attacker_id);
    let result = module.admin_only_function(&mut env);

    assert!(matches!(result, Err(e) if e == ERR_UNAUTHORIZED));
}
```

## Testing Payable Functions

```rust
#[test]
fn test_deposit() {
    let contract_id = AccountId::new(1);
    let depositor_id = AccountId::new(100);
    let token_id = AccountId::new(50);

    let mut env = MockEnv::new(contract_id, depositor_id)
        .with_funds(vec![FungibleAsset {
            asset_id: token_id,
            amount: 1000,
        }]);

    let module = MyModule::default();
    module.deposit(&mut env).expect("deposit failed");

    // Verify funds were handled
    assert_eq!(env.funds().len(), 0); // Funds consumed
}
```

## Testing Inter-Account Calls

```rust
#[test]
fn test_cross_module_call() {
    let contract_id = AccountId::new(1);
    let sender_id = AccountId::new(100);
    let token_id = AccountId::new(50);

    let mut env = MockEnv::new(contract_id, sender_id);

    // MockEnv tracks inter-account calls
    // You can verify calls were made or mock responses

    let module = MyModule::default();
    module.transfer_to_another(&mut env).expect("failed");
}
```

## Testing Error Conditions

```rust
#[test]
fn test_insufficient_balance() {
    let mut env = setup_env();
    let module = MyModule::default();

    // Set up account with 100 tokens
    module.initialize(vec![(sender, 100)], &mut env).unwrap();

    // Try to transfer 1000 tokens
    let result = module.transfer(recipient, 1000, &mut env);

    assert!(matches!(result, Err(e) if e == ERR_NOT_ENOUGH_BALANCE));
}

#[test]
fn test_overflow_protection() {
    let mut env = setup_env();
    let module = MyModule::default();

    // Set balance near u128::MAX
    module.set_balance(u128::MAX - 10, &mut env).unwrap();

    // Adding 100 should overflow
    let result = module.add_balance(100, &mut env);

    assert!(matches!(result, Err(e) if e == ERR_OVERFLOW));
}
```

## Property-Based Testing

Use `proptest` for exhaustive testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_transfer_preserves_total(
        balance1 in 0u128..1_000_000,
        balance2 in 0u128..1_000_000,
        transfer_amount in 0u128..1_000_000,
    ) {
        let mut env = setup_env();
        let module = MyModule::default();

        // Initialize with total = balance1 + balance2
        module.initialize(
            vec![(account1, balance1), (account2, balance2)],
            &mut env
        ).unwrap();

        let total_before = balance1 + balance2;

        // Attempt transfer (may fail if amount > balance)
        let _ = module.transfer(account1, account2, transfer_amount, &mut env);

        // Total should be preserved regardless
        let b1 = module.get_balance(account1, &mut env).unwrap().unwrap_or(0);
        let b2 = module.get_balance(account2, &mut env).unwrap().unwrap_or(0);

        prop_assert_eq!(b1 + b2, total_before);
    }
}
```

## Integration Testing

For full-stack tests, use the test harness:

```rust
use evolve_testapp::TestApp;

#[test]
fn test_full_block_execution() {
    let app = TestApp::new();

    // Execute a block with transactions
    let result = app.execute_block(vec![
        transaction1,
        transaction2,
    ]);

    assert!(result.is_ok());
    assert_eq!(result.tx_results.len(), 2);
}
```

## Best Practices

1. **Test both success and failure paths**
2. **Use property-based testing for arithmetic operations**
3. **Test authorization for all privileged functions**
4. **Test edge cases (zero, max values, empty collections)**
5. **Test determinism by running tests multiple times**

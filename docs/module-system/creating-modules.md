# Creating a Module

Step-by-step guide to creating a new Evolve module.

## 1. Create the Crate

```bash
cargo new --lib crates/app/sdk/extensions/my_module
```

Add to workspace `Cargo.toml`:

```toml
[workspace]
members = [
    # ...
    "crates/app/sdk/extensions/my_module",
]

[workspace.dependencies]
evolve_my_module = { path = "crates/app/sdk/extensions/my_module" }
```

## 2. Set Up Dependencies

In `crates/app/sdk/extensions/my_module/Cargo.toml`:

```toml
[package]
name = "evolve_my_module"
version = "0.1.0"
edition = "2021"
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]
evolve_core.workspace = true
evolve_collections.workspace = true
evolve_macros.workspace = true
borsh = { version = "1.5", features = ["derive"] }

[dev-dependencies]
evolve_testing.workspace = true

[lints]
workspace = true
```

## 3. Define the Module

In `src/lib.rs`:

```rust
use evolve_core::account_impl;

#[account_impl(MyModule)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{
        define_error, AccountId, Environment, EnvironmentQuery, SdkResult, ERR_UNAUTHORIZED,
    };
    use evolve_macros::{exec, init, query};

    // Define module-specific errors
    define_error!(ERR_NOT_FOUND, 0x01, "item not found");
    define_error!(ERR_ALREADY_EXISTS, 0x02, "item already exists");

    // Define the state struct
    #[derive(evolve_core::AccountState)]
    pub struct MyModule {
        #[storage(0)]
        pub owner: Item<AccountId>,
        #[storage(1)]
        pub counter: Item<u64>,
        #[storage(2)]
        pub data: Map<AccountId, String>,
    }

    impl MyModule {
        /// Initialize the module. Called once when account is created.
        #[init]
        pub fn initialize(
            &self,
            initial_counter: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // Set the owner to whoever created this account
            self.owner.set(&env.sender(), env)?;
            self.counter.set(&initial_counter, env)?;
            Ok(())
        }

        /// Increment the counter. Only owner can call.
        #[exec]
        pub fn increment(&self, env: &mut dyn Environment) -> SdkResult<u64> {
            // Check authorization
            let owner = self.owner.get(env)?;
            if env.sender() != owner {
                return Err(ERR_UNAUTHORIZED);
            }

            // Update counter
            let current = self.counter.get(env)?;
            let new_value = current.checked_add(1).ok_or(evolve_core::ERR_OVERFLOW)?;
            self.counter.set(&new_value, env)?;

            Ok(new_value)
        }

        /// Store data for an account.
        #[exec]
        pub fn set_data(
            &self,
            key: AccountId,
            value: String,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.data.set(&key, &value, env)?;
            Ok(())
        }

        /// Query the current counter value.
        #[query]
        pub fn get_counter(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
            self.counter.get(env)
        }

        /// Query data for an account.
        #[query]
        pub fn get_data(
            &self,
            key: AccountId,
            env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<Option<String>> {
            self.data.may_get(&key, env)
        }
    }
}
```

## 4. Add Tests

```rust
#[cfg(test)]
mod tests {
    use super::account::MyModule;
    use evolve_core::{AccountId, ERR_UNAUTHORIZED};
    use evolve_testing::MockEnv;

    fn setup() -> (MyModule, MockEnv) {
        let contract_id = AccountId::new(1);
        let owner_id = AccountId::new(100);
        let mut env = MockEnv::new(contract_id, owner_id);
        let module = MyModule::default();
        
        module.initialize(0, &mut env).expect("init failed");
        
        (module, env)
    }

    #[test]
    fn test_initialize() {
        let (module, mut env) = setup();
        
        let counter = module.get_counter(&mut env).unwrap();
        assert_eq!(counter, 0);
    }

    #[test]
    fn test_increment_authorized() {
        let (module, mut env) = setup();
        
        let new_value = module.increment(&mut env).unwrap();
        assert_eq!(new_value, 1);
    }

    #[test]
    fn test_increment_unauthorized() {
        let (module, mut env) = setup();
        
        // Change sender to non-owner
        env = env.with_sender(AccountId::new(999));
        
        let result = module.increment(&mut env);
        assert!(matches!(result, Err(e) if e == ERR_UNAUTHORIZED));
    }

    #[test]
    fn test_data_storage() {
        let (module, mut env) = setup();
        
        let key = AccountId::new(42);
        module.set_data(key, "hello".to_string(), &mut env).unwrap();
        
        let value = module.get_data(key, &mut env).unwrap();
        assert_eq!(value, Some("hello".to_string()));
    }
}
```

## 5. Register in Test App

To use the module in the test application, add it to code registration:

```rust
// In bin/testapp/src/lib.rs
pub fn install_account_codes(codes: &mut impl WritableAccountsCodeStorage) {
    codes.add_code(Token::new()).unwrap();
    codes.add_code(MyModule::new()).unwrap();  // Add your module
    // ...
}
```

## 6. Verify

```bash
# Build
cargo build -p evolve_my_module

# Test
cargo test -p evolve_my_module

# Check for warnings
cargo clippy -p evolve_my_module
```

## Module Patterns

### Payable Functions

```rust
#[exec(payable)]
pub fn deposit(&self, env: &mut dyn Environment) -> SdkResult<u128> {
    let funds = env.funds();
    // Process deposited funds...
    Ok(total_deposited)
}
```

### Cross-Module Calls

```rust
use evolve_token::account::TokenRef;

#[exec]
pub fn transfer_tokens(
    &self,
    token_id: AccountId,
    recipient: AccountId,
    amount: u128,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    let token = TokenRef::new(token_id);
    token.transfer(recipient, amount, env)?;
    Ok(())
}
```

### Lifecycle Hooks

```rust
use evolve_scheduler::begin_block_account_interface::BeginBlockAccountInterface;

impl BeginBlockAccountInterface for MyModule {
    #[exec]
    fn do_begin_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
        // Called at the start of each block
        Ok(())
    }
}
```

## Common Pitfalls

1. **Forgetting to check authorization** - Always verify `env.sender()`
2. **Using `unwrap()` on optional values** - Use `ok_or(ERR_...)?` instead
3. **Duplicate storage prefixes** - The compiler will catch this
4. **Non-deterministic code** - See [Determinism Requirements](./determinism.md)
5. **Missing overflow checks** - Use `checked_add`, `checked_sub`, etc.

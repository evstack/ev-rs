---
description: How to write modules for the Evolve SDK
triggers:
  - "write module"
  - "create module"
  - "account code"
  - "account_impl"
  - "module development"
  - "new module"
---

# Writing Evolve Modules

Modules in Evolve are stateless code executors that implement the `AccountCode` trait. They interact with blockchain state through the `Environment` interface and are composed via the `#[account_impl]` macro.

## Module Structure

A minimal module:

```rust
use evolve_core::account_impl;

#[account_impl(MyModule)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{AccountId, Environment, EnvironmentQuery, SdkResult};
    use evolve_macros::{exec, init, query};

    pub struct MyModule {
        pub owner: Item<AccountId>,
        pub data: Map<AccountId, u64>,
    }

    impl Default for MyModule {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MyModule {
        pub const fn new() -> Self {
            Self {
                owner: Item::new(0),  // prefix 0
                data: Map::new(1),    // prefix 1
            }
        }

        #[init]
        pub fn initialize(
            &self,
            owner: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.owner.set(&owner, env)?;
            Ok(())
        }

        #[exec]
        pub fn set_data(
            &self,
            key: AccountId,
            value: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.data.set(&key, &value, env)?;
            Ok(())
        }

        #[query]
        pub fn get_data(
            &self,
            key: AccountId,
            env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<Option<u64>> {
            self.data.may_get(&key, env)
        }
    }
}
```

## Function Markers

| Marker | Environment | Purpose | Generated Message |
|--------|-------------|---------|-------------------|
| `#[init]` | `&mut dyn Environment` | One-time initialization | `InitializeMsg` |
| `#[exec]` | `&mut dyn Environment` | State mutations | `<FnName>Msg` |
| `#[query]` | `&mut dyn EnvironmentQuery` | Read-only | `<FnName>Msg` |
| `#[payable]` | Add to `#[exec]` | Accept fungible assets | - |

## Storage Collections

Use unique prefix bytes to avoid key collisions:

```rust
use evolve_collections::{item::Item, map::Map, vector::Vector, queue::Queue};

pub struct MyModule {
    pub config: Item<Config>,           // prefix 0, single value
    pub balances: Map<AccountId, u128>, // prefix 1, key-value
    pub history: Vector<Event>,         // prefix 2, indexed list
    pub pending: Queue<Task>,           // prefix 3, FIFO queue
}
```

### Storage Operations

```rust
// Item<T>
item.set(&value, env)?;           // write
item.get(env)?;                   // read (panics if missing)
item.may_get(env)?;               // read -> Option<T>
item.update(|v| Ok(v + 1), env)?; // atomic update

// Map<K, V>
map.set(&key, &value, env)?;
map.get(&key, env)?;              // panics if missing
map.may_get(&key, env)?;          // -> Option<V>
map.remove(&key, env)?;
map.update(&key, |v| Ok(v.unwrap_or(0) + 1), env)?;

// Vector<T>
vector.push(&value, env)?;
vector.get(index, env)?;
vector.len(env)?;
vector.pop(env)?;
```

## Error Handling

Define custom errors with `define_error!`:

```rust
use evolve_core::{define_error, ERR_UNAUTHORIZED};

define_error!(ERR_INSUFFICIENT_BALANCE, 0x1, "insufficient balance");
define_error!(ERR_ALREADY_EXISTS, 0x2, "already exists");

#[exec]
pub fn withdraw(&self, amount: u128, env: &mut dyn Environment) -> SdkResult<()> {
    let balance = self.balances.may_get(&env.sender(), env)?.unwrap_or(0);
    if balance < amount {
        return Err(ERR_INSUFFICIENT_BALANCE);
    }
    // ...
}
```

## Authorization Patterns

Check sender for privileged operations:

```rust
#[exec]
pub fn admin_action(&self, env: &mut dyn Environment) -> SdkResult<()> {
    if env.sender() != self.owner.get(env)? {
        return Err(ERR_UNAUTHORIZED);
    }
    // privileged logic
    Ok(())
}
```

For system-only operations:

```rust
use evolve_core::RUNTIME_ACCOUNT_ID;

#[exec]
pub fn system_only(&self, env: &mut dyn Environment) -> SdkResult<()> {
    if env.sender() != RUNTIME_ACCOUNT_ID {
        return Err(ERR_UNAUTHORIZED);
    }
    Ok(())
}
```

## Internal vs External Functions

Separate authorization from logic:

```rust
// Internal: no auth checks
pub fn mint_unchecked(
    &self,
    recipient: AccountId,
    amount: u128,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    self.balances.update(&recipient, |b| Ok(b.unwrap_or(0) + amount), env)?;
    self.total_supply.update(|s| Ok(s.unwrap_or(0) + amount), env)?;
    Ok(())
}

// External: with auth
#[exec]
pub fn mint(
    &self,
    recipient: AccountId,
    amount: u128,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    if self.supply_manager.get(env)? != Some(env.sender()) {
        return Err(ERR_UNAUTHORIZED);
    }
    self.mint_unchecked(recipient, amount, env)
}
```

## Cross-Module Calls

The macro generates a `Ref` wrapper for type-safe calls:

```rust
use evolve_token::account::TokenRef;

#[exec]
pub fn do_transfer(&self, env: &mut dyn Environment) -> SdkResult<()> {
    let token = TokenRef::from(self.token_id.get(env)?);
    token.transfer(recipient, amount, env)?;
    Ok(())
}
```

Or use raw `InvokeRequest` for flexibility:

```rust
use evolve_core::InvokeRequest;
use evolve_fungible_asset::TransferMsg;

let request = InvokeRequest::new(&TransferMsg { to: recipient, amount })?;
env.do_exec(token_id, &request, vec![])?;
```

## Implementing Interfaces

Implement standard interfaces for composability:

```rust
use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};

impl FungibleAssetInterface for Token {
    #[exec]
    fn transfer(&self, to: AccountId, amount: u128, env: &mut dyn Environment) -> SdkResult<()> {
        // implementation
    }

    #[query]
    fn get_balance(&self, account: AccountId, env: &mut dyn EnvironmentQuery) -> SdkResult<Option<u128>> {
        self.balances.may_get(&account, env)
    }

    #[query]
    fn metadata(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<FungibleAssetMetadata> {
        self.metadata.get(env)
    }

    #[query]
    fn total_supply(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u128> {
        self.total_supply.get(env)
    }
}
```

## Testing Modules

### Unit Tests with MockEnv

```rust
#[cfg(test)]
mod tests {
    use super::account::MyModule;
    use evolve_core::AccountId;
    use evolve_testing::MockEnv;

    #[test]
    fn test_basic_flow() {
        let contract = AccountId::new(1);
        let sender = AccountId::new(2);
        let mut env = MockEnv::new(contract, sender);

        let module = MyModule::default();
        module.initialize(sender, &mut env).unwrap();

        module.set_data(AccountId::new(3), 42, &mut env).unwrap();

        let value = module.get_data(AccountId::new(3), &mut env).unwrap();
        assert_eq!(value, Some(42));
    }

    #[test]
    fn test_unauthorized() {
        let contract = AccountId::new(1);
        let owner = AccountId::new(2);
        let mut env = MockEnv::new(contract, owner);

        let module = MyModule::default();
        module.initialize(owner, &mut env).unwrap();

        // Change sender to non-owner
        env = env.with_sender(AccountId::new(999));

        let result = module.admin_action(&mut env);
        assert!(result.is_err());
    }
}
```

### Integration Tests with TestApp

```rust
use testapp::{TestApp, GenesisAccounts};

#[test]
fn test_with_full_stf() {
    let mut app = TestApp::new();
    let accounts = app.accounts();

    app.system_exec_as(accounts.alice, |env| {
        // Interact with modules through refs
        let token = TokenRef::from(accounts.atom);
        token.transfer(accounts.bob, 100, env)
    }).unwrap();

    app.next_block();
}
```

## Checklist for New Modules

1. **Unique storage prefixes** - Each field gets unique byte prefix
2. **Implement Default** - Required for code registration
3. **Authorization on exec functions** - Check `env.sender()` appropriately
4. **Use checked arithmetic** - `checked_add`, `checked_sub` to prevent overflow
5. **Handle missing values** - Use `may_get()` and handle `None`
6. **Write unit tests** - Test with MockEnv for fast iteration
7. **Write integration tests** - Test with TestApp for full STF coverage

## Files

- `crates/app/sdk/macros/src/lib.rs` - The `#[account_impl]` macro
- `crates/app/sdk/core/src/lib.rs` - Core traits (`AccountCode`, `Environment`)
- `crates/app/sdk/collections/src/` - Storage collections
- `crates/app/sdk/x/token/src/lib.rs` - Reference implementation (Token)
- `crates/app/sdk/x/escrow/src/lib.rs` - Another example (Escrow)

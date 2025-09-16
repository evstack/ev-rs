# Evolve Module System

The Evolve SDK uses an account-centric module system where every module is an account implementing the `AccountCode` trait. This guide covers architecture, best practices, and common pitfalls.

## Table of Contents

1. [Architecture Overview](./architecture.md)
2. [Creating a Module](./creating-modules.md)
3. [Storage Collections](./storage.md)
4. [Determinism Requirements](./determinism.md)
5. [Error Handling](./errors.md)
6. [Testing](./testing.md)

## Quick Start

```rust
use evolve_core::{account_impl, AccountState};
use evolve_collections::{item::Item, map::Map};

#[account_impl(MyModule)]
pub mod account {
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_macros::{exec, init, query};

    #[derive(evolve_core::AccountState)]
    pub struct MyModule {
        #[storage(0)]
        pub counter: Item<u64>,
        #[storage(1)]
        pub balances: Map<AccountId, u128>,
    }

    impl MyModule {
        #[init]
        pub fn initialize(&self, initial_value: u64, env: &mut dyn Environment) -> SdkResult<()> {
            self.counter.set(&initial_value, env)?;
            Ok(())
        }

        #[exec]
        pub fn increment(&self, env: &mut dyn Environment) -> SdkResult<u64> {
            let current = self.counter.get(env)?;
            let new_value = current + 1;
            self.counter.set(&new_value, env)?;
            Ok(new_value)
        }

        #[query]
        pub fn get_counter(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
            self.counter.get(env)
        }
    }
}
```

## Key Concepts

| Concept | Description |
|---------|-------------|
| `AccountCode` | Core trait all modules implement |
| `Environment` | Mutable execution context (can write state) |
| `EnvironmentQuery` | Read-only query context |
| `#[account_impl]` | Macro that generates message routing |
| `#[derive(AccountState)]` | Generates storage initialization with validation |
| `#[storage(n)]` | Assigns unique storage prefix to a field |

## Core Traits

### Environment vs EnvironmentQuery

- **`EnvironmentQuery`**: Read-only operations (`query` functions)
  - `whoami()` - Current account ID
  - `sender()` - Transaction sender
  - `funds()` - Attached funds
  - `do_query()` - Call another account's query

- **`Environment`**: Extends `EnvironmentQuery` with write operations (`init`, `exec` functions)
  - `do_exec()` - Execute on another account (can modify state)

### Function Attributes

| Attribute | Purpose | Mutates State | Payable |
|-----------|---------|---------------|---------|
| `#[init]` | Account initialization | Yes | `#[init(payable)]` |
| `#[exec]` | State-mutating operations | Yes | `#[exec(payable)]` |
| `#[query]` | Read-only queries | No | Never |

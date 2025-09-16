# Error Handling

Evolve uses error codes for compact, deterministic error handling.

## Defining Errors

Use the `define_error!` macro:

```rust
use evolve_core::define_error;

define_error!(ERR_NOT_ENOUGH_BALANCE, 0x01, "not enough balance");
define_error!(ERR_UNAUTHORIZED, 0x02, "unauthorized");
define_error!(ERR_OVERFLOW, 0x03, "arithmetic overflow");
```

## Error Code Namespacing

To prevent collisions between modules, use namespaced error codes:

```rust
// Recommended: Use module-specific base
const MODULE_BASE: u16 = 0x0100;  // Each module gets a range

define_error!(ERR_MY_ERROR, MODULE_BASE | 0x01, "my error");
define_error!(ERR_ANOTHER, MODULE_BASE | 0x02, "another error");
```

### Reserved Ranges

| Range | Module |
|-------|--------|
| `0x0000-0x00FF` | Core SDK errors |
| `0x0100-0x01FF` | Token module |
| `0x0200-0x02FF` | Gas module |
| `0x0300-0x03FF` | Scheduler module |
| `0x0400-0x04FF` | Available for custom modules |

## Core SDK Errors

From `evolve_core`:

| Error | Code | Description |
|-------|------|-------------|
| `ERR_ENCODING` | `0x01` | Borsh encoding/decoding failed |
| `ERR_UNKNOWN_FUNCTION` | `0x02` | Function ID not found |
| `ERR_ACCOUNT_NOT_INIT` | `0x03` | Account not initialized |
| `ERR_UNAUTHORIZED` | `0x04` | Sender not authorized |
| `ERR_NOT_PAYABLE` | `0x05` | Function doesn't accept funds |
| `ERR_ONE_COIN` | `0x06` | Expected exactly one coin |
| `ERR_INCOMPATIBLE_FA` | `0x07` | Incompatible fungible asset |
| `ERR_INSUFFICIENT_BALANCE` | `0x08` | Insufficient balance |
| `ERR_OVERFLOW` | `0x09` | Arithmetic overflow |

## Using the `ensure!` Macro

For conditional error returns:

```rust
use evolve_core::ensure;

fn transfer(&self, to: AccountId, amount: u128, env: &mut dyn Environment) -> SdkResult<()> {
    let balance = self.balances.get(&env.sender(), env)?;
    
    // Returns Err(ERR_INSUFFICIENT_BALANCE) if condition is false
    ensure!(balance >= amount, ERR_INSUFFICIENT_BALANCE);
    
    // ... proceed with transfer
    Ok(())
}
```

## Error Propagation

Use the `?` operator for clean propagation:

```rust
fn complex_operation(&self, env: &mut dyn Environment) -> SdkResult<()> {
    // Each ? propagates errors up the call stack
    let value = self.storage.get(env)?;
    let result = self.other_account.query(env)?;
    self.storage.set(&new_value, env)?;
    Ok(())
}
```

## Handling Optional Values

Prefer explicit error handling over `unwrap()`:

```rust
// BAD - panics if not found
let value = self.data.may_get(env)?.unwrap();

// GOOD - returns error if not found
let value = self.data.may_get(env)?.ok_or(ERR_NOT_FOUND)?;

// GOOD - use default
let value = self.data.may_get(env)?.unwrap_or_default();
```

## Transaction Atomicity

Errors automatically trigger rollback via checkpoint/restore:

```rust
fn multi_step_operation(&self, env: &mut dyn Environment) -> SdkResult<()> {
    // Step 1: Succeeds
    self.step_one(env)?;
    
    // Step 2: Fails
    self.step_two(env)?; // Returns error
    
    // Step 1 is automatically rolled back
    // No partial state changes are committed
}
```

## Error Decoding (Development)

Enable the `error-decode` feature for human-readable errors:

```toml
[dependencies]
evolve_core = { workspace = true, features = ["error-decode"] }
```

This changes `SdkResult<T>` from `Result<T, ErrorCode>` to `Result<T, DecodedError>` with full error messages.

## Best Practices

1. **Use namespaced error codes** - Prevent collisions with other modules
2. **Prefer `ensure!`** - Cleaner than manual if/return
3. **Never use `unwrap()`** - Use `ok_or(ERR_...)` or `unwrap_or_default()`
4. **Document error conditions** - Add doc comments explaining when errors occur
5. **Keep error messages short** - They're stored in the binary

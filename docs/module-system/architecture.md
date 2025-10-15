# Module System Architecture

## Overview

The Evolve module system uses an account-centric model where every piece of application logic is an account. Accounts have:

1. **Identity** - Unique `AccountId` (u128)
2. **Code** - Implementation of `AccountCode` trait
3. **State** - Isolated storage namespace

## Core Components

```
┌─────────────────────────────────────────────────────────────┐
│                      State Transition Function (STF)         │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │                      ExecutionState                      │ │
│  │  - Overlay (write cache)                                 │ │
│  │  - Undo log (for rollback)                              │ │
│  │  - Events                                                │ │
│  └─────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │                        Invoker                           │ │
│  │  - Implements Environment/EnvironmentQuery               │ │
│  │  - Manages call stack (max depth: 64)                   │ │
│  │  - Handles fund transfers                                │ │
│  └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     AccountCode Registry                     │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐         │
│  │  Token  │  │   Gas   │  │Scheduler│  │   EOA   │  ...    │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘         │
└─────────────────────────────────────────────────────────────┘
```

## Execution Flow

### Block Processing

```
BeginBlock
    │
    ├── BeginBlocker accounts execute
    │   └── e.g., Scheduler, PoA validator updates
    │
    ▼
For each Transaction:
    │
    ├── 1. Validate (TxValidator)
    │   └── Check signature, nonce
    │
    ├── 2. Execute (AccountCode.execute)
    │   └── State changes in overlay
    │
    ├── 3. Post-TX Handler
    │   └── Fee collection, logging
    │
    └── 4. Commit or Rollback
        └── Based on success/failure
    │
    ▼
EndBlock
    │
    └── EndBlocker accounts execute
```

### Inter-Account Calls

```rust
// Account A calls Account B
env.do_exec(account_b_id, &message, funds)?;

// Internally:
// 1. Invoker creates checkpoint
// 2. Funds are transferred (if any)
// 3. Account B's code executes
// 4. On error: checkpoint restored
// 5. On success: changes committed
```

## System Accounts

Reserved `AccountId` values with special behavior:

| ID | Name | Purpose |
|----|------|---------|
| 0 | `RUNTIME_ACCOUNT_ID` | Account creation, migrations |
| 1 | `STORAGE_ACCOUNT_ID` | Storage read/write operations |
| 2 | `EVENT_HANDLER_ACCOUNT_ID` | Event emission |
| 5 | `UNIQUE_HANDLER_ACCOUNT_ID` | Unique ID generation |

These accounts have hardcoded handlers in the invoker.

## Message Dispatch

The `#[account_impl]` macro generates:

1. **Message structs** for each function
2. **Function IDs** from SHA-256 of function name
3. **Dispatch logic** in `AccountCode::execute/query`

```
InvokeRequest { function_id, payload }
         │
         ▼
┌─────────────────────────┐
│  AccountCode::execute   │
│  match function_id {    │
│    TransferMsg::ID =>   │──▶ Token::transfer()
│    MintMsg::ID =>       │──▶ Token::mint()
│    _ => ERR_UNKNOWN     │
│  }                      │
└─────────────────────────┘
```

## State Management

### Checkpoint/Restore

Every `do_exec` call creates a checkpoint:

```rust
let checkpoint = state.checkpoint();

match execute_call() {
    Ok(result) => result,
    Err(e) => {
        state.restore(checkpoint)?;  // Rollback all changes
        return Err(e);
    }
}
```

### Storage Isolation

Each account's storage is prefixed by its `AccountId`:

```
[AccountId][FieldPrefix][Key] => Value

Example:
[0x0A][0x01][alice] => 1000  // Account 10, field 1, key "alice"
```

## Lifecycle Hooks

Modules can implement lifecycle traits:

```rust
pub trait BeginBlocker<B> {
    fn begin_block(&self, block: &B, env: &mut dyn Environment);
}

pub trait EndBlocker {
    fn end_block(&self, env: &mut dyn Environment);
}

pub trait TxValidator<Tx> {
    fn validate_tx(&self, tx: &Tx, env: &mut dyn Environment) -> SdkResult<()>;
}

pub trait PostTxExecution<Tx> {
    fn after_tx_executed(
        tx: &Tx,
        gas_consumed: u64,
        tx_result: Result<...>,
        env: &mut dyn Environment,
    ) -> SdkResult<()>;
}
```

## Resource Limits

| Limit | Value | Purpose |
|-------|-------|---------|
| Max call depth | 64 | Prevent stack overflow |
| Max overlay entries | 100,000 | Memory bound |
| Max events | 10,000 | Memory bound |
| Max key size | 254 bytes | Storage efficiency |
| Max value size | 1 MB | Storage efficiency |

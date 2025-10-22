# State Transition Function (STF) Architecture

## Overview

The STF is the core execution engine for Evolve. It processes blocks of transactions deterministically, transforming state from one consistent snapshot to the next.

**Design Principles:**

- Sequential, deterministic execution (no tx parallelism)
- Atomic transactions with checkpoint/rollback
- Single-threaded block processing
- Lazy system configuration loading

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CONSENSUS LAYER                                 │
│  ┌─────────────┐   ┌─────────────────┐   ┌──────────────────────────────┐  │
│  │   Block     │   │ Parallel Sig    │   │     State Commit             │  │
│  │  Producer   │──▶│  Verification   │──▶│  (apply_changes to ADB)     │  │
│  └─────────────┘   └─────────────────┘   └──────────────────────────────┘  │
│         │                   │                         ▲                     │
│         ▼                   ▼                         │                     │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                         STF.apply_block()                             │  │
│  │  ┌────────────────────────────────────────────────────────────────┐  │  │
│  │  │                        Block Processing                         │  │  │
│  │  │                                                                  │  │  │
│  │  │  ┌──────────────┐                                               │  │  │
│  │  │  │ BEGIN_BLOCK  │ BeginBlocker (scheduler, etc.)                │  │  │
│  │  │  │  (infinite   │ Load gas config from GasService               │  │  │
│  │  │  │   gas)       │                                               │  │  │
│  │  │  └──────┬───────┘                                               │  │  │
│  │  │         │                                                        │  │  │
│  │  │         ▼                                                        │  │  │
│  │  │  ┌──────────────────────────────────────────────────────────┐   │  │  │
│  │  │  │              FOR EACH TRANSACTION (sequential)            │   │  │  │
│  │  │  │  ┌─────────────────────────────────────────────────────┐ │   │  │  │
│  │  │  │  │ 1. VALIDATE                                         │ │   │  │  │
│  │  │  │  │    - Create Invoker (validation mode)               │ │   │  │  │
│  │  │  │  │    - TxValidator.validate_tx()                      │ │   │  │  │
│  │  │  │  │    - On error: return TxResult with error           │ │   │  │  │
│  │  │  │  └─────────────────────┬───────────────────────────────┘ │   │  │  │
│  │  │  │                        │ (same ExecutionState)           │   │  │  │
│  │  │  │                        ▼                                 │   │  │  │
│  │  │  │  ┌─────────────────────────────────────────────────────┐ │   │  │  │
│  │  │  │  │ 2. EXECUTE                                          │ │   │  │  │
│  │  │  │  │    - Transform to run context (into_new_run)        │ │   │  │  │
│  │  │  │  │    - Checkpoint before changes                      │ │   │  │  │
│  │  │  │  │    - handle_transfers() (fund movement)             │ │   │  │  │
│  │  │  │  │    - invoke() (account code execution)              │ │   │  │  │
│  │  │  │  │    - On error: restore checkpoint                   │ │   │  │  │
│  │  │  │  └─────────────────────┬───────────────────────────────┘ │   │  │  │
│  │  │  │                        │                                 │   │  │  │
│  │  │  │                        ▼                                 │   │  │  │
│  │  │  │  ┌─────────────────────────────────────────────────────┐ │   │  │  │
│  │  │  │  │ 3. POST-TX HANDLER                                  │ │   │  │  │
│  │  │  │  │    - PostTxExecution.after_tx_executed()            │ │   │  │  │
│  │  │  │  │    - Fee deduction, logging, etc.                   │ │   │  │  │
│  │  │  │  │    - Can reject tx after execution                  │ │   │  │  │
│  │  │  │  └─────────────────────┬───────────────────────────────┘ │   │  │  │
│  │  │  │                        │                                 │   │  │  │
│  │  │  │                        ▼                                 │   │  │  │
│  │  │  │  ┌─────────────────────────────────────────────────────┐ │   │  │  │
│  │  │  │  │ Collect TxResult { events, gas_used, response }     │ │   │  │  │
│  │  │  │  └─────────────────────────────────────────────────────┘ │   │  │  │
│  │  │  └──────────────────────────────────────────────────────────┘   │  │  │
│  │  │         │                                                        │  │  │
│  │  │         ▼                                                        │  │  │
│  │  │  ┌──────────────┐                                               │  │  │
│  │  │  │  END_BLOCK   │ EndBlocker (scheduler, etc.)                  │  │  │
│  │  │  │  (infinite   │                                               │  │  │
│  │  │  │   gas)       │                                               │  │  │
│  │  │  └──────────────┘                                               │  │  │
│  │  └────────────────────────────────────────────────────────────────┘  │  │
│  │                                                                       │  │
│  │  Output: BlockResult + ExecutionState.into_changes()                 │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Component Relationships

```
┌─────────────────────┐      ┌─────────────────────────────────────────┐
│        Stf          │      │            ExecutionState               │
│  ┌───────────────┐  │      │  ┌─────────────────────────────────┐   │
│  │ begin_blocker │  │      │  │  base_storage: &S (immutable)   │   │
│  │ end_blocker   │  │      │  │  overlay: HashMap<key, Option>  │   │
│  │ tx_validator  │  │      │  │  undo_log: Vec<StateChange>     │   │
│  │ post_tx_handler│ │      │  │  events: Vec<Event>             │   │
│  │ system_accounts│ │      │  │  unique_objects: u64            │   │
│  └───────────────┘  │      │  │  metrics: ExecutionMetrics      │   │
└─────────────────────┘      │  └─────────────────────────────────┘   │
         │                    │                                        │
         │ creates            │  Checkpoint = { undo_log_idx,          │
         ▼                    │                 events_idx,            │
┌─────────────────────┐      │                 unique_objects }       │
│      Invoker        │      └─────────────────────────────────────────┘
│  ┌───────────────┐  │                         ▲
│  │ whoami        │  │      borrows mutably     │
│  │ sender        │  │─────────────────────────┘
│  │ funds         │  │
│  │ account_codes │  │      ┌─────────────────────────────────────────┐
│  │ storage ──────│──┼─────▶│            GasCounter                   │
│  │ gas_counter ──│──┼─────▶│  Infinite | Finite { limit, used, cfg } │
│  │ scope         │  │      └─────────────────────────────────────────┘
│  │ call_depth    │  │
│  └───────────────┘  │      ┌─────────────────────────────────────────┐
│                     │      │          ExecutionScope                  │
│  Implements:        │      │  BeginBlock(height)                      │
│  - EnvironmentQuery │      │  Transaction(tx_id)                      │
│  - Environment      │      │  Query                                   │
└─────────────────────┘      │  EndBlock(height)                        │
                             └─────────────────────────────────────────┘
```

---

## Threading Model

### What's Single-Threaded

| Component             | Why                                           |
|-----------------------|-----------------------------------------------|
| Block execution       | Determinism requires sequential tx processing |
| Transaction execution | Transactions modify shared ExecutionState     |
| Checkpoint/restore    | Undo log assumes single-writer                |
| Event collection      | Sequential append, truncate on rollback       |

### What Can Be Parallel

| Component              | Where         | Mechanism             |
|------------------------|---------------|-----------------------|
| Signature verification | Pre-consensus | rayon::par_iter()     |
| Cache reads            | Storage layer | 256-shard RwLock      |
| ADB access             | Storage layer | tokio::RwLock (async) |

### Send/Sync Bounds Analysis

| Trait/Type        | Send | Sync | Why                                   |
|-------------------|------|------|---------------------------------------|
| ReadonlyKV        | No   | No   | Removed - only warm_cache needs +Sync |
| AccountCode       | Yes  | Yes  | Stored in shared registry             |
| ExecutionState    | No   | No   | Contains &mut references              |
| Invoker           | No   | No   | Contains &mut references              |
| CommonwareStorage | Yes  | Yes  | Needs cross-thread access             |

---

## Key Invariants

### 1. Atomic Transactions

Every do_invoke() call is atomic:

```rust
let checkpoint = self.storage.checkpoint();
// ... run ...
if error {
    self.storage.restore(checkpoint);  // Roll back all changes
}
```

### 2. Validation-Execution Atomicity

Validation and execution share the same ExecutionState - no TOCTOU issues.

### 3. Deterministic Execution

- Same block + same state = same result
- No randomness, no time-dependent logic
- Sequential transaction processing

### 4. Resource Limits

| Limit               | Value     | Purpose                   |
|---------------------|-----------|---------------------------|
| Max overlay entries | 100,000   | Prevent memory exhaustion |
| Max events          | 10,000    | Bound event log size      |
| Max key size        | 254 bytes | Limit key storage         |
| Max value size      | 1 MB      | Limit value storage       |
| Max call depth      | 64        | Prevent stack overflow    |

---

## Storage Layer

```
ExecutionState.overlay
       │
       │ into_changes()
       ▼
Vec<StateChange>
  - Set { key, value }
  - Remove { key }
       │
       │ apply to storage
       ▼
CommonwareStorage.adb
       │
       │ commit
       ▼
Merkle Root (state hash)
```

---

## System Accounts

| Account        | ID | Purpose                      |
|----------------|----|------------------------------|
| RUNTIME        | 0  | Account creation, migration  |
| STORAGE        | 1  | Key-value storage operations |
| EVENT_HANDLER  | 2  | Event emission               |
| UNIQUE_HANDLER | 3  | Unique ID generation         |

---

## Gas Model

- **Infinite gas**: BeginBlock, EndBlock, system operations
- **Finite gas**: User transactions
- **Config loaded**: From GasService account at block start
- **Charges**: Per-byte for reads, writes, and deletes

---

## Testing Status

- [x] Call depth limit exhaustion test (`test_call_depth_exhaustion`)
- [x] Post-tx handler rejection test (`test_post_tx_handler_rejection`)
- [x] Determinism test (`prop_determinism`)
- [x] State changes collection (`test_state_changes_collection`)
- [ ] Nested call gas exhaustion
- [x] State change ordering determinism (sorted `into_changes()` output)

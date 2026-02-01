# Evolve SDK - Claude Code Instructions

## Self-Improvement Directive

**Claude agents MUST update this file** when:
- A repeated correction or better approach emerges
- New patterns are discovered that should be codified
- Architectural decisions are made that affect future development
- Testing patterns or debugging techniques prove useful

Add learned patterns to the "Learned Patterns" section at the bottom.

---

## Project Overview

Evolve is a modular blockchain SDK written in Rust. Key characteristics:
- **Account Model**: All execution is account-driven
- **Determinism First**: Non-deterministic types (HashMap, SystemTime) are forbidden
- **Ethereum Compatible**: Full EIP-2718 transactions and JSON-RPC support
- **Macro-Driven**: Account development uses procedural macros to reduce boilerplate

---

## Codebase Structure

```
/crates/
├── app/
│   ├── sdk/           # Core SDK components
│   │   ├── core/      # Environment, AccountCode, errors, messages
│   │   ├── macros/    # #[account_impl], #[exec], #[query], etc.
│   │   ├── collections/ # Item, Map, UnorderedMap, Vector, Queue
│   │   ├── stf_traits/  # Transaction, Block, TxValidator traits
│   │   ├── testing/   # MockEnv, test utilities
│   │   ├── math/      # Fixed-point arithmetic (FP64)
│   │   ├── standards/ # fungible_asset, authentication interfaces
│   │   └── x/         # Pre-built modules (token, scheduler)
│   ├── stf/           # State Transition Function implementation
│   ├── tx/eth/        # EIP-2718 transactions, ECDSA, RLP
│   ├── tx/micro/      # Minimal transaction type
│   ├── mempool/       # Thread-safe transaction pool
│   ├── server/        # DevConsensus, block building, persistence
│   ├── node/          # Dev node runner
│   └── genesis/       # Genesis file handling
├── rpc/
│   ├── types/         # Ethereum-compatible RPC types
│   ├── eth-jsonrpc/   # JSON-RPC 2.0 server
│   ├── grpc/          # gRPC server
│   ├── evnode/        # Node-specific RPC handlers
│   └── chain-index/   # Block indexing
├── storage/           # RocksDB-backed persistent storage
└── testing/
    ├── simulator/     # Deterministic simulation with fault injection
    ├── proptest/      # Property testing utilities
    ├── debugger/      # Execution tracing and replay
    └── cli/           # CLI testing tools

/bin/
├── testapp/           # Test application binary and integration tests
└── evd/               # Production node binary
```

---

## Build Commands (justfile)

Always check available commands with `just --list`. Key commands:

| Command | Purpose |
|---------|---------|
| `just build` | Build all crates |
| `just check` | Fast type checking |
| `just fmt` | Format all code |
| `just lint` | Run clippy with deny warnings |
| `just quality` | fmt-check + lint (run before commits) |
| `just test` | Run all tests |
| `just test-pkg <name>` | Test specific package |
| `just test-one <name>` | Run single test by name |
| `just sim` | Run simulation tests |
| `just sim-seed <seed>` | Reproduce simulation with seed |
| `just bench` | Run criterion benchmarks |
| `just doc` | Build documentation |

---

## Writing Rust Code for Evolve

### Determinism Rules (Enforced by Clippy)

**Forbidden types** (see `.clippy.toml`):
- `std::collections::HashMap` / `HashSet` → Use `BTreeMap` / `BTreeSet`
- `std::time::Instant` / `SystemTime` → Use block time from context

**Why**: Non-deterministic iteration order or time sources break consensus.

### Account Implementation Pattern

```rust
use evolve_core::prelude::*;
use evolve_collections::*;

#[account_impl(MyAccount)]
impl MyAccount {
    #[storage(path = "owner")]
    owner: Item<AccountId>,

    #[storage(path = "balances")]
    balances: Map<AccountId, u64>,

    #[init]
    fn init(env: &impl Environment, initial_owner: AccountId) -> Result<(), ErrorCode> {
        self.owner.set(env, initial_owner)?;
        Ok(())
    }

    #[exec]
    fn transfer(
        env: &impl Environment,
        to: AccountId,
        amount: u64,
    ) -> Result<(), ErrorCode> {
        let from = env.sender();
        // Implementation...
        Ok(())
    }

    #[query]
    fn balance(env: &impl EnvironmentQuery, account: AccountId) -> Result<u64, ErrorCode> {
        Ok(self.balances.get(env, account)?.unwrap_or(0))
    }
}
```

### Error Handling

Use `define_error!` macro for compile-time error registration:

```rust
define_error!(
    MyModuleError,
    INSUFFICIENT_BALANCE = 0x01,
    UNAUTHORIZED = 0x02,
);
```

Error ID ranges:
- `0x00-0x3F`: Validation errors
- `0x40-0x7F`: System errors
- `0x80-0xBF`: Business logic errors

### Storage Collections

| Type | Use Case |
|------|----------|
| `Item<T>` | Single value (config, owner) |
| `Map<K, V>` | Ordered key-value (deterministic iteration) |
| `UnorderedMap<K, V>` | Hash-based (no iteration guarantees) |
| `Vector<T>` | Dynamic array |
| `Queue<T>` | FIFO queue |

### Code Quality Standards

1. **Bounded complexity**: Functions < 70 lines
2. **Explicit types**: Sized types, minimize scope
3. **Fail fast**: Assertions for invariants, explicit error handling
4. **Minimal allocation**: Preallocate when possible, avoid hot-loop allocations
5. **Zero warnings**: `just quality` must pass

---

## Testing

### Unit Tests
Place alongside implementation in same file.

### Property Tests (proptest)
For invariant testing. Control cases with `EVOLVE_PROPTEST_CASES` env var.

### Simulation Tests
Use `evolve_simulator` for deterministic end-to-end testing:
```rust
let config = SimulatorConfig::default()
    .with_seed(12345)
    .with_blocks(100);
let result = Simulator::new(config).run();
```

### Debugging
Use `evolve_debugger` for trace recording and time-travel debugging.

---

## Available Skills

Claude agents have access to these project-specific skills:

| Skill | Trigger | Purpose |
|-------|---------|---------|
| `evolve-modules` | Writing account code | AccountCode trait, account_impl macro, storage patterns |
| `evolve-testing` | Writing tests | TestApp, SimTestApp, MockEnv, test infrastructure |
| `evolve-simulator` | Simulation testing | Deterministic simulation, fault injection, seed replay |
| `evolve-debugger` | Debugging | Trace recording, replay, time-travel debugging |
| `evolve-proptest` | Property testing | Invariant testing, fuzz testing, generators |
| `evolve-node-composer` | Creating nodes | Composing evd/testapp with storage, STF, RPC, gRPC |
| `stf-analysis` | STF review | Threading issues, non-determinism, correctness |

**Invoke skills** with `/<skill-name>` or let Claude auto-select based on context.

---

## Key Files Reference

| Purpose | Path |
|---------|------|
| Workspace config | `/Cargo.toml` |
| Build commands | `/justfile` |
| Clippy rules | `/.clippy.toml` |
| STF core | `/crates/app/stf/src/lib.rs` |
| STF traits | `/crates/app/sdk/stf_traits/src/lib.rs` |
| Core SDK | `/crates/app/sdk/core/src/lib.rs` |
| Account macros | `/crates/app/sdk/macros/src/lib.rs` |
| Error system | `/crates/app/sdk/core/src/error.rs` |
| Test harness | `/bin/testapp/tests/` |
| Simulator | `/crates/testing/simulator/src/lib.rs` |

---

## Pre-Commit Checklist

Before considering work complete:
1. `just fmt` - Format code
2. `just lint` - No clippy warnings
3. `just test` - All tests pass
4. `just quality` - Full quality check

---

## Learned Patterns

<!--
Claude agents: Add patterns learned from corrections here.
Format:
### Pattern Name
Brief description of the pattern and when to apply it.
-->


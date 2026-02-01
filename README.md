# Evolve

Evolve is a modular Rust SDK for building blockchain applications with Ethereum compatibility.

The goal of the project is to build a diverse ecosystem allowing applications to freely customize and innovate without needing to start from scratch. We opted to develop a set of simple primitives allowing anyone to build modules, integrate with any VM, and work with any consensus engine.

## Core Principles

- **Everything is an Account**: Storage, execution, and queries are namespaced by AccountId
- **Macro-Driven Development**: `#[account_impl]`, `#[init]`, `#[exec]`, `#[query]` eliminate boilerplate
- **Deterministic Execution**: Sequential block processing with checkpoint/rollback for atomicity
- **Ethereum Compatible**: Full EIP-2718 typed transactions, EIP-1559, and JSON-RPC support

## Features

### Account Development

Define accounts with zero boilerplate using procedural macros:

```rust
#[account_impl(MyAccount)]
pub mod account {
    #[init]
    fn initialize(env: &impl Env, initial_value: u128) -> SdkResult<()> {
        BALANCE.set(env, initial_value)?;
        Ok(())
    }

    #[exec]
    fn transfer(env: &impl Env, to: AccountId, amount: u128) -> SdkResult<()> {
        // transfer logic
        Ok(())
    }

    #[query]
    fn balance(env: &impl Env, account: AccountId) -> SdkResult<u128> {
        BALANCE.get(env, account)
    }
}
```

### State Collections

Type-safe, collision-free storage primitives:

- `Item<T>` - Single value storage
- `Map<K, V>` - Key-value mapping
- `Vector<T>` - Dynamic array
- `UnorderedMap<K, V>` - Hash-based mapping
- `Queue<T>` - FIFO queue

### Ethereum Compatibility

- **Typed Transactions**: EIP-2718 support (Legacy, EIP-1559)
- **JSON-RPC API**: Complete `eth_` namespace (`sendRawTransaction`, `call`, `estimateGas`, etc.)
- **WebSocket Subscriptions**: `eth_subscribe` with `newHeads` and logs filtering
- **Account/Address Mapping**: AccountId (u128) to Ethereum Address (20 bytes)
- **ECDSA Signatures**: Full signature verification support

### Transaction Mempool

- In-memory transaction pool with thread-safe Arc-based sharing
- Fair ordering with three-index design for O(log n) operations
- Hash-based duplicate detection for replay prevention
- EIP-1559 gas price ordering support

### Performance

- Lock-free primitives for hot-path metrics
- Multi-level caching: TxScratchpad -> DbCache -> RocksDB
- Live/archival storage split for optimized access patterns
- Predictive cache warming for known access patterns

## Modules

Pre-built modules available out of the box:

| Module | Description |
|--------|-------------|
| `token` | Fungible token with mint/burn/transfer and supply management |
| `scheduler` | Begin/end block hooks for scheduling block-level operations |

## Crate Structure

```
crates/
├── app/
│   ├── sdk/
│   │   ├── core/          # Environment traits, AccountCode, error handling
│   │   ├── macros/        # Procedural macros (#[account_impl], etc.)
│   │   ├── collections/   # Type-safe state collections
│   │   ├── stf_traits/    # Transaction, block, validator traits
│   │   ├── testing/       # MockEnv builder and test utilities
│   │   ├── math/          # Fixed-point arithmetic
│   │   ├── standards/     # Interface standards (fungible_asset, authentication)
│   │   └── x/             # Pre-built modules (token, scheduler)
│   ├── stf/               # State Transition Function engine
│   ├── tx/                # EIP-2718 transaction types, ECDSA, RLP encoding
│   ├── mempool/           # In-memory transaction pool
│   ├── server/            # Infrastructure: storage, RPC, indexing
│   ├── node/              # Dev node runner with block production
│   ├── genesis/           # Genesis file handling
│   └── operations/        # Config loading, shutdown, startup checks
├── storage/               # RocksDB-backed storage with caching
├── rpc/
│   ├── types/             # Ethereum-compatible RPC types
│   ├── eth-jsonrpc/       # JSON-RPC 2.0 server (eth_, net_, web3_)
│   ├── grpc/              # gRPC server
│   └── chain-index/       # Chain indexer
├── bin/
│   └── testapp/           # Example application
└── testing/
    ├── simulator/         # Deterministic simulator with fault injection
    ├── proptest/          # Property testing integration
    ├── debugger/          # Step-by-step execution tracing
    └── cli/               # CLI testing utilities
```

## Getting Started

### Prerequisites

- Rust 1.86.0 or later
- [just](https://github.com/casey/just) - command runner for development tasks

```bash
# Install just (macOS)
brew install just

# Install just (cargo)
cargo install just

# Install just (other platforms)
# See https://github.com/casey/just#installation
```

### Build

```bash
# Clone the repository
git clone https://github.com/01builders/evolve.git
cd evolve

# List all available commands
just --list

# Build the project
just build

# Run all tests
just test

# Run quality checks (format, lint, check)
just quality

# Run simulation tests
just test-sim

# Run CI checks locally
just ci
```

### Creating Your First Module

1. Add the SDK dependency to your `Cargo.toml`:

```toml
[dependencies]
evolve_core = { path = "crates/app/sdk/core" }
evolve_macros = { path = "crates/app/sdk/macros" }
evolve_collections = { path = "crates/app/sdk/collections" }
```

2. Define your account module:

```rust
use evolve_core::prelude::*;
use evolve_macros::account_impl;
use evolve_collections::Item;

// Define storage
static VALUE: Item<u128> = Item::new(b"value");

#[account_impl(Counter)]
pub mod counter {
    use super::*;

    #[init]
    fn initialize(env: &impl Env) -> SdkResult<()> {
        VALUE.set(env, 0)?;
        Ok(())
    }

    #[exec]
    fn increment(env: &impl Env) -> SdkResult<()> {
        let current = VALUE.get(env)?.unwrap_or(0);
        VALUE.set(env, current + 1)?;
        Ok(())
    }

    #[query]
    fn get_value(env: &impl Env) -> SdkResult<u128> {
        Ok(VALUE.get(env)?.unwrap_or(0))
    }
}
```

3. Register your account in the STF and create a genesis configuration.

### Running a Dev Node

```bash
# Initialize the dev node (creates data directory and genesis)
just node-init

# Run the dev node
just node-run

# Or reset and start fresh
just node-reset
```

Connect your wallet to `http://localhost:8545` (default RPC endpoint).

### Running Simulations

```bash
# Run a simulation (100 blocks by default)
just sim

# Run with a specific seed for reproducibility
just sim-seed 12345

# Run with tracing enabled
just sim-trace

# Replay and debug a trace
just sim-debug trace.json
```

### Testing

Use the provided `MockEnv` for unit testing:

```rust
use evolve_testing::MockEnv;

#[test]
fn test_counter() {
    let env = MockEnv::builder()
        .with_caller(AccountId::new(1))
        .build();

    counter::initialize(&env).unwrap();
    counter::increment(&env).unwrap();

    assert_eq!(counter::get_value(&env).unwrap(), 1);
}
```

## Configuration

Example `config.yaml`:

```yaml
chain:
  chain_id: 1
  gas:
    storage_get_charge: 10
    storage_set_charge: 10
    storage_remove_charge: 10

storage:
  path: "./data"
  cache_size: 1GB
  write_buffer_size: 64MB

rpc:
  enabled: true
  http_addr: "127.0.0.1:8545"
  client_version: "evolve/0.1.0"

grpc:
  enabled: false
  addr: "127.0.0.1:9545"
  max_message_size: 4MB

operations:
  shutdown_timeout_secs: 30
  startup_checks: true
  min_disk_space_mb: 1024

observability:
  log_level: "info"
  log_format: "json"
  metrics_enabled: true
```

## Documentation

- [SDK Guide](crates/app/sdk/README.md) - Complete SDK usage with examples
- [STF Architecture](docs/STF_ARCHITECTURE.md) - Block execution flow and components
- [Concurrency Model](docs/concurrency-model.md) - Lock-free designs and patterns
- [Module System](docs/module-system/) - Determinism guidelines and testing patterns

## Claude Code Skills

If you're using [Claude Code](https://claude.ai/claude-code), the following project-specific skills are available:

| Skill | Triggers | Description |
|-------|----------|-------------|
| **Module Development** | "write module", "create module", "account_impl" | Guide for writing Evolve modules with storage, events, and testing |
| **Testing Guide** | "how to test", "TestApp", "SimTestApp" | Overview of testing infrastructure (MockEnv, TestApp, SimTestApp) |
| **Simulator** | "use simulator", "deterministic test", "fault injection" | Deterministic simulation with time control and fault injection |
| **Property Testing** | "property test", "proptest", "invariant testing" | Property-based testing with generators, invariants, and shrinking |
| **Debugger** | "debug execution", "trace recording", "time travel debug" | Execution tracing and time-travel debugging for failing tests |
| **STF Analysis** | "analyze STF", "check threading model", "find non-determinism" | Analyze State Transition Function for correctness issues |

### Example Queries

- **"How do I write a module?"** → Triggers `evolve-modules` skill with full guide
- **"How do I test my module?"** → Triggers `evolve-testing` skill with TestApp/SimTestApp examples
- **"Debug this failing test"** → Triggers `evolve-debugger` skill with trace recording instructions
- **"Add property tests"** → Triggers `evolve-proptest` skill with generator and invariant examples
- **"Analyze the STF for issues"** → Triggers `stf-analysis` skill with threading/determinism checklist

### Onboarding with Claude Code

When starting with the codebase, you can ask Claude Code to:

1. **Explore the architecture**: "What is the codebase structure?" or "How does the STF work?"
2. **Understand a module**: "Explain how the token module works"
3. **Find code patterns**: "Where is transaction validation handled?"
4. **Create new modules**: "How do I write a module?" for step-by-step guidance
5. **Set up testing**: "How do I test my module?" for testing patterns
6. **Debug issues**: "Debug this failing test" with trace recording

Claude Code will use the Explore agent to navigate the codebase and provide context-aware answers.

## License

Apache-2.0

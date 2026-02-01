---
name: evolve-testing
description: Overview of Evolve SDK testing infrastructure including TestApp, SimTestApp, MockEnv, and transaction generators. Use when writing tests, setting up test harnesses, using test infrastructure, or choosing between testing approaches.
---

# Evolve Testing Infrastructure

The Evolve SDK provides a layered testing infrastructure for unit tests, integration tests, stress testing, and property-based testing.

## Test Harnesses

### TestApp - Simple Unit Testing

Use for fast, isolated tests without full simulation:

```rust
use testapp::TestApp;

#[test]
fn test_basic_flow() {
    let mut app = TestApp::new();
    let accounts = app.accounts();

    // Execute as a specific account
    app.system_exec_as(accounts.alice, |env| {
        let token = TokenRef::from(accounts.atom);
        token.transfer(accounts.bob, 100, env)
    }).unwrap();

    // Advance blocks
    app.next_block();
    app.go_to_height(10);

    // Mint test tokens
    let asset = app.mint_atom(accounts.alice, 1000);
}
```

**When to use:** Unit tests, quick functional verification, no need for deterministic replay.

### SimTestApp - Full Simulation

Use for integration tests with deterministic execution, tracing, and fault injection:

```rust
use testapp::SimTestApp;
use evolve_simulator::SimConfig;

#[test]
fn test_with_simulation() {
    // Default config with seed 0
    let mut app = SimTestApp::new();

    // Or with custom config
    let mut app = SimTestApp::with_config(SimConfig::default(), 42);

    let accounts = app.accounts();

    // Apply blocks directly
    let block = TestBlock::new(1, vec![tx1, tx2]);
    let result = app.apply_block(&block);

    // Or generate blocks with closures
    let results = app.run_blocks_with(10, |height, sim| {
        vec![generate_tx(height, sim)]
    });

    // Access simulator for time/RNG
    app.simulator().time().block_height();
    app.simulator_mut().rng().gen_range(0..100);
}
```

**When to use:** Integration tests, multi-block scenarios, deterministic reproduction, trace capture.

## Transaction Generators

For simulation-based testing, use the registry pattern:

```rust
use testapp::{TxGeneratorRegistry, TokenTransferGenerator};

#[test]
fn test_with_generators() {
    let mut app = SimTestApp::new();
    let accounts = app.accounts();

    let mut registry = TxGeneratorRegistry::new();

    // Register weighted generators
    registry.register(10, TokenTransferGenerator::new(
        accounts.atom,
        vec![accounts.alice, accounts.bob],  // senders
        vec![accounts.alice, accounts.bob],  // recipients
        1,      // min amount
        100,    // max amount
        100000, // gas limit
    ));

    // Run 100 blocks with generated transactions
    let results = app.run_blocks_with_registry(100, &mut registry);

    // Check results
    for result in &results {
        assert!(result.tx_results.iter().all(|r| r.response.is_ok()));
    }
}
```

### Custom Generator

```rust
use testapp::TxGenerator;
use evolve_simulator::Simulator;

struct MyGenerator {
    // state
}

impl TxGenerator for MyGenerator {
    fn generate_tx(&mut self, height: u64, sim: &mut Simulator) -> Option<TestTx> {
        let amount = sim.rng().gen_range(1..=100);
        Some(TestTx {
            sender: self.sender,
            recipient: self.target,
            request: InvokeRequest::new(&MyMsg { amount }).ok()?,
            gas_limit: 100000,
            funds: vec![],
        })
    }
}
```

## Trace Recording

Capture execution traces for debugging:

```rust
use evolve_debugger::{TraceBuilder, StateSnapshot};

#[test]
fn test_with_trace() {
    let mut app = SimTestApp::new();

    // Run with automatic trace capture
    let (results, trace) = app.run_blocks_with_trace(10, |height, sim| {
        vec![generate_tx(height, sim)]
    });

    // Or manual trace building
    let snapshot = StateSnapshot::from_data(
        app.simulator().storage().snapshot().data,
        app.simulator().time().block_height(),
        app.simulator().time().now_ms(),
    );
    let mut builder = TraceBuilder::new(app.simulator().seed_info().seed, snapshot);

    let result = app.apply_block_with_trace(&block, &mut builder);
    let trace = builder.finish();

    // Save trace for later analysis
    trace.save("test_trace.bin").unwrap();
}
```

## Mock Environment

For testing modules in isolation without the STF:

```rust
use evolve_testing::MockEnv;
use evolve_core::AccountId;

#[test]
fn test_module_directly() {
    let contract = AccountId::new(1);
    let sender = AccountId::new(2);
    let mut env = MockEnv::new(contract, sender);

    let module = MyModule::default();
    module.initialize(sender, &mut env).unwrap();

    // Change sender for subsequent calls
    env = env.with_sender(AccountId::new(3));

    // Test unauthorized access
    let result = module.admin_only(&mut env);
    assert!(result.is_err());
}
```

### MockEnv with Failure Injection

```rust
use evolve_collections::mocks::MockEnvironment;

let mut env = MockEnvironment::default();
env = env.with_failure(); // Next storage operation will fail

let result = module.some_operation(&mut env);
assert!(result.is_err());
```

## Test Patterns

### Setup Helper

```rust
fn setup_test_environment() -> (StorageMock, AccountStorageMock, CustomStf, GenesisAccounts) {
    let bootstrap_stf = build_stf(SystemAccounts::placeholder(), PLACEHOLDER_ACCOUNT);
    let mut codes = AccountStorageMock::default();
    install_account_codes(&mut codes);

    let mut state = StorageMock::default();
    let (genesis_state, accounts) = do_genesis(&bootstrap_stf, &codes, &state).unwrap();
    state.apply_changes(genesis_state.into_changes().unwrap()).unwrap();

    let stf = build_stf(SystemAccounts::new(accounts.gas_service), accounts.scheduler);
    (state, codes, stf, accounts)
}
```

### Block Execution Test

```rust
#[test]
fn test_block_execution() {
    let (mut storage, codes, stf, accounts) = setup_test_environment();

    let tx = TestTx {
        sender: accounts.alice,
        recipient: accounts.atom,
        request: InvokeRequest::new(&TransferMsg { to: accounts.bob, amount: 50 }).unwrap(),
        gas_limit: 100000,
        funds: vec![],
    };

    let block = TestBlock::new(1, vec![tx]);
    let (result, new_state) = stf.apply_block(&storage, &codes, &block);

    assert!(result.tx_results[0].response.is_ok());
    storage.apply_changes(new_state.into_changes().unwrap()).unwrap();
}
```

### State Verification

```rust
#[test]
fn test_state_changes() {
    let mut app = TestApp::new();
    let accounts = app.accounts();

    // Get balance before
    let before = app.system_exec_as(accounts.alice, |env| {
        TokenRef::from(accounts.atom).get_balance(accounts.alice, env)
    }).unwrap();

    // Do transfer
    app.system_exec_as(accounts.alice, |env| {
        TokenRef::from(accounts.atom).transfer(accounts.bob, 100, env)
    }).unwrap();

    // Verify balance after
    let after = app.system_exec_as(accounts.alice, |env| {
        TokenRef::from(accounts.atom).get_balance(accounts.alice, env)
    }).unwrap();

    assert_eq!(before.unwrap() - after.unwrap(), 100);
}
```

## Benchmarking

Use Criterion with `iter_batched` for proper setup/teardown:

```rust
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

fn bench_apply_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_execution");

    for tx_count in [10, 100, 500] {
        group.bench_function(format!("apply_block_{tx_count}_txs"), |b| {
            b.iter_batched(
                || {
                    // Setup: create app and block
                    let mut app = SimTestApp::new();
                    let block = make_block(tx_count, &mut app);
                    (app, block)
                },
                |(mut app, block)| {
                    // Measured: apply block
                    app.apply_block(&block)
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_apply_block);
criterion_main!(benches);
```

## When to Use What

| Scenario | Tool |
|----------|------|
| Unit test a single module | `MockEnv` |
| Test module interactions | `TestApp` |
| Multi-block scenarios | `SimTestApp` |
| Deterministic reproduction | `SimTestApp` with fixed seed |
| Stress testing with faults | `SimTestApp` with `SimConfig::stress_test()` |
| Debug failing test | `apply_block_with_trace` + debugger |
| Property-based testing | `evolve_proptest` + `SimTestApp` |
| Performance benchmarks | Criterion + `SimTestApp` |

## Files

- `crates/app/testapp/src/testing.rs` - TestApp harness
- `crates/app/testapp/src/sim_testing.rs` - SimTestApp harness
- `crates/app/sdk/testing/src/server_mocks.rs` - Storage/code mocks
- `crates/app/sdk/collections/src/mocks.rs` - MockEnvironment
- `crates/testing/simulator/` - Deterministic simulator
- `crates/testing/debugger/` - Trace recording/replay
- `crates/testing/proptest/` - Property-based testing
- `crates/app/testapp/benches/` - Benchmark examples

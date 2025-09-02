---
description: How to use the Evolve deterministic simulator for testing
triggers:
  - "use simulator"
  - "create simulation"
  - "deterministic test"
  - "fault injection"
  - "seed-based testing"
---

# Evolve Simulator

The `evolve_simulator` crate provides a deterministic simulation engine for testing Evolve SDK applications with controlled time, storage, and randomness.

## Key Features

1. **Seed-based reproducibility**: Every simulation can be exactly reproduced by providing the same seed
2. **Time acceleration**: Simulate hours of blockchain operation in seconds
3. **Fault injection**: Test error handling with configurable storage failures
4. **Performance tracking**: Collect detailed metrics about block/tx execution

## Basic Usage

```rust
use evolve_simulator::{Simulator, SimConfig, SimulatorBuilder};

// Create a simulator with default config
let mut sim = Simulator::new(12345, SimConfig::default());

// Run simulation for 100 blocks
sim.run_blocks(100).unwrap();

// Get performance report
let report = sim.generate_report();
println!("{}", report.to_string_pretty());

// Print reproduction command
println!("Reproduce with seed: {}", sim.seed());
```

## Configuration

```rust
use evolve_simulator::{SimConfig, StorageConfig, TimeConfig, MetricsConfig};

let config = SimConfig {
    time: TimeConfig {
        ticks_per_block: 10,
        tick_ms: 100,           // 100ms per tick
        initial_timestamp_ms: 0,
    },
    storage: StorageConfig {
        read_fault_prob: 0.01,  // 1% read failures
        write_fault_prob: 0.01, // 1% write failures
        log_operations: true,   // Log all operations
    },
    metrics: MetricsConfig::default(),
    max_blocks: 1000,
    max_txs_per_block: 100,
    stop_on_error: false,
};
```

## Using the Builder

```rust
let (sim, seed) = SimulatorBuilder::new()
    .seed(42)                          // Optional: use specific seed
    .max_blocks(500)
    .stop_on_error()
    .storage_config(StorageConfig::with_faults(0.01, 0.01))
    .with_state(b"key".to_vec(), b"value".to_vec())  // Initial state
    .build_with_seed();

println!("Running with seed: {seed}");
```

## Time Control

```rust
// Advance by ticks
sim.tick();
sim.advance_ticks(100);

// Advance by blocks
sim.advance_block();
sim.set_block_height(50);

// Query time
let height = sim.time().block_height();
let timestamp = sim.time().now_ms();
```

## Storage Operations

```rust
use evolve_server_core::StateChange;

// Apply state changes
let changes = vec![
    StateChange::Set { key: b"key1".to_vec(), value: b"value1".to_vec() },
    StateChange::Delete { key: b"key2".to_vec() },
];
sim.apply_state_changes(changes).unwrap();

// Read storage
let value = sim.storage().get(b"key1").unwrap();
```

## Snapshots and Restore

```rust
// Create checkpoint
let snapshot = sim.snapshot();

// Do some work...
sim.run_blocks(10).unwrap();

// Restore to checkpoint
sim.restore(snapshot);
```

## Metrics and Reporting

```rust
let report = sim.generate_report();

println!("Total blocks: {}", report.total_blocks);
println!("Total txs: {}", report.total_txs);
println!("Success rate: {:.2}%", report.success_rate * 100.0);
println!("Avg gas/tx: {:.2}", report.avg_gas_per_tx);
println!("P99 block time: {:.2}ms", report.p99_block_time_ms);
println!("Time acceleration: {:.1}x", report.time_acceleration);
```

## Stress Testing

```rust
// Use pre-configured stress test settings
let config = SimConfig::stress_test();
let mut sim = Simulator::new(seed, config);

sim.run_until(|s| s.metrics().total_errors > 0).unwrap();

if let Some(reason) = sim.abort_reason() {
    println!("Simulation aborted: {reason}");
}
```

## Reproduction Pattern

When a test fails, capture the seed for exact reproduction:

```rust
#[test]
fn property_test() {
    let (mut sim, seed) = Simulator::with_random_seed(SimConfig::default());

    // ... run test ...

    if test_failed {
        eprintln!("FAILURE! Reproduce with seed: {seed}");
        panic!("Test failed");
    }
}
```

## Files

- `crates/testing/simulator/src/lib.rs` - Main Simulator struct
- `crates/testing/simulator/src/seed.rs` - Deterministic RNG
- `crates/testing/simulator/src/time.rs` - Simulated time
- `crates/testing/simulator/src/storage.rs` - Storage with fault injection
- `crates/testing/simulator/src/metrics.rs` - Performance tracking

---
name: evolve-debugger
description: Use the Evolve execution debugger for trace recording, replay, and time-travel debugging. Use when debugging failing tests, recording execution traces, stepping through state changes, or analyzing execution flow.
---


# Evolve Execution Debugger

The `evolve_debugger` crate provides execution tracing, time-travel debugging, and state inspection for debugging failing tests and understanding execution flow.

## Recording Traces

### With SimTestApp

```rust
use testapp::SimTestApp;

let mut app = SimTestApp::new();

// Automatic trace capture
let (results, trace) = app.run_blocks_with_trace(10, |height, sim| {
    vec![generate_tx(height, sim)]
});

// Save for later analysis
trace.save("debug_trace.bin").unwrap();
```

### Manual Trace Building

```rust
use evolve_debugger::{TraceBuilder, StateSnapshot};

let snapshot = StateSnapshot::from_data(
    app.simulator().storage().snapshot().data,
    app.simulator().time().block_height(),
    app.simulator().time().now_ms(),
);

let mut builder = TraceBuilder::new(seed, snapshot);

// Record block execution
builder.block_start(height, timestamp_ms);

for tx in block.txs() {
    builder.tx_start(tx.compute_identifier(), tx.sender, tx.recipient);
}

// Execute and capture state changes
let (result, state) = stf.apply_block(&storage, &codes, &block);

for change in state.into_changes()? {
    match change {
        StateChange::Set { key, value } => {
            let old = storage.get(&key)?;
            builder.state_change(key, old, Some(value));
        }
        StateChange::Remove { key } => {
            let old = storage.get(&key)?;
            builder.state_change(key, old, None);
        }
    }
}

for (tx, tx_result) in block.txs().iter().zip(result.tx_results.iter()) {
    builder.tx_end(tx.compute_identifier(), tx_result.response.is_ok(), tx_result.gas_used);
}

builder.block_end(height, storage.state_hash());

let trace = builder.finish();
```

## Loading and Replaying Traces

```rust
use evolve_debugger::ExecutionTrace;

// Load from file
let trace = ExecutionTrace::load("debug_trace.bin")?;

// Inspect metadata
println!("Seed: {}", trace.seed());
println!("Blocks: {}", trace.block_count());
println!("Total transactions: {}", trace.tx_count());

// Iterate through blocks
for block in trace.blocks() {
    println!("Block {}: {} txs", block.height, block.tx_count());

    for tx in block.transactions() {
        println!("  TX {}: {} -> {}", tx.id, tx.sender, tx.recipient);
        if let Some(error) = &tx.error {
            println!("    ERROR: {}", error);
        }
    }
}
```

## State Inspection

```rust
// Get state at a specific point
let state_at_block_5 = trace.state_at_block(5)?;

// Compare states
let diff = trace.state_diff(block_start, block_end)?;
for (key, (old, new)) in diff {
    println!("Key {:?}: {:?} -> {:?}", key, old, new);
}

// Query specific key through execution
let key_history = trace.key_history(b"my_key")?;
for (block, value) in key_history {
    println!("Block {}: {:?}", block, value);
}
```

## Breakpoints and Stepping

```rust
use evolve_debugger::{Debugger, Breakpoint};

let mut debugger = Debugger::new(trace);

// Set breakpoints
debugger.add_breakpoint(Breakpoint::OnError);
debugger.add_breakpoint(Breakpoint::AtBlock(5));
debugger.add_breakpoint(Breakpoint::AtTx(tx_id));
debugger.add_breakpoint(Breakpoint::OnStateChange(b"key".to_vec()));
debugger.add_breakpoint(Breakpoint::Custom(|ctx| ctx.gas_used > 100000));

// Step through execution
while debugger.step() {
    let ctx = debugger.context();
    println!("Block: {}, TX: {:?}", ctx.block_height, ctx.current_tx);
    println!("State changes: {}", ctx.pending_changes.len());

    if debugger.hit_breakpoint() {
        println!("Hit breakpoint: {:?}", debugger.last_breakpoint());
        // Inspect state
        let value = debugger.get_state(b"key")?;
    }
}

// Step backwards (time travel)
debugger.step_back();
```

## Trace Formats

```rust
use evolve_debugger::TraceFormat;

// Binary (default, smallest)
trace.save_with_format("trace.bin", TraceFormat::Binary)?;

// Compressed binary
trace.save_with_format("trace.bin.gz", TraceFormat::BinaryCompressed)?;

// JSON (human-readable)
trace.save_with_format("trace.json", TraceFormat::Json)?;

// Load auto-detects format
let trace = ExecutionTrace::load("trace.bin")?;
```

## Reproduction Commands

When a test fails, generate a reproduction command:

```rust
#[test]
fn property_test() {
    let (mut app, seed_info) = SimTestApp::with_random_seed(SimConfig::default());

    // ... test fails ...

    eprintln!("FAILURE! Reproduce with:");
    eprintln!("{}", seed_info.reproduction_command());
    // Outputs: cargo test property_test -- --seed 12345
}
```

## Integration with Property Testing

```rust
use evolve_proptest::PropertyTestRunner;

let runner = PropertyTestRunner::new()
    .with_trace_on_failure(true);

let result = runner.run(|scenario| {
    let mut app = SimTestApp::new();
    // ... execute scenario ...
});

if let Err(failure) = result {
    // Trace is automatically captured on failure
    let trace = failure.trace.unwrap();
    trace.save(format!("failure_{}.bin", failure.seed))?;

    println!("Minimal failing case saved");
    println!("Seed: {}", failure.seed);
    println!("Blocks: {}", failure.minimal.blocks.len());
}
```

## Debugging Workflow

1. **Capture trace on failure:**
   ```rust
   let (results, trace) = app.run_blocks_with_trace(100, generator);
   if results.iter().any(|r| r.has_errors()) {
       trace.save("failure.bin")?;
   }
   ```

2. **Load and analyze:**
   ```rust
   let trace = ExecutionTrace::load("failure.bin")?;
   let mut debugger = Debugger::new(trace);
   debugger.add_breakpoint(Breakpoint::OnError);
   debugger.run_until_breakpoint();
   ```

3. **Inspect state at failure:**
   ```rust
   let ctx = debugger.context();
   println!("Failed at block {} tx {}", ctx.block_height, ctx.tx_index);
   println!("Error: {:?}", ctx.error);

   // Check relevant state
   let balance = debugger.get_state(&balance_key(account))?;
   println!("Balance at failure: {:?}", balance);
   ```

4. **Step back to find root cause:**
   ```rust
   debugger.step_back();
   while debugger.context().error.is_none() {
       debugger.step_back();
   }
   // Now at the first state where things went wrong
   ```

## Files

- `crates/testing/debugger/src/lib.rs` - Main exports
- `crates/testing/debugger/src/trace.rs` - ExecutionTrace
- `crates/testing/debugger/src/builder.rs` - TraceBuilder
- `crates/testing/debugger/src/debugger.rs` - Debugger with breakpoints
- `crates/testing/debugger/src/formats.rs` - Serialization formats

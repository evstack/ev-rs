# Determinism Requirements

All module code must be deterministic. Given identical inputs, execution must produce identical outputs on all nodes. Violating determinism causes consensus failures.

## Banned Patterns

### 1. Non-Deterministic Collections

**NEVER use `HashMap` or `HashSet`** - iteration order varies by platform/run.

```rust
// BAD - iteration order is non-deterministic
use std::collections::HashMap;
let map = HashMap::new();
for (k, v) in &map { /* order varies! */ }

// GOOD - use BTreeMap for deterministic ordering
use std::collections::BTreeMap;
let map = BTreeMap::new();
for (k, v) in &map { /* order is consistent */ }

// BEST - use Evolve's storage collections
use evolve_collections::map::Map;
let map: Map<Key, Value> = Map::new(0);
```

### 2. System Time

**NEVER use `std::time`** - system clocks vary between nodes.

```rust
// BAD - non-deterministic
use std::time::{Instant, SystemTime};
let now = SystemTime::now();

// GOOD - use block time from BlockInfo module
let block_time = block_info.get_time_unix_ms(env)?;
```

### 3. Random Number Generation

**NEVER use `rand` or any RNG** without deterministic seeding from chain state.

```rust
// BAD - non-deterministic
use rand::Rng;
let random = rand::thread_rng().gen::<u64>();

// GOOD - derive randomness from block hash or other chain state
let seed = block_hash.as_bytes();
```

### 4. Floating Point Arithmetic

**NEVER use `f32` or `f64`** - floating point operations can vary across platforms.

```rust
// BAD - platform-dependent results
let result = 0.1 + 0.2; // might not equal 0.3 exactly

// GOOD - use fixed-point arithmetic
use evolve_math::FixedPoint;
let result = FixedPoint::from_decimal(1, 1) + FixedPoint::from_decimal(2, 1);
```

### 5. External I/O

**NEVER perform I/O operations** - file access, network calls, or environment variables.

```rust
// BAD - all of these
std::fs::read("file.txt");
std::net::TcpStream::connect("...");
std::env::var("SECRET");
```

### 6. Threading and Async

**NEVER spawn threads or use async** - execution order is non-deterministic.

```rust
// BAD
std::thread::spawn(|| { /* ... */ });

// Modules execute synchronously within the STF
```

## Compile-Time Protection

The workspace is configured to warn/deny common non-deterministic patterns. Add this to `.clippy.toml`:

```toml
disallowed-types = [
    { path = "std::collections::HashMap", reason = "Non-deterministic iteration order" },
    { path = "std::collections::HashSet", reason = "Non-deterministic iteration order" },
    { path = "std::time::Instant", reason = "Non-deterministic - use BlockInfo" },
    { path = "std::time::SystemTime", reason = "Non-deterministic - use BlockInfo" },
]
```

## Safe Patterns

### Using Evolve Collections

All Evolve collections are deterministic by design:

- `Item<T>` - Single value storage
- `Map<K, V>` - Key-value with deterministic iteration (by insertion order)
- `Vector<T>` - Ordered sequence
- `Queue<T>` - FIFO queue
- `UnorderedMap<K, V>` - Unordered but deterministic within a session

### Deriving Values from Chain State

When you need "randomness" or unique values:

```rust
// Use the Unique module for unique IDs
let unique_id = unique_ref.next_unique_id(env)?;

// Use block info for time-based logic
let block_height = block_info.get_height(env)?;
let block_time = block_info.get_time_unix_ms(env)?;
```

## Testing for Determinism

Run the same test multiple times and verify identical results:

```rust
#[test]
fn test_determinism() {
    for _ in 0..100 {
        let result = execute_module_logic();
        assert_eq!(result, EXPECTED_VALUE);
    }
}
```

## Debugging Consensus Failures

If nodes diverge:

1. Check for `HashMap`/`HashSet` usage
2. Look for floating point arithmetic
3. Verify no time-dependent logic
4. Check for uninitialized memory access
5. Run with `RUST_BACKTRACE=1` to find the divergence point

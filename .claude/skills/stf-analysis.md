---
description: Analyze State Transition Function implementations for correctness, threading issues, and simplification opportunities
triggers:
  - analyze STF
  - analyze state transition
  - check threading model
  - find non-determinism
  - simplify execution
---

# STF Analysis Skill

Use this skill to analyze State Transition Function implementations for:
- Threading correctness (Send/Sync bounds)
- Non-determinism sources
- Simplification opportunities
- Test coverage gaps

## Analysis Checklist

### 1. Threading Model Analysis

Check for unnecessary Send/Sync bounds:

```bash
# Find all Send/Sync trait bounds
grep -rn "Send + Sync\|: Send\|: Sync" crates/

# Check if bounds are actually needed by looking for:
# - spawn, thread usage
# - Arc, Mutex, RwLock patterns
# - async/await with cross-task sharing
```

Questions to answer:
- [ ] Is block execution single-threaded? (Should be for determinism)
- [ ] Where does actual concurrency exist?
- [ ] Are there bounds that propagate unnecessarily?

### 2. Non-Determinism Sources

Check for common non-determinism issues:

```bash
# HashMap iteration (non-deterministic order)
grep -rn "\.iter()\|\.into_iter()" --include="*.rs" crates/app/stf/

# Look for into_changes() and verify output is sorted or order-independent
```

Common sources:
- [ ] HashMap/HashSet iteration without sorting
- [ ] Floating point arithmetic
- [ ] System time usage
- [ ] Random number generation
- [ ] Parallel execution without deterministic ordering

### 3. Atomicity Verification

Verify checkpoint/restore pattern:

```rust
// Every state-changing operation should follow this pattern:
let checkpoint = storage.checkpoint();
let result = do_operation();
if result.is_err() {
    storage.restore(checkpoint);
}
```

Check:
- [ ] All do_invoke() calls have checkpoint before
- [ ] All error paths restore checkpoint
- [ ] Events are truncated on restore
- [ ] Unique object counter is restored

### 4. Resource Limits

Verify security limits exist:

| Resource | Expected Limit | Location |
|----------|---------------|----------|
| Overlay entries | ~100,000 | execution_state.rs |
| Events per tx | ~10,000 | execution_state.rs |
| Key size | ~256 bytes | execution_state.rs |
| Value size | ~1 MB | execution_state.rs |
| Call depth | ~64 | invoker.rs |

### 5. Test Coverage Gaps

Property tests that should exist:

```rust
// ExecutionState model test
#[test]
fn prop_execution_state_model() { /* overlay + undo = correct state */ }

// Gas counter model test  
#[test]
fn prop_gas_counter_model() { /* no overflow, correct tracking */ }

// Block failure invariance
#[test]
fn prop_apply_block_failure_invariance() { /* checkpoint/restore works */ }

// Determinism test (NEW - often missing)
#[test]
fn prop_determinism() { /* same input = same output, always */ }
```

### 6. Simplification Opportunities

Look for:

1. **Unused trait bounds**
   ```rust
   // If ReadonlyKV: Send + Sync but only warm_cache needs Sync
   // Move the bound to warm_cache, not the trait
   ```

2. **Atomic overhead without threading**
   ```rust
   // If metrics use AtomicU64 but execution is single-threaded
   // Consider Cell<u64> if Sync not required
   ```

3. **Dead code paths**
   ```rust
   // Fields that exist but are never called
   // e.g., post_tx_handler that's never invoked
   ```

## Output Template

After analysis, produce a report:

```markdown
## STF Analysis Report

### Threading Model
- Execution model: [single-threaded / parallel]
- Concurrency locations: [list]
- Unnecessary bounds: [list]

### Non-Determinism Risks
- [x] HashMap iteration in into_changes() - NEEDS SORTING
- [ ] No floating point usage - OK
- [ ] No system time in execution - OK

### Atomicity
- [x] Checkpoints before state changes - OK
- [x] Restore on all error paths - OK
- [ ] Missing: [any gaps]

### Resource Limits
- [x] Overlay: 100,000 entries
- [x] Call depth: 64 levels
- [ ] Missing: [any gaps]

### Test Gaps
- [ ] Determinism test missing
- [ ] Call depth exhaustion test missing

### Simplification Recommendations
1. Remove Send+Sync from ReadonlyKV, add to warm_cache only
2. Replace AtomicU64 with Cell<u64> in metrics (if Sync not needed)
3. [other recommendations]
```

## Quick Commands

```bash
# Check current Send/Sync usage
rg "Send \+ Sync|: Send|: Sync" crates/app/stf/src/

# Find HashMap iterations that might be non-deterministic
rg "\.into_iter\(\)|\.iter\(\)" crates/app/stf/src/ -A 2

# Check for checkpoint usage
rg "checkpoint\(\)|restore\(" crates/app/stf/src/

# Find all error returns without restore
rg "return Err" crates/app/stf/src/invoker.rs -B 5

# Check test coverage
cargo test -p evolve_stf -- --list 2>&1 | grep "test "
```

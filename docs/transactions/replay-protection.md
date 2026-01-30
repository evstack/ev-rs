# Replay Protection

Replay attacks occur when a valid transaction is re-submitted to drain funds or duplicate actions. Evolve provides multiple layers of replay protection depending on the transaction type.

## Overview

| Protection Layer | ETH TX | Micro TX |
|-----------------|--------|----------|
| Chain ID | ✓ | ✓ |
| Sequential Nonce | ✓ | ✗ |
| Timestamp Ordering | ✗ | ✓ |

---

## Chain ID Protection

Both transaction types include a chain ID to prevent cross-chain replay attacks.

### How It Works

The chain ID is included in the signed data, so a transaction signed for chain 1 cannot be replayed on chain 2:

```
signing_hash = keccak256(tx_type || chain_id || ... || other_fields)
```

### Verification

```rust
// Ethereum transactions
let verifier = EcdsaVerifier::new(expected_chain_id);
verifier.verify(&tx)?;  // Fails if chain_id doesn't match

// Micro transactions
if tx.chain_id() != expected_chain_id {
    return Err(ERR_INVALID_CHAIN_ID);
}
```

---

## Nonce-Based Protection (Ethereum Transactions)

Traditional Ethereum transactions use sequential nonces.

### How It Works

```
Account State:
  nonce: 5

Valid next transaction must have nonce = 5
After execution, account nonce becomes 6
```

### Pros and Cons

**Pros:**
- Guaranteed ordering
- Simple mental model
- Battle-tested in production

**Cons:**
- **No parallel submission** - Must wait for tx N to confirm before sending tx N+1
- **Nonce management complexity** - Clients must track pending txs
- **Head-of-line blocking** - One stuck tx blocks all subsequent txs

### Implementation

Nonce validation typically happens in the transaction validator:

```rust
impl TxValidator<TxEnvelope> for NonceValidator {
    fn validate_tx(&self, tx: &TxEnvelope, env: &mut dyn Environment) -> SdkResult<()> {
        let account_nonce = get_account_nonce(tx.sender(), env)?;

        if tx.nonce() < account_nonce {
            return Err(ERR_NONCE_TOO_LOW);
        }
        if tx.nonce() > account_nonce {
            return Err(ERR_NONCE_TOO_HIGH);
        }

        Ok(())
    }
}
```

---

## Timestamp-Based Protection (Micro Transactions)

Micro transactions replace nonces with timestamps, enabling parallel submission.

### How It Works

```
Account State:
  last_seen_timestamp: 1706500000000 (ms)

Valid transaction must have timestamp > 1706500000000
After execution, last_seen_timestamp = transaction.timestamp
```

### Throughput

With millisecond timestamps and strict ordering (`>`), max throughput is ~1000 tx/sec per account.

> **TODO: Higher Throughput Option**
>
> If ~1000 tx/sec per account is insufficient, add transaction hash tracking with a
> rolling window and change the check to `timestamp >= last_seen`. This allows multiple
> transactions in the same millisecond, differentiated by their hash. The hash window
> only needs to cover recent timestamps (e.g., last few seconds) since older timestamps
> would fail the `>=` check anyway.

### The Key Insight

Timestamps are **monotonically increasing** like nonces, but:
- No coordination needed between parallel senders
- Clock skew is bounded and manageable

### Implementation

The `NoncelessAccount` module handles timestamp-based replay protection:

```rust
#[derive(evolve_core::AccountState)]
pub struct NoncelessAccount {
    #[storage(0)]
    pub last_seen_timestamp: Item<u64>,

    #[storage(1)]
    pub owner: Item<AccountId>,
}

#[exec]
pub fn verify_and_update(
    &self,
    timestamp: u64,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    // 1. Verify sender is owner
    if env.sender() != self.owner.get(env)? {
        return Err(ERR_UNAUTHORIZED);
    }

    // 2. Verify timestamp is strictly greater than last seen
    let last_ts = self.last_seen_timestamp.may_get(env)?.unwrap_or(0);
    if timestamp <= last_ts {
        return Err(ERR_TIMESTAMP_REPLAY);
    }

    // 3. Update last seen timestamp
    self.last_seen_timestamp.set(&timestamp, env)?;

    Ok(())
}
```

Note: The `NoncelessAccount` only handles replay protection state. Actual value
transfers are handled separately at the execution layer (similar to ETH nonce accounts).

### Parallel Submission

With timestamps, the same account can submit multiple transactions simultaneously:

```
Time 0ms:  Submit tx with timestamp=1000
Time 1ms:  Submit tx with timestamp=1001  (doesn't wait for first)
Time 2ms:  Submit tx with timestamp=1002  (doesn't wait for either)

All three can be in the mempool simultaneously.
Whichever confirms first sets the new last_seen_timestamp.
The others will only succeed if their timestamp > that value.
```

### Handling Clock Skew

Validators can enforce a maximum timestamp drift:

```rust
// Reject transactions too far in the future
let max_drift = Duration::from_secs(5);
let block_time = env.block_timestamp();

if tx.timestamp() > block_time + max_drift.as_millis() as u64 {
    return Err(ERR_TIMESTAMP_TOO_FUTURE);
}
```

---

## Comparison: Nonce vs Timestamp

| Aspect | Nonce | Timestamp |
|--------|-------|-----------|
| Ordering | Strict sequential | Monotonic increasing |
| Parallel submit | No | Yes |
| Coordination | Required | None |
| Storage per account | 8 bytes (u64) | 8 bytes (u64) |
| Stuck tx handling | Complex | Simple (just use later ts) |
| Determinism | Perfect | Requires drift bounds |
| Max throughput | Unlimited (sequential) | ~1000 tx/sec/account |

---

## Best Practices

### For Ethereum Transactions
1. Always verify chain ID
2. Track pending transactions to avoid nonce gaps
3. Implement nonce management in your client SDK
4. Handle "nonce too low" by re-querying and retrying

### For Micro Transactions
1. Use current time + small offset for timestamp
2. Don't reuse timestamps even for different recipients

### General
1. Never accept transactions without chain ID in production
2. Log rejected transactions for monitoring
3. Test replay scenarios in your integration tests

---

## See Also

- [Ethereum Transactions](./ethereum.md) - Nonce-based transactions
- [Micro Transactions](./micro.md) - Timestamp-based transactions
- [STF Architecture](/docs/STF_ARCHITECTURE.md) - Transaction validation flow

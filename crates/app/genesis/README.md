# Genesis Module

The genesis module provides JSON-based genesis configuration for Evolve blockchains. Instead of hardcoding genesis logic, genesis is represented as an ordered list of unsigned transactions that execute before block 1.

## Design Philosophy

Traditional blockchain genesis implementations have a separate code path for initializing state - custom structs, dedicated initialization functions per module, and compiled-in configuration. This creates several problems:

1. **Two code paths**: Genesis initialization differs from runtime execution
2. **Not portable**: Genesis config is compiled into the binary
3. **Hard to audit**: Requires reading Rust code to understand genesis state

The Evolve genesis module takes a different approach: **genesis is just transactions**.

```
┌─────────────────────────────────────────────────────────┐
│                    Genesis File (JSON)                   │
│  ┌─────────────────────────────────────────────────┐    │
│  │ { "chain_id": "evolve-mainnet",                 │    │
│  │   "transactions": [                             │    │
│  │     { "id": "alice", "msg_type": "eoa/init" },  │    │
│  │     { "id": "token", "msg_type": "token/init",  │    │
│  │       "msg": { "owner": "$alice" } }            │    │
│  │   ] }                                           │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                   Reference Resolution                   │
│  • Parse JSON                                            │
│  • Resolve $references to AccountIds                     │
│  • Encode messages via MessageRegistry                   │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                     STF Execution                        │
│  • Block height = 0                                      │
│  • No signature verification                             │
│  • Infinite gas                                          │
│  • Same execution path as runtime                        │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                    Initial State                         │
│  Accounts created, tokens minted, parameters set         │
└─────────────────────────────────────────────────────────┘
```

### Benefits

- **Single execution path**: Genesis uses the same `do_exec` as runtime
- **Portable**: Genesis is a JSON file, not compiled code
- **Auditable**: Anyone can read the JSON to understand initial state
- **Testable**: Same transaction semantics in tests and production

## File Format

Genesis files are JSON with this structure:

```json
{
  "chain_id": "evolve-mainnet",
  "genesis_time": 1704067200000,
  "consensus_params": {
    "block_gas_limit": 30000000
  },
  "transactions": [
    {
      "id": "alice",
      "sender": "system",
      "recipient": "runtime",
      "msg_type": "eoa/initialize",
      "msg": {}
    },
    {
      "id": "atom",
      "sender": "system",
      "recipient": "runtime",
      "msg_type": "token/initialize",
      "msg": {
        "metadata": {
          "name": "uatom",
          "symbol": "ATOM",
          "decimals": 6
        },
        "balances": [
          ["$alice", 1000000]
        ],
        "supply_manager": "$alice"
      }
    }
  ]
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `chain_id` | string | Network identifier |
| `genesis_time` | u64 | Unix timestamp in milliseconds (optional, default 0) |
| `consensus_params` | object | Consensus parameters (optional, uses defaults) |
| `transactions` | array | Ordered list of genesis transactions |

### Consensus Parameters

| Field | Type | Description |
|-------|------|-------------|
| `block_gas_limit` | u64 | Maximum gas per block (default: 30,000,000) |

Consensus parameters are stored in state during genesis at a well-known key, allowing syncing nodes to read them for block validation.

### Transaction Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | string? | Optional identifier for referencing the created account |
| `sender` | sender_spec | Who sends the transaction (default: "system") |
| `recipient` | recipient_spec | Target account for the message |
| `msg_type` | string | Message type identifier (e.g., "token/initialize") |
| `msg` | object | Message payload as JSON |

### Sender Specification

- `"system"` - The system account (AccountId 0), used for privileged operations
- `"$ref"` - Reference to a previously created account
- `123` - Explicit account ID as a number

### Recipient Specification

- `"runtime"` - The runtime account, used for account creation
- `"$ref"` - Reference to a previously created account
- `123` - Explicit account ID as a number

## Reference Resolution

Genesis transactions often need to reference accounts created by earlier transactions. The `$reference` syntax handles this:

```json
{
  "transactions": [
    { "id": "alice", "msg_type": "eoa/initialize", "msg": {} },
    { "id": "token", "msg_type": "token/initialize",
      "msg": { "owner": "$alice" } }
  ]
}
```

When the genesis file is processed:

1. Transactions are processed in order
2. Each transaction with an `id` gets assigned the next sequential AccountId
3. `$references` in later transactions resolve to these AccountIds
4. References work in any JSON field: strings, arrays, nested objects

**Important**: References can only point to earlier transactions (forward references are invalid).

## Message Registry

The genesis module needs to convert JSON message payloads to borsh-encoded `InvokeRequest` objects. This is done via the `MessageRegistry` trait:

```rust
pub trait MessageRegistry {
    fn encode_message(
        &self,
        msg_type: &str,
        value: &serde_json::Value,
    ) -> Result<InvokeRequest, GenesisError>;

    fn list_message_types(&self) -> Vec<MessageTypeInfo>;
}
```

Applications implement this trait to register their message types:

```rust
use evolve_genesis::{SimpleRegistry, register_message};

fn build_registry() -> SimpleRegistry {
    let mut registry = SimpleRegistry::new();

    registry.register("eoa/initialize", "Create EOA account", |value| {
        let msg: EoaInitialize = serde_json::from_value(value.clone())?;
        InvokeRequest::new(&msg)
    });

    registry.register("token/initialize", "Create token", |value| {
        let msg: TokenInitialize = serde_json::from_value(value.clone())?;
        InvokeRequest::new(&msg)
    });

    registry
}
```

## Integration

### Loading Genesis

```rust
use evolve_genesis::{GenesisFile, apply_genesis};

// Load and parse
let genesis = GenesisFile::load("genesis.json")?;

// Validate structure
genesis.validate()?;

// Convert to transactions
let registry = build_registry();
let txs = genesis.to_transactions(&registry)?;

// Apply to state (consensus params are stored automatically)
let (state, results) = apply_genesis(
    &stf,
    &storage,
    &codes,
    &genesis.consensus_params,
    txs,
)?;

// Commit state changes
storage.apply_changes(state.into_changes()?)?;
```

### Reading Consensus Parameters

Syncing nodes can read consensus parameters from state:

```rust
use evolve_genesis::read_consensus_params;

let params = read_consensus_params(&storage)?;
println!("Block gas limit: {}", params.block_gas_limit);
```

### Error Handling

Genesis execution stops at the first error. The `GenesisError::TransactionFailed` variant includes:
- `index`: Which transaction failed (0-indexed)
- `id`: The transaction's `id` field if present
- `error`: Error message from execution

```rust
match apply_genesis(&stf, &storage, &codes, &genesis.consensus_params, txs) {
    Ok((state, results)) => { /* success */ }
    Err(GenesisError::TransactionFailed { index, id, error }) => {
        eprintln!("Genesis tx {} ({:?}) failed: {}", index, id, error);
    }
    Err(e) => { /* other error */ }
}
```

## Execution Model

Genesis transactions execute with special privileges:

| Property | Value | Reason |
|----------|-------|--------|
| Block height | 0 | Distinguishes genesis from runtime |
| Block time | 0 (or genesis_time) | Chain hasn't started yet |
| Signature verification | Disabled | No private keys at genesis |
| Gas metering | Infinite | No gas market at genesis |
| Execution context | system_exec | Privileged STF method |

All transactions share the same execution state, so earlier transactions' state changes are visible to later transactions.

## Best Practices

### Transaction Ordering

Order transactions by dependency:
1. Create accounts that other transactions reference
2. Create tokens/assets
3. Configure system parameters
4. Set up governance/validators

### Use Descriptive IDs

```json
{ "id": "treasury_multisig", ... }
{ "id": "governance_token", ... }
{ "id": "staking_pool", ... }
```

### Validate Before Deployment

```rust
// Dry-run validation
genesis.validate()?;
let txs = genesis.to_transactions(&registry)?;

// Optionally execute against in-memory storage
let test_storage = InMemoryStorage::new();
apply_genesis(&stf, &test_storage, &codes, &genesis.consensus_params, txs)?;
```

## Future Work

- **CLI tooling**: `evolve genesis init`, `evolve genesis validate`, `evolve genesis add-tx`
- **Schema generation**: Auto-generate JSON schemas from message types
- **Genesis export**: Reconstruct genesis file from chain state
- **Migration support**: Tools for genesis file versioning

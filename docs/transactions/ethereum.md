# Ethereum Transactions

The `evolve_tx_eth` crate provides EIP-2718 typed transaction support, enabling full Ethereum wallet compatibility.

## Supported Types

| Type | Byte | Description |
|------|------|-------------|
| Legacy | `0x00` | Pre-EIP-2718, optional EIP-155 replay protection |
| EIP-1559 | `0x02` | Fee market with base fee and priority fee |

Future types (EIP-2930, EIP-4844) can be added without breaking changes.

## Installation

```toml
[dependencies]
evolve_tx_eth = { workspace = true }
```

## Usage

### Decoding Transactions

```rust
use evolve_tx_eth::{TxEnvelope, TypedTxDecoder, SignatureVerifierRegistry};
use evolve_stf_traits::TxDecoder;

// Create decoder and verifier
let decoder = TypedTxDecoder::ethereum();
let verifier = SignatureVerifierRegistry::ethereum(chain_id);

// Decode raw transaction bytes
let tx = decoder.decode(&mut &raw_bytes[..])?;

// Verify signature and chain ID
verifier.verify(&tx)?;

// Access transaction data
let sender = tx.sender();
let recipient = tx.to();
let value = tx.value();
let nonce = tx.nonce();
```

### Transaction Envelope

The `TxEnvelope` enum holds any supported transaction type:

```rust
pub enum TxEnvelope {
    Legacy(SignedLegacyTx),
    Eip1559(SignedEip1559Tx),
}
```

All variants implement the `TypedTransaction` trait:

```rust
pub trait TypedTransaction: Send + Sync {
    fn tx_type(&self) -> u8;
    fn sender(&self) -> Address;
    fn tx_hash(&self) -> B256;
    fn gas_limit(&self) -> u64;
    fn chain_id(&self) -> Option<u64>;
    fn nonce(&self) -> u64;
    fn to(&self) -> Option<Address>;
    fn value(&self) -> U256;
    fn input(&self) -> &[u8];
}
```

## Signature Verification

The `SignatureVerifierRegistry` maps transaction types to their verifiers:

```rust
// Create registry with ECDSA verification for all Ethereum types
let mut registry = SignatureVerifierRegistry::ethereum(chain_id);

// Optionally add custom verifiers for new types
registry.register(0x80, CustomVerifier::new());

// Verify a transaction
registry.verify(&tx)?;
```

### Chain ID Verification

EIP-155 chain IDs prevent cross-chain replay attacks:

```rust
// Strict mode (default): reject txs without chain ID
let verifier = EcdsaVerifier::new(chain_id);

// Permissive mode: allow legacy txs without chain ID
let verifier = EcdsaVerifier::new_permissive(chain_id);
```

## Type Filtering

Control which transaction types are accepted:

```rust
// Only accept specific types
let decoder = TypedTxDecoder::with_types([
    tx_type::LEGACY,
    tx_type::EIP1559,
]);

// Add types dynamically
decoder.allow_type(tx_type::EIP2930);

// Check if type is allowed
if decoder.is_allowed(tx_type::EIP4844) {
    // ...
}
```

## Transaction Format

### Legacy Transaction (RLP)

```
rlp([nonce, gasPrice, gasLimit, to, value, data, v, r, s])
```

### EIP-1559 Transaction

```
0x02 || rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
             gasLimit, to, value, data, accessList, v, r, s])
```

## Error Handling

```rust
use evolve_tx_eth::{
    ERR_TX_DECODE,           // Failed to decode transaction
    ERR_SIGNATURE_RECOVERY,  // Signature recovery failed
    ERR_INVALID_CHAIN_ID,    // Chain ID mismatch
    ERR_UNSUPPORTED_TX_TYPE, // Unknown transaction type
};
```

## Dependencies

The crate uses the [Alloy](https://github.com/alloy-rs) ecosystem for Ethereum primitives:

- `alloy-primitives` - Address, B256, U256
- `alloy-consensus` - Transaction types with k256 signatures
- `alloy-rlp` - RLP encoding/decoding
- `alloy-eips` - EIP implementations (access lists, etc.)

## See Also

- [Micro Transactions](./micro.md) - For high-throughput use cases
- [Replay Protection](./replay-protection.md) - How nonce-based protection works

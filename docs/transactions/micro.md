# Micro Transactions

The `evolve_tx_micro` crate provides a minimal transaction format optimized for high-throughput scenarios like micropayments, streaming payments, and IoT applications.

## Key Features

- **150 bytes minimum** - Compact fixed-layout binary encoding
- **Ed25519 signatures** - Hardware wallet friendly, ~3x faster verification than ECDSA
- **Timestamp-based ordering** - No nonce coordination needed
- **Parallel submission** - Multiple txs from same account simultaneously
- **Minimal dependencies** - Just `alloy-primitives` and `ed25519-dalek`

## Installation

```toml
[dependencies]
evolve_tx_micro = { workspace = true }
```

## Binary Format

Micro transactions use a fixed-layout binary format for maximum decode performance. Fields are read directly from byte offsets without parsing.

```
| Field      | Offset | Size | Description                    |
|------------|--------|------|--------------------------------|
| chain_id   | 0      | 8    | u64 big-endian                 |
| from       | 8      | 32   | sender Ed25519 public key      |
| to         | 40     | 20   | recipient address              |
| amount     | 60     | 16   | u128 big-endian                |
| timestamp  | 76     | 8    | u64 big-endian (milliseconds)  |
| data_len   | 84     | 2    | u16 big-endian (max 65535)     |
| data       | 86     | var  | optional calldata              |
| signature  | 86+N   | 64   | Ed25519 signature              |
```

**Total: 150 bytes minimum (no calldata)**

## Usage

### Decoding Transactions

```rust
use evolve_tx_micro::{SignedMicroTx, MicroTx};

// Decode from bytes (type byte already stripped)
let tx = SignedMicroTx::decode(&bytes)?;

// Access transaction data
let pubkey = tx.sender_pubkey(); // Ed25519 public key (32 bytes)
let sender = tx.sender();        // Derived address (20 bytes)
let recipient = tx.recipient();
let amount = tx.amount();        // u128
let timestamp = tx.timestamp();  // milliseconds
let chain_id = tx.chain_id();
let hash = tx.tx_hash();
```

### Creating Unsigned Payloads

For signing on the client side:

```rust
use evolve_tx_micro::{create_unsigned_payload, hash_for_signing, address_from_pubkey};

// Create the payload to sign (from_pubkey is 32-byte Ed25519 public key)
let payload = create_unsigned_payload(
    chain_id,
    &from_pubkey,
    recipient_address,
    amount,
    timestamp_ms,
    &calldata,  // or &[] for simple transfer
);

// Get the hash to sign
let signing_hash = hash_for_signing(&payload);

// Sign with Ed25519 (hardware wallet, ed25519-dalek, etc.)
let signature = ed25519_sign(signing_hash, private_key);

// Derive address from pubkey if needed
let sender_address = address_from_pubkey(&from_pubkey);
```

### Encoding Transactions

```rust
// Encode back to bytes
let encoded = tx.encode();

// Prepend type byte for transmission
let mut wire_format = vec![MICRO_TX_TYPE];  // 0x83
wire_format.extend_from_slice(&encoded);
```

## The MicroTx Trait

All micro transaction types implement the `MicroTx` trait:

```rust
pub trait MicroTx {
    fn tx_type(&self) -> u8;
    fn sender_pubkey(&self) -> &[u8; 32];  // Ed25519 public key
    fn sender(&self) -> Address;            // Derived from pubkey
    fn recipient(&self) -> Address;
    fn amount(&self) -> u128;
    fn timestamp(&self) -> u64;
    fn chain_id(&self) -> u64;
    fn tx_hash(&self) -> B256;
    fn data(&self) -> &[u8];
}
```

Note the differences from Ethereum's `TypedTransaction`:
- No `nonce()` - uses timestamp instead
- No `gas_limit()`, `gas_price()` - gas handled separately
- `amount()` returns `u128` not `U256`
- `recipient()` always returns an address (no contract creation)
- `sender_pubkey()` returns the Ed25519 public key directly

## Why u128 Instead of U256?

The amount field uses `u128` (16 bytes) instead of `U256` (32 bytes):

- **Saves 16 bytes per transaction**
- `u128` max ≈ 340 undecillion - more than enough for any currency
- Even at mass scale (billions of txs), the savings are significant

If you need U256 for compatibility:

```rust
let amount_u256 = tx.amount_u256();  // Converts to U256
```

## Error Handling

```rust
use evolve_tx_micro::MicroTxError;

match SignedMicroTx::decode(&bytes) {
    Ok(tx) => { /* process tx */ }
    Err(MicroTxError::TooShort) => { /* input too small */ }
    Err(MicroTxError::InvalidDataLen) => { /* data_len > 65535 */ }
    Err(MicroTxError::SizeMismatch) => { /* actual size != expected */ }
    Err(MicroTxError::SignatureRecovery) => { /* invalid signature */ }
    Err(MicroTxError::SenderMismatch) => { /* recovered != declared */ }
}
```

## Performance Characteristics

### Decode Time

Fixed-layout decoding is essentially memcpy operations:

```rust
// Direct byte reads - no parsing
let chain_id = u64::from_be_bytes(bytes[0..8].try_into()?);
let from = Address::from_slice(&bytes[8..28]);
// etc.
```

### Memory

- No intermediate allocations during decode (except for variable `data`)
- Single hash computation for signing verification
- Single hash computation for tx_hash

### Signature Verification

Uses `ed25519-dalek` for Ed25519 signature verification:

```rust
use ed25519_dalek::{Signature, VerifyingKey};

let verifying_key = VerifyingKey::from_bytes(&pubkey)?;
let signature = Signature::from_bytes(&sig_bytes);
verifying_key.verify_strict(message, &signature)?;
```

Ed25519 advantages over ECDSA:
- ~3x faster verification
- Deterministic signatures (no random nonce issues)
- No signature malleability
- Widely supported by hardware wallets (Ledger, Trezor)

## Integration with Account Module

Micro transactions are designed to work with the `NoncelessAccount` module for replay protection:

```rust
// NoncelessAccount handles replay protection only
#[exec]
pub fn verify_and_update(
    &self,
    timestamp: u64,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    // 1. Verify sender is owner
    // 2. Verify timestamp > last_seen
    // 3. Update last_seen_timestamp
}
```

Value transfers are handled separately at the execution layer, similar to how
ETH nonce accounts work. This separation keeps the account module focused on
replay protection.

See [Replay Protection](./replay-protection.md) for details.

## When NOT to Use Micro Transactions

- You need Ethereum wallet compatibility
- You need EVM execution with access lists
- You need complex gas pricing (EIP-1559 tips)
- You need contract creation transactions
- Sequential ordering is strictly required

## See Also

- [Ethereum Transactions](./ethereum.md) - For wallet compatibility
- [Replay Protection](./replay-protection.md) - Timestamp-based protection
- [NoncelessAccount Module](/modules/nonceless-account) - Account-level replay protection

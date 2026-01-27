# Evolve Client Tutorial: Submitting Transactions

This tutorial covers how to interact with an Evolve node as a client, including transaction format, submission methods, calldata encoding, and using the schema system for discovery.

## Table of Contents

1. [Transaction Format](#transaction-format)
2. [Address and AccountId Mapping](#address-and-accountid-mapping)
3. [Calldata Encoding](#calldata-encoding)
4. [Submitting Transactions](#submitting-transactions)
5. [Using the Schema System](#using-the-schema-system)
6. [Complete Examples](#complete-examples)

---

## Transaction Format

Evolve uses **EIP-2718 typed transactions**, providing Ethereum wallet compatibility while supporting custom transaction types.

### Supported Transaction Types

| Type | Byte | Description |
|------|------|-------------|
| Legacy | `0x00` | Pre-EIP-2718 transactions (optional EIP-155 replay protection) |
| EIP-1559 | `0x02` | Fee market transactions with base fee + priority fee |
| Batch | `0x80` | Multi-message transactions (planned) |
| Sponsored | `0x81` | Gasless/meta-transactions (planned) |
| Scheduled | `0x82` | Delayed execution (planned) |

### EIP-1559 Transaction Structure

The recommended transaction type for new integrations:

```rust
TxEip1559 {
    chain_id: u64,                    // Network chain ID
    nonce: u64,                       // Sender's transaction count
    max_priority_fee_per_gas: u128,   // Tip for block producers
    max_fee_per_gas: u128,            // Maximum total fee
    gas_limit: u64,                   // Gas budget
    to: TxKind,                       // Recipient (Call or Create)
    value: U256,                      // Native token transfer amount
    input: Bytes,                     // Calldata (function selector + args)
    access_list: AccessList,          // Storage access hints (optional)
}
```

### Transaction Encoding

Transactions are RLP-encoded with an EIP-2718 type prefix:

```rust
// EIP-1559: type byte (0x02) + RLP-encoded signed transaction
let mut encoded = vec![0x02];
signed_tx.rlp_encode(&mut encoded);

// Legacy: RLP-encoded directly (starts with 0xc0-0xff)
signed_tx.rlp_encode(&mut encoded);
```

---

## Address and AccountId Mapping

Evolve uses 128-bit `AccountId` internally, while Ethereum uses 160-bit addresses. The mapping is deterministic and reversible for contract accounts.

### Address to AccountId

Takes the **last 16 bytes** of the 20-byte Ethereum address:

```rust
pub fn address_to_account_id(addr: Address) -> AccountId {
    let bytes = addr.as_slice();
    let mut id_bytes = [0u8; 16];
    id_bytes.copy_from_slice(&bytes[4..]);  // Skip first 4 bytes
    AccountId::new(u128::from_be_bytes(id_bytes))
}
```

### AccountId to Address

Pads with **4 zero bytes** at the start:

```rust
pub fn account_id_to_address(id: AccountId) -> Address {
    let id_bytes = id.as_bytes();
    let mut addr_bytes = [0u8; 20];
    addr_bytes[4..20].copy_from_slice(&id_bytes[..16]);
    Address::from_slice(&addr_bytes)
}
```

### Important Notes

- Round-trip is perfect: `address_to_account_id(account_id_to_address(id)) == id`
- EOA addresses from public keys have random first 4 bytes; these are lost in the mapping
- Contract addresses derived from AccountIds round-trip perfectly
- Two Ethereum addresses differing only in the first 4 bytes map to the same AccountId (collision)

---

## Calldata Encoding

Evolve uses a **function selector + Borsh-encoded arguments** format, different from Ethereum's ABI encoding.

### Function Selector

The selector is the first 4 bytes of the `keccak256` hash of the function name:

```rust
fn compute_selector(fn_name: &str) -> [u8; 4] {
    let hash = keccak256(fn_name.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

// Example
let selector = compute_selector("transfer");  // e.g., [0x83, 0xf7, ...]
```

### Argument Encoding

Arguments are encoded using **Borsh serialization**, not Ethereum ABI:

```rust
use borsh::BorshSerialize;

// For transfer(to: AccountId, amount: u128)
let args = borsh::to_vec(&(recipient_account_id, amount))?;
```

### Complete Calldata

```rust
let selector = compute_selector("transfer");
let args = borsh::to_vec(&(bob_account_id, 100u128))?;

let mut calldata = Vec::with_capacity(4 + args.len());
calldata.extend_from_slice(&selector);
calldata.extend_from_slice(&args);
```

### Why Borsh Instead of ABI?

1. **Compact**: Borsh produces smaller payloads than ABI encoding
2. **Deterministic**: Consistent encoding across languages
3. **Type-safe**: Strong type guarantees with no padding ambiguity
4. **Rust-native**: First-class support in the Rust ecosystem

---

## Submitting Transactions

### Method 1: JSON-RPC (Recommended for Clients)

Submit via the standard `eth_sendRawTransaction` endpoint:

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendRawTransaction",
    "params": ["0x02f8...encoded_tx..."],
    "id": 1
  }'
```

Response:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x...transaction_hash..."
}
```

### Method 2: Direct Mempool (For Testing/Embedded Use)

```rust
use evolve_mempool::new_shared_mempool;

// Create mempool
let mempool = new_shared_mempool(chain_id);

// Submit raw transaction bytes
let tx_hash = {
    let mut pool = mempool.write().await;
    pool.add_raw(&raw_tx_bytes)?
};

println!("Submitted: {:?}", tx_hash);
```

### Available RPC Methods

| Method | Description |
|--------|-------------|
| `eth_sendRawTransaction` | Submit signed transaction |
| `eth_getTransactionCount` | Get nonce for address |
| `eth_getTransactionReceipt` | Get execution result |
| `eth_getTransactionByHash` | Get transaction details |
| `eth_call` | Simulate call without state change |
| `eth_estimateGas` | Estimate gas for transaction |
| `eth_gasPrice` | Get current gas price |
| `eth_chainId` | Get network chain ID |
| `eth_blockNumber` | Get latest block number |

---

## Using the Schema System

The schema system enables runtime introspection of account modules, allowing clients to discover available functions without hardcoded knowledge.

> **See Also:** [Schema Introspection API](./schema-introspection.md) for comprehensive schema documentation, including gRPC endpoints and server setup.

### Schema RPC Endpoints

Evolve exposes module schemas via the `evolve_` namespace:

```bash
# List all registered modules
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_listModules","params":[],"id":1}'

# Get schema for a specific module
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_getModuleSchema","params":["Token"],"id":1}'

# Get all schemas
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_getAllSchemas","params":[],"id":1}'
```

### Using Schemas for Client Calls

When building calldata from schema information:

```rust
// Fetch schema
let schema: AccountSchema = rpc_client
    .call("evolve_getModuleSchema", ["Token"])
    .await?;

// Find the function
let transfer_fn = schema.exec_functions
    .iter()
    .find(|f| f.name == "transfer")
    .expect("transfer function not found");

// IMPORTANT: Use keccak256 selector for calldata, NOT function_id
// function_id (SHA-256, 8 bytes) is for internal dispatch
// calldata selector (keccak256, 4 bytes) is for transaction encoding
let selector = compute_selector(&transfer_fn.name);

// Encode args based on schema params
let args = encode_from_schema(&transfer_fn.params, [recipient, amount])?;
```

### Function ID vs Calldata Selector

| Concept | Hash | Size | Purpose |
|---------|------|------|---------|
| `function_id` | SHA-256 | 8 bytes (u64) | Internal message dispatch |
| Calldata selector | keccak256 | 4 bytes | Transaction encoding |

The `function_id` in schemas is for internal use. When building Ethereum-compatible transactions, always compute the keccak256 selector from the function name.

---

## Complete Examples

### Example 1: Token Transfer (Rust)

```rust
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Bytes, PrimitiveSignature, TxKind, U256};
use k256::ecdsa::SigningKey;
use tiny_keccak::{Hasher, Keccak};

// Helper: keccak256
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut keccak = Keccak::v256();
    let mut output = [0u8; 32];
    keccak.update(data);
    keccak.finalize(&mut output);
    output
}

// Helper: function selector
fn compute_selector(fn_name: &str) -> [u8; 4] {
    let hash = keccak256(fn_name.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

// Helper: sign transaction
fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
    let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).unwrap();
    let r = U256::from_be_slice(&sig.r().to_bytes());
    let s = U256::from_be_slice(&sig.s().to_bytes());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

async fn transfer_tokens(
    signing_key: &SigningKey,
    chain_id: u64,
    nonce: u64,
    token_address: Address,
    recipient: AccountId,
    amount: u128,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // 1. Build calldata
    let selector = compute_selector("transfer");
    let args = borsh::to_vec(&(recipient, amount))?;

    let mut calldata = Vec::with_capacity(4 + args.len());
    calldata.extend_from_slice(&selector);
    calldata.extend_from_slice(&args);

    // 2. Create transaction
    let tx = TxEip1559 {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1_000_000_000,  // 1 gwei
        max_fee_per_gas: 20_000_000_000,          // 20 gwei
        gas_limit: 100_000,
        to: TxKind::Call(token_address),
        value: U256::ZERO,
        input: Bytes::from(calldata),
        access_list: Default::default(),
    };

    // 3. Sign transaction
    let signature = sign_hash(signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    // 4. Encode with type prefix
    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    Ok(encoded)
}
```

### Example 2: Query Balance (JSON-RPC)

```bash
# First, get the token contract address (from genesis or known deployment)
TOKEN_ADDRESS="0x0000000000000000112233445566778899aabbcc"

# Use eth_call to simulate a query
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_call",
    "params": [{
      "to": "'$TOKEN_ADDRESS'",
      "data": "0x..."
    }, "latest"],
    "id": 1
  }'
```

### Example 3: TypeScript/JavaScript Client

```typescript
import { ethers } from "ethers";
import * as borsh from "borsh";

// Borsh schema for transfer arguments
class TransferArgs {
  to: Uint8Array;      // AccountId as 16 bytes
  amount: bigint;      // u128

  constructor(to: Uint8Array, amount: bigint) {
    this.to = to;
    this.amount = amount;
  }
}

const transferArgsSchema = {
  struct: {
    to: { array: { type: "u8", len: 16 } },
    amount: "u128",
  },
};

function computeSelector(fnName: string): Uint8Array {
  const hash = ethers.keccak256(ethers.toUtf8Bytes(fnName));
  return ethers.getBytes(hash).slice(0, 4);
}

function accountIdToAddress(accountId: bigint): string {
  const bytes = new Uint8Array(20);
  const idBytes = new Uint8Array(16);

  // Convert bigint to bytes (big-endian)
  for (let i = 15; i >= 0; i--) {
    idBytes[i] = Number(accountId & 0xffn);
    accountId >>= 8n;
  }

  // Pad with 4 zero bytes, then 16 bytes of AccountId
  bytes.set(idBytes, 4);
  return ethers.hexlify(bytes);
}

async function sendTransfer(
  wallet: ethers.Wallet,
  tokenAddress: string,
  recipientAccountId: bigint,
  amount: bigint
) {
  const provider = wallet.provider!;

  // Build calldata
  const selector = computeSelector("transfer");

  // Encode recipient AccountId as 16 bytes (big-endian)
  const recipientBytes = new Uint8Array(16);
  let temp = recipientAccountId;
  for (let i = 15; i >= 0; i--) {
    recipientBytes[i] = Number(temp & 0xffn);
    temp >>= 8n;
  }

  // Borsh encode: (AccountId, u128) - AccountId is 16 bytes, u128 is 16 bytes little-endian
  const args = new Uint8Array(32);
  args.set(recipientBytes, 0);  // AccountId (big-endian as stored)

  // amount as little-endian u128
  let amountTemp = amount;
  for (let i = 0; i < 16; i++) {
    args[16 + i] = Number(amountTemp & 0xffn);
    amountTemp >>= 8n;
  }

  const calldata = new Uint8Array(4 + args.length);
  calldata.set(selector, 0);
  calldata.set(args, 4);

  // Send transaction
  const tx = await wallet.sendTransaction({
    to: tokenAddress,
    data: ethers.hexlify(calldata),
    gasLimit: 100000,
  });

  console.log("Transaction hash:", tx.hash);
  const receipt = await tx.wait();
  console.log("Confirmed in block:", receipt?.blockNumber);

  return tx.hash;
}

// Usage
const provider = new ethers.JsonRpcProvider("http://localhost:8545");
const wallet = new ethers.Wallet(PRIVATE_KEY, provider);

const tokenAccountId = 12345n;  // Your token's AccountId
const tokenAddress = accountIdToAddress(tokenAccountId);

await sendTransfer(wallet, tokenAddress, recipientAccountId, 100n);
```

### Example 4: Python Client

```python
from web3 import Web3
import hashlib
import struct

def keccak256(data: bytes) -> bytes:
    return Web3.keccak(data)

def compute_selector(fn_name: str) -> bytes:
    return keccak256(fn_name.encode())[:4]

def account_id_to_address(account_id: int) -> str:
    """Convert 128-bit AccountId to Ethereum address."""
    id_bytes = account_id.to_bytes(16, 'big')
    addr_bytes = bytes(4) + id_bytes  # Pad with 4 zero bytes
    return Web3.to_checksum_address(addr_bytes.hex())

def encode_transfer_args(recipient_id: int, amount: int) -> bytes:
    """Borsh-encode transfer arguments: (AccountId, u128)."""
    # AccountId: 16 bytes big-endian (as stored internally)
    recipient_bytes = recipient_id.to_bytes(16, 'big')
    # u128 amount: 16 bytes little-endian (Borsh convention)
    amount_bytes = amount.to_bytes(16, 'little')
    return recipient_bytes + amount_bytes

def send_transfer(
    w3: Web3,
    private_key: str,
    token_address: str,
    recipient_id: int,
    amount: int
) -> str:
    account = w3.eth.account.from_key(private_key)

    # Build calldata
    selector = compute_selector("transfer")
    args = encode_transfer_args(recipient_id, amount)
    calldata = selector + args

    # Build transaction
    tx = {
        'to': token_address,
        'data': calldata,
        'gas': 100000,
        'maxFeePerGas': w3.to_wei(20, 'gwei'),
        'maxPriorityFeePerGas': w3.to_wei(1, 'gwei'),
        'nonce': w3.eth.get_transaction_count(account.address),
        'chainId': w3.eth.chain_id,
    }

    # Sign and send
    signed = account.sign_transaction(tx)
    tx_hash = w3.eth.send_raw_transaction(signed.raw_transaction)

    print(f"Transaction hash: {tx_hash.hex()}")
    receipt = w3.eth.wait_for_transaction_receipt(tx_hash)
    print(f"Confirmed in block: {receipt['blockNumber']}")

    return tx_hash.hex()

# Usage
w3 = Web3(Web3.HTTPProvider('http://localhost:8545'))
token_account_id = 12345
token_address = account_id_to_address(token_account_id)

send_transfer(
    w3,
    PRIVATE_KEY,
    token_address,
    recipient_account_id=67890,
    amount=100
)
```

---

## Key Takeaways

1. **Transaction Format**: Use EIP-1559 transactions for new integrations; standard Ethereum signing works.

2. **Address Mapping**: AccountId uses last 16 bytes of address; use `account_id_to_address()` to target contracts.

3. **Calldata**: `selector (4 bytes) + Borsh-encoded args` — not ABI encoding.

4. **Submission**: Use `eth_sendRawTransaction` via JSON-RPC; compatible with ethers.js, web3.py, etc.

5. **Schema Discovery**: Use `evolve_*` RPC methods to introspect modules without hardcoded ABIs.

6. **Borsh Encoding**: Arguments are little-endian for integers; AccountId is stored as 16 bytes.

---

## Troubleshooting

### Transaction Rejected

- **Invalid chain ID**: Ensure `chain_id` matches the network
- **Nonce too low**: Fetch current nonce with `eth_getTransactionCount`
- **Insufficient gas**: Increase `gas_limit` or use `eth_estimateGas`
- **Invalid signature**: Verify signing key matches sender address

### Calldata Encoding Errors

- **Wrong selector**: Verify function name spelling; selectors are case-sensitive
- **Borsh encoding mismatch**: Ensure argument types match schema exactly
- **Endianness**: Borsh uses little-endian for integers; AccountId is big-endian bytes

### Schema Not Found

- **Module not registered**: Check module identifier with `evolve_listModules`
- **Case sensitivity**: Module identifiers are case-sensitive

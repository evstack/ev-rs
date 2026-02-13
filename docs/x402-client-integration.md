# X402 Client Integration Guide

How to connect and interact with Evolve-powered APIs using the [X402 payment protocol](https://x402.org).

---

## What is X402?

X402 uses HTTP status code **402 (Payment Required)** to create a machine-readable payment flow between clients and APIs. Instead of API keys or subscriptions, each request is paid individually on-chain.

```
Client                          Server                         Evolve Node
  │                               │                               │
  │  1. POST /api/transform/hash  │                               │
  │──────────────────────────────>│                               │
  │                               │                               │
  │  2. 402 + PAYMENT-REQUIRED    │                               │
  │<──────────────────────────────│                               │
  │                               │                               │
  │  3. Token transfer tx         │                               │
  │───────────────────────────────┼──────────────────────────────>│
  │  4. txHash                    │                               │
  │<──────────────────────────────┼───────────────────────────────│
  │                               │                               │
  │  5. POST + PAYMENT-SIGNATURE  │                               │
  │──────────────────────────────>│                               │
  │                               │  6. Verify tx on-chain        │
  │                               │──────────────────────────────>│
  │                               │<──────────────────────────────│
  │                               │                               │
  │  7. 200 + result              │                               │
  │<──────────────────────────────│                               │
```

---

## Prerequisites

### What You Need

| Requirement | Details |
|-------------|---------|
| **Ethereum wallet** | Any secp256k1 keypair (MetaMask, viem, ethers.js, etc.) |
| **Funded account** | Tokens on the Evolve chain to pay for API calls |
| **HTTP client** | Any language that can make HTTP requests and set custom headers |
| **Evolve RPC access** | JSON-RPC endpoint to submit transactions (default: `http://localhost:8545`) |

### Recommended Libraries (TypeScript/JavaScript)

```json
{
  "viem": "^2.21.0"
}
```

`viem` handles wallet creation, transaction signing, calldata encoding, and RPC communication. It's the only required dependency for a client integration.

---

## Step-by-Step Integration

### Step 1: Discover Pricing

Query the server to get available endpoints and their prices:

```bash
curl http://localhost:3000/api/pricing
```

Response:

```json
{
  "treasury": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
  "network": "evolve:1",
  "asset": "native",
  "endpoints": [
    { "route": "POST /api/transform/echo",      "price": "100", "description": "Echo - returns input unchanged" },
    { "route": "POST /api/transform/reverse",    "price": "100", "description": "Reverse - reverses input string" },
    { "route": "POST /api/transform/uppercase",  "price": "100", "description": "Uppercase - uppercases input string" },
    { "route": "POST /api/transform/hash",       "price": "200", "description": "Hash - returns SHA256 of input" }
  ]
}
```

### Step 2: Make the Initial Request (Get 402)

Send a normal request to a protected endpoint:

```bash
curl -X POST http://localhost:3000/api/transform/reverse \
  -H "Content-Type: application/json" \
  -d '{"input": "hello world"}'
```

The server responds with **HTTP 402** and a `PAYMENT-REQUIRED` header:

```
HTTP/1.1 402 Payment Required
PAYMENT-REQUIRED: eyJ4NDAyVmVyc2lvbiI6MiwiZXJyb3IiOi...

{
  "error": "payment_required",
  "description": "Reverse - reverses input string",
  "price": "100",
  "payTo": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
}
```

The `PAYMENT-REQUIRED` header is a **base64-encoded JSON** object with full X402 protocol details:

```json
{
  "x402Version": 2,
  "error": "payment_required",
  "resource": {
    "url": "http://localhost:3000/api/transform/reverse",
    "method": "POST"
  },
  "accepts": [
    {
      "scheme": "exact",
      "network": "evolve:1",
      "asset": "native",
      "amount": "100",
      "payTo": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
      "maxTimeoutSeconds": 300
    }
  ],
  "description": "Reverse - reverses input string"
}
```

### Step 3: Submit Payment On-Chain

Pay by sending a **token transfer** transaction to the Evolve node. Payments use token calldata (not native `value` transfers).

#### Understanding Evolve Address Mapping

Evolve uses 128-bit `AccountId` internally. Ethereum addresses (20 bytes) map to AccountIds by taking the last 16 bytes:

```
Ethereum address: 0x 0000 0000 3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
                       ^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                       padding   AccountId (16 bytes = 128 bits)
```

#### Building the Transfer Calldata

The token contract expects calldata in this format:

```
[4 bytes: function selector][16 bytes: recipient AccountId (LE)][16 bytes: amount (LE)]
```

- **Function selector**: first 4 bytes of `keccak256("transfer")` = `0xa9059cbb` (but computed via Evolve's convention, not Solidity's)
- **Recipient**: the treasury's AccountId as 16 bytes, **little-endian**
- **Amount**: the price as u128, **little-endian**

#### TypeScript Example with viem

```typescript
import {
  createWalletClient,
  createPublicClient,
  http,
  defineChain,
  keccak256,
  toBytes,
  bytesToHex,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";

// -- Evolve-specific helpers --

function addressToAccountId(address: `0x${string}`): bigint {
  // Strip "0x" prefix, skip first 8 hex chars (4 bytes padding)
  const idHex = address.slice(10);
  return BigInt(`0x${idHex}`);
}

function accountIdToAddress(id: bigint): `0x${string}` {
  const idBytes = new Uint8Array(16);
  let v = id;
  for (let i = 15; i >= 0; i--) {
    idBytes[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  const addrBytes = new Uint8Array(20);
  addrBytes.set(idBytes, 4); // 4 bytes padding prefix
  return bytesToHex(addrBytes) as `0x${string}`;
}

function u128ToLeBytes(value: bigint): Uint8Array {
  const bytes = new Uint8Array(16);
  let v = value;
  for (let i = 0; i < 16; i++) {
    bytes[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return bytes;
}

function buildTransferData(toAccountId: bigint, amount: bigint): `0x${string}` {
  // Function selector: keccak256("transfer"), first 4 bytes
  const selector = keccak256(toBytes("transfer")).slice(0, 10);
  // Arguments: [16 bytes toAccountId LE][16 bytes amount LE]
  const args = new Uint8Array(32);
  args.set(u128ToLeBytes(toAccountId), 0);
  args.set(u128ToLeBytes(amount), 16);
  // Combine selector + args
  const data = new Uint8Array(4 + args.length);
  data.set(Buffer.from(selector.slice(2), "hex"), 0);
  data.set(args, 4);
  return bytesToHex(data) as `0x${string}`;
}

// -- Setup --

const evolveChain = defineChain({
  id: 1337, // check via eth_chainId
  name: "Evolve Testnet",
  nativeCurrency: { decimals: 18, name: "Evolve", symbol: "EVO" },
  rpcUrls: { default: { http: ["http://localhost:8545"] } },
});

const account = privateKeyToAccount("0xYOUR_PRIVATE_KEY");

const walletClient = createWalletClient({
  account,
  chain: evolveChain,
  transport: http("http://localhost:8545"),
});

const publicClient = createPublicClient({
  chain: evolveChain,
  transport: http("http://localhost:8545"),
});

// -- Submit payment --

async function submitPayment(payTo: `0x${string}`, amount: bigint): Promise<`0x${string}`> {
  const TOKEN_ACCOUNT_ID = 65537n; // Token contract AccountId (check genesis)
  const tokenAddress = accountIdToAddress(TOKEN_ACCOUNT_ID);
  const recipientAccountId = addressToAccountId(payTo);
  const data = buildTransferData(recipientAccountId, amount);

  const txHash = await walletClient.sendTransaction({
    to: tokenAddress,
    data,
    value: 0n,
    gas: 100_000n,
  });

  // Wait for confirmation
  await publicClient.waitForTransactionReceipt({ hash: txHash });

  return txHash;
}
```

### Step 4: Retry with Payment Proof

After the transaction is confirmed, build a `PaymentPayload` and send it as a base64-encoded `PAYMENT-SIGNATURE` header:

```typescript
async function callPaidEndpoint(
  url: string,
  method: string,
  body: unknown,
  payTo: `0x${string}`,
  amount: bigint
): Promise<Response> {
  // Step 1: Initial request
  const initialResponse = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (initialResponse.status !== 402) {
    return initialResponse; // No payment needed (or error)
  }

  // Step 2: Parse payment requirement
  const paymentHeader = initialResponse.headers.get("PAYMENT-REQUIRED");
  const paymentRequired = JSON.parse(
    Buffer.from(paymentHeader!, "base64").toString("utf-8")
  );

  const requirement = paymentRequired.accepts[0];

  // Step 3: Pay on-chain
  const txHash = await submitPayment(
    requirement.payTo as `0x${string}`,
    BigInt(requirement.amount)
  );

  // Step 4: Build payment proof
  const paymentPayload = {
    x402Version: 2,
    scheme: "exact",
    network: requirement.network,
    payload: { txHash },
  };

  const paymentSignature = Buffer.from(
    JSON.stringify(paymentPayload)
  ).toString("base64");

  // Step 5: Retry with proof
  return fetch(url, {
    method,
    headers: {
      "Content-Type": "application/json",
      "PAYMENT-SIGNATURE": paymentSignature,
    },
    body: JSON.stringify(body),
  });
}

// Usage
const response = await callPaidEndpoint(
  "http://localhost:3000/api/transform/reverse",
  "POST",
  { input: "hello world" },
  "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
  100n
);

const result = await response.json();
// { "output": "dlrow olleh", "operation": "reverse", "cost": "100", "txHash": "0x..." }
```

---

## Protocol Reference

### Headers

| Header | Direction | Encoding | Description |
|--------|-----------|----------|-------------|
| `PAYMENT-REQUIRED` | Server -> Client | Base64 JSON | Payment requirements (amount, recipient, network) |
| `PAYMENT-SIGNATURE` | Client -> Server | Base64 JSON | Payment proof (txHash) |
| `PAYMENT-RESPONSE` | Server -> Client | Base64 JSON | Settlement confirmation |
| `X-Agent-ID` | Client -> Server | Plain text | Optional: identify the paying agent |

### PaymentRequired (Server -> Client)

```typescript
{
  x402Version: 2,
  error: "payment_required",
  resource: {
    url: string,      // Requested URL
    method: string,    // HTTP method
  },
  accepts: [{
    scheme: "exact",
    network: string,   // e.g. "evolve:1"
    asset: string,     // e.g. "native"
    amount: string,    // Price in token units (no decimals)
    payTo: string,     // Treasury Ethereum address
    maxTimeoutSeconds: number,
  }],
  description?: string,
}
```

### PaymentPayload (Client -> Server)

```typescript
{
  x402Version: 2,
  scheme: "exact",
  network: string,     // Must match requirement
  payload: {
    txHash: string,    // On-chain transaction hash (0x-prefixed)
  }
}
```

### PaymentSettled (Server -> Client)

Returned in the `PAYMENT-RESPONSE` header on successful payment:

```typescript
{
  x402Version: 2,
  success: boolean,
  transaction?: string,  // Confirmed txHash
  error?: string,
}
```

---

## Evolve-Specific Details

### Token Transfers (not native value)

Evolve's execution layer ignores the `value` field on transactions. Payments are made via **token contract calldata**:

```
Transaction:
  to:    <token contract address>
  value: 0
  data:  <transfer calldata>
```

The token contract address is derived from its AccountId at genesis (typically `65537`).

### Calldata Encoding

Evolve uses its own ABI encoding, different from Solidity:

```
[4 bytes]  Function selector = keccak256("transfer")[0..4]
[16 bytes] Recipient AccountId (little-endian u128)
[16 bytes] Amount (little-endian u128)
```

Total: **36 bytes** of calldata.

### Nonce Management

Under load, the JSON-RPC `eth_getTransactionCount` may lag. For high-throughput clients:

1. Fetch nonce once at startup via `eth_getTransactionCount` with `blockTag: "pending"`
2. Increment locally for each subsequent transaction
3. If you get a "nonce too high" error, re-fetch from the node

### Chain Configuration

| Parameter | Default | How to Discover |
|-----------|---------|-----------------|
| Chain ID | 1337 | `eth_chainId` RPC call |
| Token AccountId | 65537 | Check genesis file or `/health` endpoint |
| RPC URL | `http://localhost:8545` | Server config |
| Gas limit per tx | 100,000 | Sufficient for token transfers |

---

## Monitoring

### Health Check

```bash
curl http://localhost:3000/health
```

```json
{
  "status": "ok",
  "mode": "json-rpc",
  "chain": { "id": 1337, "blockNumber": "42" },
  "x402": {
    "treasury": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
    "network": "evolve:1",
    "asset": "native"
  }
}
```

### Treasury Balance

```bash
curl http://localhost:3000/api/treasury
```

```json
{
  "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
  "balance": "15000"
}
```

### Real-Time Events (WebSocket)

Connect to `ws://localhost:3000/ws/events` for live payment stream:

```typescript
const ws = new WebSocket("ws://localhost:3000/ws/events");

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  // Types: payment_submitted, payment_confirmed, payment_failed, request_served
  console.log(data.type, data.agentId, data.txHash);
};
```

Event format:

```json
{
  "type": "payment_confirmed",
  "timestamp": 1700000000000,
  "agentId": "agent-01",
  "txHash": "0xabc...",
  "amount": "100",
  "recipient": "0x3C44..."
}
```

---

## Quick Reference: Complete Flow in curl

```bash
# 1. Get 402 response
PAYMENT_HEADER=$(curl -s -D - -X POST http://localhost:3000/api/transform/reverse \
  -H "Content-Type: application/json" \
  -d '{"input":"hello"}' 2>&1 | grep -i "payment-required:" | awk '{print $2}' | tr -d '\r')

# 2. Decode payment details
echo $PAYMENT_HEADER | base64 -d | jq .

# 3. Send token transfer via JSON-RPC (using viem/ethers/cast)
#    ... get txHash ...

# 4. Build payment proof
PROOF=$(echo -n '{"x402Version":2,"scheme":"exact","network":"evolve:1","payload":{"txHash":"0xYOUR_TX_HASH"}}' | base64)

# 5. Retry with proof
curl -X POST http://localhost:3000/api/transform/reverse \
  -H "Content-Type: application/json" \
  -H "PAYMENT-SIGNATURE: $PROOF" \
  -d '{"input":"hello"}'

# Response: {"output":"olleh","operation":"reverse","cost":"100","txHash":"0x..."}
```

---

## Integration with Other Languages

The X402 protocol is language-agnostic. Any client that can:

1. Make HTTP requests with custom headers
2. Parse base64-encoded JSON
3. Sign and submit Ethereum transactions (EIP-1559 or legacy)

can integrate with X402 on Evolve. The key Evolve-specific part is the **calldata encoding** for token transfers (little-endian u128 arguments instead of Solidity's ABI encoding).

### Python Example (with web3.py)

```python
import requests, json, base64
from web3 import Web3
from eth_account import Account

w3 = Web3(Web3.HTTPProvider("http://localhost:8545"))
acct = Account.from_key("0xYOUR_PRIVATE_KEY")

# Step 1: Get 402
resp = requests.post("http://localhost:3000/api/transform/reverse",
                     json={"input": "hello"})

# Step 2: Parse requirement
payment_req = json.loads(base64.b64decode(resp.headers["PAYMENT-REQUIRED"]))
amount = int(payment_req["accepts"][0]["amount"])
pay_to = payment_req["accepts"][0]["payTo"]

# Step 3: Build calldata and send tx (see Evolve encoding above)
# ... build_transfer_data(pay_to_account_id, amount) ...

# Step 4: Retry with proof
proof = base64.b64encode(json.dumps({
    "x402Version": 2,
    "scheme": "exact",
    "network": "evolve:1",
    "payload": {"txHash": tx_hash}
}).encode()).decode()

result = requests.post("http://localhost:3000/api/transform/reverse",
                       json={"input": "hello"},
                       headers={"PAYMENT-SIGNATURE": proof})
```

### Go Example (with go-ethereum)

```go
// Same flow: HTTP request -> parse 402 -> submit tx -> retry with proof
// Key difference: implement buildTransferData with little-endian encoding
// Use go-ethereum's ethclient for transaction submission
```

---

## Error Handling

| HTTP Status | Meaning | Action |
|-------------|---------|--------|
| 402 (no PAYMENT-SIGNATURE header) | Payment required | Parse PAYMENT-REQUIRED, pay, retry |
| 402 (with PAYMENT-SIGNATURE header) | Payment verification failed | Check tx status, amount, or replay |
| 400 | Invalid payment header | Fix base64 encoding or payload format |
| 503 | Evolve node unavailable | Retry later |
| 200 | Success | Parse result from response body |

Common errors in the 402 verification response:

```json
{"error": "Payment verification failed", "reason": "Transaction not found"}
{"error": "Payment verification failed", "reason": "Transaction failed"}
{"error": "Payment verification failed", "reason": "Transaction already used"}
```

---

## Security Considerations

- **Replay protection**: Each `txHash` can only be used once. The server caches used hashes.
- **Transaction timeout**: Payments must be submitted within `maxTimeoutSeconds` (default 300s).
- **On-chain verification**: The server verifies the transaction receipt on the Evolve node before granting access.
- **No API keys**: Authentication is purely based on on-chain payment proof.

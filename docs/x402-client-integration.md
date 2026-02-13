# X402 Integration Guide for Evolve

How to build and consume [X402](https://x402.org)-protected APIs on the Evolve blockchain using JavaScript.

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

## Part 1: Building an X402-Protected Server

This section shows how to create an API server that requires on-chain payment for access, using [Hono](https://hono.dev) and the `@x402` libraries.

### Dependencies

```json
{
  "hono": "^4.6.0",
  "@x402/core": "^2.2.0",
  "@x402/hono": "^2.2.0",
  "viem": "^2.21.0"
}
```

### Step 1: Define Routes and Pricing

Each protected route needs a `RouteConfig` that specifies the payment scheme, price, and recipient.

```typescript
import type { Address } from "viem";
import type { Network } from "@x402/core/types";
import type { RouteConfig, RoutesConfig } from "@x402/core/http";

const TREASURY_ADDRESS: Address = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
const NETWORK: Network = "evolve:1337" as Network;

function route(price: string, description: string): RouteConfig {
  return {
    accepts: { scheme: "exact", payTo: TREASURY_ADDRESS, price, network: NETWORK },
    description,
    mimeType: "application/json",
  };
}

const PROTECTED_ROUTES: RoutesConfig = {
  "POST /api/transform/echo":      route("100", "Echo - returns input unchanged"),
  "POST /api/transform/reverse":   route("100", "Reverse - reverses input string"),
  "POST /api/transform/uppercase": route("100", "Uppercase - uppercases input string"),
  "POST /api/transform/hash":      route("200", "Hash - returns SHA256 of input"),
};
```

The keys in `RoutesConfig` follow the format `"METHOD /path"`. The `price` is in raw token units (no decimals).

### Step 2: Implement the Facilitator

The facilitator verifies payment transactions on-chain and settles them. Implement the `FacilitatorClient` interface from `@x402/core/server`.

```typescript
import type {
  PaymentPayload,
  PaymentRequirements,
  VerifyResponse,
  SettleResponse,
  SupportedResponse,
  Network,
} from "@x402/core/types";
import type { FacilitatorClient } from "@x402/core/server";
import { createPublicClient, http, defineChain } from "viem";

const evolveChain = defineChain({
  id: 1337,
  name: "Evolve Testnet",
  nativeCurrency: { decimals: 18, name: "Evolve Token", symbol: "EVO" },
  rpcUrls: { default: { http: ["http://127.0.0.1:8545"] } },
});

const publicClient = createPublicClient({
  chain: evolveChain,
  transport: http("http://127.0.0.1:8545"),
});

class EvolveFacilitator implements FacilitatorClient {
  private usedTxHashes = new Map<string, number>();
  private network: Network;

  constructor(network: Network) {
    this.network = network;
  }

  async verify(
    paymentPayload: PaymentPayload,
    _paymentRequirements: PaymentRequirements,
  ): Promise<VerifyResponse> {
    // Evict stale entries (older than 1 hour)
    const now = Date.now();
    for (const [hash, ts] of this.usedTxHashes) {
      if (now - ts > 3_600_000) this.usedTxHashes.delete(hash);
    }

    const txHash = paymentPayload.payload.txHash as string;

    if (!txHash) {
      return { isValid: false, invalidReason: "Missing transaction hash" };
    }

    if (this.usedTxHashes.has(txHash)) {
      return { isValid: false, invalidReason: "Transaction already used" };
    }

    try {
      const receipt = await publicClient.getTransactionReceipt({
        hash: txHash as `0x${string}`,
      });

      if (receipt.status !== "success") {
        return { isValid: false, invalidReason: "Transaction failed" };
      }

      return { isValid: true, payer: txHash.slice(0, 42) };
    } catch {
      return { isValid: false, invalidReason: "Transaction not found" };
    }
  }

  async settle(
    paymentPayload: PaymentPayload,
    _paymentRequirements: PaymentRequirements,
  ): Promise<SettleResponse> {
    const txHash = paymentPayload.payload.txHash as string;
    this.usedTxHashes.set(txHash, Date.now());

    return {
      success: true,
      transaction: txHash,
      network: this.network,
      payer: txHash.slice(0, 42),
    };
  }

  async getSupported(): Promise<SupportedResponse> {
    return {
      kinds: [{ x402Version: 2, scheme: "exact", network: this.network }],
      extensions: [],
      signers: {},
    };
  }
}
```

`verify()` is called before your route handler runs. `settle()` is called after the handler returns a successful response. This prevents charging for failed requests.

### Step 3: Implement the Scheme Server

The scheme server tells the framework how to parse prices for your payment scheme.

```typescript
import type {
  SchemeNetworkServer,
  PaymentRequirements,
  Network,
  Price,
  AssetAmount,
} from "@x402/core/types";

class EvolveSchemeServer implements SchemeNetworkServer {
  readonly scheme = "exact";

  async parsePrice(price: Price, _network: Network): Promise<AssetAmount> {
    const amount =
      typeof price === "object" && "amount" in price
        ? price.amount
        : String(price);
    return { amount, asset: "native" };
  }

  async enhancePaymentRequirements(
    paymentRequirements: PaymentRequirements,
    _supportedKind: { x402Version: number; scheme: string; network: Network },
    _facilitatorExtensions: string[],
  ): Promise<PaymentRequirements> {
    return paymentRequirements;
  }
}
```

For Evolve, prices are raw token amounts, so `parsePrice` just passes the value through.

### Step 4: Wire It All Together

```typescript
import { Hono } from "hono";
import { cors } from "hono/cors";
import { x402ResourceServer } from "@x402/core/server";
import { paymentMiddleware } from "@x402/hono";

const app = new Hono();

// CORS: expose x402 headers so clients can read them
app.use("*", cors({
  origin: "*",
  allowHeaders: ["Content-Type", "PAYMENT-SIGNATURE", "X-Agent-ID"],
  exposeHeaders: ["PAYMENT-REQUIRED", "PAYMENT-RESPONSE"],
}));

// Set up x402 resource server
const facilitator = new EvolveFacilitator(NETWORK);
const resourceServer = new x402ResourceServer(facilitator);
resourceServer.register(NETWORK, new EvolveSchemeServer());

// Apply payment middleware to protected routes
app.use("/api/transform/*", paymentMiddleware(PROTECTED_ROUTES, resourceServer));

// Pricing discovery endpoint (unprotected)
app.get("/api/pricing", (c) => {
  const endpoints = Object.entries(PROTECTED_ROUTES).map(([route, config]) => ({
    route,
    price: String((config as { accepts: { price: string } }).accepts.price),
    description: (config as { description?: string }).description ?? "",
  }));
  return c.json({ treasury: TREASURY_ADDRESS, network: NETWORK, endpoints });
});

// Protected routes — only reachable after x402 payment
app.post("/api/transform/echo", async (c) => {
  const { input } = await c.req.json<{ input: string }>();
  return c.json({ output: input, operation: "echo" });
});

app.post("/api/transform/reverse", async (c) => {
  const { input } = await c.req.json<{ input: string }>();
  return c.json({ output: input.split("").reverse().join(""), operation: "reverse" });
});

app.post("/api/transform/uppercase", async (c) => {
  const { input } = await c.req.json<{ input: string }>();
  return c.json({ output: input.toUpperCase(), operation: "uppercase" });
});

app.post("/api/transform/hash", async (c) => {
  const { input } = await c.req.json<{ input: string }>();
  const buf = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(input));
  const hex = Array.from(new Uint8Array(buf)).map((b) => b.toString(16).padStart(2, "0")).join("");
  return c.json({ output: `0x${hex}`, operation: "hash" });
});

export default app; // Works with Bun, Node, Deno, Cloudflare Workers, etc.
```

Any request to `/api/transform/*` without a valid `PAYMENT-SIGNATURE` header will receive a `402` response with a `PAYMENT-REQUIRED` header.

---

## Part 2: Building a Client

This section shows how to build a JS client that discovers pricing, pays on-chain, and accesses protected endpoints.

### Dependencies

```json
{
  "viem": "^2.21.0"
}
```

### Step 1: Setup

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
```

### Step 2: Evolve Helpers

Evolve uses its own calldata encoding for token transfers. You need these helper functions (see [Evolve Reference](#evolve-reference) for details):

```typescript
function addressToAccountId(address: `0x${string}`): bigint {
  return BigInt(`0x${address.slice(10)}`);
}

function accountIdToAddress(id: bigint): `0x${string}` {
  const idBytes = new Uint8Array(16);
  let v = id;
  for (let i = 15; i >= 0; i--) {
    idBytes[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  const addrBytes = new Uint8Array(20);
  addrBytes.set(idBytes, 4);
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
  const selector = keccak256(toBytes("transfer")).slice(0, 10);
  const args = new Uint8Array(32);
  args.set(u128ToLeBytes(toAccountId), 0);
  args.set(u128ToLeBytes(amount), 16);
  const data = new Uint8Array(4 + args.length);
  data.set(Buffer.from(selector.slice(2), "hex"), 0);
  data.set(args, 4);
  return bytesToHex(data) as `0x${string}`;
}
```

### Step 3: Submit Payment

```typescript
const TOKEN_ACCOUNT_CANDIDATES = [65535n, 65537n];

async function submitPayment(payTo: `0x${string}`, amount: bigint): Promise<`0x${string}`> {
  const recipientAccountId = addressToAccountId(payTo);

  for (const tokenAccountId of TOKEN_ACCOUNT_CANDIDATES) {
    const tokenAddress = accountIdToAddress(tokenAccountId);
    const data = buildTransferData(recipientAccountId, amount);

    try {
      const txHash = await walletClient.sendTransaction({
        to: tokenAddress,
        data,
        value: 0n,
        gas: 100_000n,
        maxFeePerGas: 1_000_000_000n,
        maxPriorityFeePerGas: 1_000_000_000n,
      });

      const receipt = await publicClient.waitForTransactionReceipt({ hash: txHash });
      if (receipt.status === "success") return txHash;
    } catch {
      // Try next token account candidate
    }
  }
  throw new Error("Failed to submit payment");
}
```

The token contract AccountId varies by genesis (commonly `65535` or `65537`). The client tries both candidates until one succeeds.

### Step 4: Complete Payment Flow

This function handles the full cycle: request -> 402 -> pay -> retry.

```typescript
async function callPaidEndpoint(
  url: string,
  method: string,
  body: unknown,
): Promise<Response> {
  // 1. Initial request
  const initialResponse = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (initialResponse.status !== 402) {
    return initialResponse;
  }

  // 2. Parse payment requirement
  const paymentHeader = initialResponse.headers.get("PAYMENT-REQUIRED");
  if (!paymentHeader) throw new Error("402 without PAYMENT-REQUIRED header");

  const paymentRequired = JSON.parse(
    Buffer.from(paymentHeader, "base64").toString("utf-8")
  );

  const requirement = paymentRequired.accepts[0];

  // 3. Pay on-chain
  const txHash = await submitPayment(
    requirement.payTo as `0x${string}`,
    BigInt(requirement.amount)
  );

  // 4. Build v2 payment proof
  const paymentPayload = {
    x402Version: 2,
    resource: paymentRequired.resource,
    accepted: requirement,
    payload: { txHash },
  };

  const paymentSignature = Buffer.from(
    JSON.stringify(paymentPayload)
  ).toString("base64");

  // 5. Retry with proof
  return fetch(url, {
    method,
    headers: {
      "Content-Type": "application/json",
      "PAYMENT-SIGNATURE": paymentSignature,
    },
    body: JSON.stringify(body),
  });
}
```

### Usage

```typescript
// Discover pricing
const pricing = await fetch("http://localhost:3000/api/pricing").then(r => r.json());
console.log(pricing.endpoints);
// [{ route: "POST /api/transform/echo", price: "100", description: "..." }, ...]

// Make a paid request
const response = await callPaidEndpoint(
  "http://localhost:3000/api/transform/reverse",
  "POST",
  { input: "hello world" }
);

const result = await response.json();
// { "output": "dlrow olleh", "operation": "reverse" }
```

---

## Evolve Reference

### Address Mapping

Evolve uses 128-bit `AccountId` internally. Ethereum addresses (20 bytes) embed the AccountId in the last 16 bytes:

```
Ethereum address: 0x 0000 0000 3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
                       ^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                       padding   AccountId (16 bytes = 128 bits)
```

- `addressToAccountId(address)`: strips the `0x` prefix and 4-byte padding, returns the remaining 16 bytes as a bigint.
- `accountIdToAddress(id)`: writes the bigint as 16 big-endian bytes, prepends 4 zero bytes, returns as `0x`-prefixed hex.

### Calldata Encoding

Evolve uses its own ABI convention, different from Solidity:

```
[4 bytes]  Function selector = keccak256("transfer")[0..4] = 0xb483afd3
[16 bytes] Recipient AccountId (little-endian u128)
[16 bytes] Amount (little-endian u128)
```

Total: **36 bytes** of calldata.

The selector `0xb483afd3` comes from hashing just the string `"transfer"`. Solidity would hash `"transfer(address,uint256)"` producing `0xa9059cbb` — these are different.

### Token Transfers (not native value)

Evolve's execution layer ignores the `value` field on transactions. Payments are made via **token contract calldata**:

```
Transaction:
  to:    <token contract address>   (derived from AccountId 65535 or 65537)
  value: 0
  data:  <transfer calldata>        (36 bytes, see above)
```

### Chain Configuration

| Parameter | Default | How to Discover |
|-----------|---------|-----------------|
| Chain ID | 1337 | `eth_chainId` RPC call |
| Token AccountId | 65535 or 65537 | Check genesis or try both |
| RPC URL | `http://localhost:8545` | Server config |
| Gas limit per tx | 100,000 | Sufficient for token transfers |
| Network identifier | `evolve:1337` | `evolve:{chainId}` |

### Nonce Management

Under load, `eth_getTransactionCount` may lag. For high-throughput clients:

1. Fetch nonce once at startup via `eth_getTransactionCount` with `blockTag: "pending"`
2. Increment locally for each subsequent transaction
3. If you get a "nonce too high" error, re-fetch from the node

---

## Protocol Reference (v2)

### Headers

| Header | Direction | Encoding | Description |
|--------|-----------|----------|-------------|
| `PAYMENT-REQUIRED` | Server -> Client | Base64 JSON | Payment requirements |
| `PAYMENT-SIGNATURE` | Client -> Server | Base64 JSON | Payment proof |
| `PAYMENT-RESPONSE` | Server -> Client | Base64 JSON | Settlement confirmation |
| `X-Agent-ID` | Client -> Server | Plain text | Optional: identify the paying agent |

### PaymentRequired (Server -> Client)

```typescript
{
  x402Version: 2,
  error: "payment_required",
  resource: {
    url: string,         // Requested URL
    description: string, // Human-readable description
    mimeType: string,    // e.g. "application/json"
  },
  accepts: [{
    scheme: "exact",
    network: string,   // e.g. "evolve:1337"
    asset: string,     // e.g. "native"
    amount: string,    // Price in token units
    payTo: string,     // Treasury Ethereum address
    maxTimeoutSeconds: number,
    extra?: Record<string, unknown>,
  }],
}
```

### PaymentPayload (Client -> Server)

```typescript
{
  x402Version: 2,
  resource: {
    url: string,         // Echo from PaymentRequired
    description: string,
    mimeType: string,
  },
  accepted: {            // Echo back the chosen option from accepts[0]
    scheme: "exact",
    network: string,
    asset: string,
    amount: string,
    payTo: string,
    maxTimeoutSeconds: number,
    extra?: Record<string, unknown>,
  },
  payload: {
    txHash: string,      // On-chain transaction hash (0x-prefixed)
  },
}
```

### PaymentSettled (Server -> Client)

Returned in the `PAYMENT-RESPONSE` header on successful payment:

```typescript
{
  x402Version: 2,
  success: boolean,
  transaction?: string,  // Confirmed txHash
  network: string,
  payer?: string,
  error?: string,
}
```

---

## Error Handling

| HTTP Status | Meaning | Action |
|-------------|---------|--------|
| 402 (no PAYMENT-SIGNATURE) | Payment required | Parse PAYMENT-REQUIRED, pay, retry |
| 402 (with PAYMENT-SIGNATURE) | Verification failed | Check tx status, amount, or replay |
| 400 | Invalid payment header | Fix base64 encoding or payload format |
| 503 | Evolve node unavailable | Retry later |
| 200 | Success | Parse result from response body |

Common verification errors:

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

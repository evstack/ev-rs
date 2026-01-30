---
description: Onboard onto the x402-demo project. Use when developers ask about the x402 demo, need to understand its architecture, or want to contribute to it.
---

# X402 Demo Onboarding

## Overview

This demo showcases pay-per-request API monetization using the [X402 protocol](https://x402.org) (HTTP 402 Payment Required) with the Evolve SDK.

## Architecture

```
Frontend (React + Vite)     →  API Server (Bun + Hono)  →  Evolve Node
localhost:5173                  localhost:3000              localhost:8545
```

### Key Flow

1. User registers with passkey → server creates secp256k1 keypair
2. User requests protected API → server returns 402 with payment requirements
3. User pays via `/wallet/transfer` → server signs tx with user's key
4. User retries with `PAYMENT-SIGNATURE` header containing txHash
5. Server verifies tx on Evolve node → returns result

## Directory Structure

```
examples/x402-demo/
├── server/                 # Bun + Hono backend
│   └── src/
│       ├── index.ts        # Entry point, route mounting
│       ├── evolve.ts       # Viem-based JSON-RPC client
│       ├── x402.ts         # X402 middleware (402 responses, payment verification)
│       ├── passkey.ts      # WebAuthn registration/auth, session management
│       ├── wallet.ts       # Balance queries, transfers, faucet
│       └── transform.ts    # Protected API endpoints
├── frontend/               # React + Vite
│   └── src/
│       ├── App.tsx         # Router + nav
│       ├── lib/api.ts      # API client with X402 error handling
│       ├── hooks/usePasskey.ts  # WebAuthn + session state
│       └── pages/
│           ├── Landing.tsx     # Register/login
│           ├── Wallet.tsx      # Balance + faucet
│           └── Playground.tsx  # X402 flow visualization
├── docker-compose.yml
└── README.md
```

## Key Files to Understand

1. **`server/src/x402.ts`** - Core X402 protocol implementation
   - `x402Middleware()` - Hono middleware that returns 402 or verifies payment
   - `PaymentRequired`, `PaymentPayload` types match X402 v2 spec
   - Replay protection via txHash cache

2. **`server/src/passkey.ts`** - Authentication
   - Uses `@simplewebauthn/server` for WebAuthn
   - Maps passkey credentials to server-managed secp256k1 keypairs
   - Session tokens stored in memory (use Redis for production)

3. **`server/src/evolve.ts`** - Blockchain client
   - Viem client configured for Evolve chain
   - Standard eth_* methods + custom evolve_* methods

4. **`frontend/src/pages/Playground.tsx`** - X402 flow demo
   - Shows step-by-step: request → 402 → pay → retry → success

## Running the Demo

```bash
# Terminal 1: Evolve node
cargo run -p evolve_testapp

# Terminal 2: Server
cd examples/x402-demo/server && bun run dev

# Terminal 3: Frontend
cd examples/x402-demo/frontend && bun run dev
```

## Tests

```bash
cd examples/x402-demo/server && bun test
```

Key test files:
- `test/x402.test.ts` - Payment flow, replay protection
- `test/auth.test.ts` - WebAuthn options, session validation

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `EVOLVE_RPC_URL` | `http://127.0.0.1:8545` | Evolve node endpoint |
| `TREASURY_ADDRESS` | `0x...0001` | Receives payments |
| `FAUCET_PRIVATE_KEY` | - | Funded account for faucet |
| `RP_ID` | `localhost` | WebAuthn relying party |
| `RP_ORIGIN` | `http://localhost:5173` | WebAuthn origin |

## Known Limitations

1. **Transaction format** - Currently uses viem's `sendTransaction` which produces standard Ethereum transactions. May need adaptation if Evolve uses a different tx format.

2. **In-memory storage** - Sessions, users, and txHash cache are in memory. Use Redis/SQLite for production.

3. **No full payment verification** - Currently only checks tx exists and succeeded. Should verify: amount, recipient, sender matches user, tx recency.

## Common Tasks

### Add a new protected endpoint

1. Add route config in `server/src/transform.ts`:
   ```typescript
   export const TRANSFORM_ROUTES = {
     "POST /api/transform/new": { price: 150n, description: "New endpoint" },
   };
   ```

2. Add handler in same file:
   ```typescript
   app.post("/new", (c) => {
     const input = c.get("transformInput");
     return c.json({ output: process(input) });
   });
   ```

### Modify payment verification

Edit `verifyPayment()` in `server/src/x402.ts` to add checks for amount, recipient, or timing.

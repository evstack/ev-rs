# X402 Payments Demo

## Overview

Demo showcasing Evolve SDK through pay-per-request API monetization using the [Coinbase X402 protocol](https://x402.org).

X402 uses HTTP 402 (Payment Required) for machine-readable payment flows: client requests resource → server returns 402 with payment details → client pays → client retries with proof → server delivers resource.

## Decisions

| Aspect           | Decision                                      |
|------------------|-----------------------------------------------|
| Use Case         | API Monetization (pay-per-request)            |
| X402 Libraries   | `@x402/core` types + custom Evolve middleware |
| Frontend         | React + Vite + `@x402/fetch`                  |
| Wallet           | Passkey/WebAuthn (server-side signing)        |
| Token            | Native Evolve Token                           |
| Backend          | Bun + Hono                                    |
| Deployment       | Fly.io (node + server) + Vercel (frontend)    |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React)                          │
│  - @x402/fetch handles 402 → pay → retry automatically      │
│  - @simplewebauthn/browser for passkey auth                 │
└─────────────────────────────┬───────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────┐
│                    Bun + Hono Server                         │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Passkey Auth (@simplewebauthn/server)                  │ │
│  │ - WebAuthn registration/authentication                 │ │
│  │ - Maps credentialId → secp256k1 keypair                │ │
│  └────────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ X402 Middleware (custom, uses @x402/core types)        │ │
│  │ - Returns 402 with PaymentRequired per X402 spec       │ │
│  │ - Verifies payment via Evolve JSON-RPC                 │ │
│  └────────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Transform API                                          │ │
│  │ - /echo, /reverse, /uppercase, /hash                   │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────┬───────────────────────────────┘
                              │ JSON-RPC
┌─────────────────────────────▼───────────────────────────────┐
│                      Evolve Node                             │
│  - Token balances and transfers                              │
│  - Transaction processing                                    │
└─────────────────────────────────────────────────────────────┘
```

### Why Custom Middleware?

The Coinbase X402 libraries assume a facilitator (Coinbase/Cloudflare) for payment settlement. We verify directly against Evolve node, so we:
- Use `@x402/core` for protocol types (`PaymentRequired`, `PaymentPayload`)
- Write custom Hono middleware for Evolve-specific verification
- Use `@x402/fetch` on frontend for automatic 402 handling

---

## X402 Flow

```
1. GET /api/transform/reverse

2. Server returns 402 with X402-compliant headers:
   HTTP/1.1 402 Payment Required
   X-Payment: <base64-encoded PaymentRequired>

3. @x402/fetch intercepts, calls payment callback

4. Client pays via /wallet/transfer → gets txHash

5. @x402/fetch retries with payment proof:
   GET /api/transform/reverse
   X-Payment-Response: <base64-encoded PaymentPayload>

6. Server verifies on Evolve node, returns resource
```

---

## Directory Structure

```
examples/x402-demo/
├── docker-compose.yml
├── server/
│   ├── package.json
│   └── src/
│       ├── index.ts
│       ├── x402.ts           # Custom middleware + verification
│       ├── passkey.ts        # WebAuthn registration/auth
│       ├── wallet.ts         # Balance, transfer, faucet
│       ├── transform.ts      # Protected API endpoints
│       └── evolve.ts         # JSON-RPC client
└── frontend/
    ├── package.json
    └── src/
        ├── App.tsx
        ├── pages/
        │   ├── Landing.tsx
        │   ├── Wallet.tsx
        │   └── Playground.tsx
        ├── hooks/
        │   ├── usePasskey.ts
        │   └── useX402Client.ts  # Wraps @x402/fetch
        └── lib/
            └── api.ts
```

---

## API Endpoints

### Auth
```
POST /auth/register      → WebAuthn registration options
POST /auth/register/verify → Complete registration, return address
POST /auth/login         → WebAuthn auth options
POST /auth/login/verify  → Complete auth, return session token
```

### Wallet
```
GET  /wallet/balance     → { address, balance }
POST /wallet/faucet      → { txHash, newBalance }
POST /wallet/transfer    → { txHash }
```

### Transform (X402 Protected)
```
POST /api/transform/echo      → 100 tokens
POST /api/transform/reverse   → 100 tokens
POST /api/transform/uppercase → 100 tokens
POST /api/transform/hash      → 200 tokens
```

---

## Dependencies

### Server
```json
{
  "dependencies": {
    "hono": "^4.0.0",
    "@x402/core": "latest",
    "@simplewebauthn/server": "^9.0.0",
    "viem": "^2.0.0"
  }
}
```

### Frontend
```json
{
  "dependencies": {
    "react": "^18.0.0",
    "react-router-dom": "^6.0.0",
    "@x402/fetch": "latest",
    "@simplewebauthn/browser": "^9.0.0",
    "@tanstack/react-query": "^5.0.0"
  }
}
```

---

## Implementation Phases

| Phase | Scope |
|-------|-------|
| 1     | Bun/Hono skeleton, Evolve JSON-RPC client, balance query |
| 2     | X402 middleware using @x402/core types, Transform API |
| 3     | Passkey auth with @simplewebauthn |
| 4     | React frontend with @x402/fetch integration |
| 5     | Docker compose + Fly.io deployment |

---

## Security Notes

- Passkey proves user identity (phishing-resistant, device-bound)
- Server holds signing keys (custodial, protected by passkey)
- Replay protection: cache used txHashes, enforce nonce ordering
- Future: P-256 on-chain verification enables client-side signing

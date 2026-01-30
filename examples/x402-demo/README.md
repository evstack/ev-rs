# X402 Demo

Pay-per-request API monetization using [HTTP 402](https://x402.org) and the Evolve SDK.

## Quick Start

### Local Development (without Docker)

```bash
# Terminal 1: Start Evolve node
cargo run -p evolve_testapp

# Terminal 2: Start API server
cd server && bun install && bun run dev

# Terminal 3: Start frontend
cd frontend && bun install && bun run dev
```

Open http://localhost:5173

### Docker

```bash
# Start Evolve node on host
cargo run -p evolve_testapp

# Start server + frontend in Docker
docker compose up --build
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React)                          │
│  http://localhost:5173                                       │
└─────────────────────────────┬───────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────┐
│                    API Server (Bun + Hono)                   │
│  http://localhost:3000                                       │
│  - /auth/* (passkey registration/login)                      │
│  - /wallet/* (balance, faucet, transfer)                     │
│  - /api/transform/* (X402 protected endpoints)               │
└─────────────────────────────┬───────────────────────────────┘
                              │ JSON-RPC
┌─────────────────────────────▼───────────────────────────────┐
│                    Evolve Node                               │
│  http://localhost:8545                                       │
└─────────────────────────────────────────────────────────────┘
```

## X402 Flow

1. Client requests `/api/transform/reverse` without payment
2. Server returns `402 Payment Required` with `PAYMENT-REQUIRED` header
3. Client pays treasury via `/wallet/transfer`
4. Client retries with `PAYMENT-SIGNATURE` header containing txHash
5. Server verifies payment on-chain, returns result

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `EVOLVE_RPC_URL` | `http://127.0.0.1:8545` | Evolve node RPC endpoint |
| `TREASURY_ADDRESS` | `0x...0001` | Address receiving payments |
| `FAUCET_PRIVATE_KEY` | - | Private key for faucet transfers |
| `RP_ID` | `localhost` | WebAuthn relying party ID |
| `RP_ORIGIN` | `http://localhost:5173` | WebAuthn expected origin |

## API Pricing

| Endpoint | Price | Description |
|----------|-------|-------------|
| `/api/transform/echo` | 100 | Returns input unchanged |
| `/api/transform/reverse` | 100 | Reverses input string |
| `/api/transform/uppercase` | 100 | Uppercases input |
| `/api/transform/hash` | 200 | SHA256 hash of input |

# X402 Demo

Pay-per-request API monetization using [HTTP 402](https://x402.org) and the Evolve SDK.

## Architecture

```
                          ┌──────────────────────┐
                          │   Frontend (React)    │
                          │  http://localhost:5173 │
                          └──────────┬───────────┘
                                     │ HTTP + WebSocket
                          ┌──────────▼───────────┐
                          │  API Server (Bun+Hono)│
                          │  http://localhost:3000 │
                          │  - /api/transform/*   │
                          │  - /ws/events         │
                          └──────────┬───────────┘
                                     │ JSON-RPC
┌───────────────┐  gRPC   ┌──────────▼───────────┐
│  ev-node /    │◄───────►│    evd (Evolve node)  │
│  local-da     │ :50051  │  http://localhost:8545 │
└───────────────┘         └──────────────────────┘
```

**Components:**

| Component | Port | Description |
|-----------|------|-------------|
| **evd** | 8545 (RPC), 50051 (gRPC) | Evolve execution node |
| **ev-node / local-da** | - | External consensus driving block production via gRPC |
| **server** | 3000 | X402 API server with payment middleware |
| **simulator** | - | CLI tool that spawns agents making paid requests |
| **frontend** | 5173 | React dashboard showing real-time metrics |

## Prerequisites

- Rust toolchain (for evd)
- [Bun](https://bun.sh) (for server + simulator)
- Node.js + npm (for frontend)
- External consensus process (ev-node or local-da)

## Quick Start

You need **4 terminals** (5 if you want the dashboard).

### Terminal 1: Start evd

```bash
# From repo root
rm -rf ./data
cargo run -p evd -- init --genesis-file examples/x402-demo/genesis.json
cargo run -p evd -- run  --genesis-file examples/x402-demo/genesis.json
```

Wait for `Server ready. Press Ctrl+C to stop.` The genesis log will show all pre-registered EOA accounts with their addresses and balances.

### Terminal 2: External consensus

Start whatever drives block production via gRPC on `:50051`. Without this, transactions will be accepted into the mempool but never mined.

### Terminal 3: API server

```bash
cd examples/x402-demo/server
bun install   # first time only
bun run src/index.ts
```

Verify: `curl http://localhost:3000/health`

### Terminal 4: Simulator

```bash
cd examples/x402-demo/simulator
bun install   # first time only
FAUCET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  bun run src/index.ts --agents 5 --rps 2 --duration 60
```

### Terminal 5 (optional): Frontend dashboard

```bash
cd examples/x402-demo/frontend
npm install   # first time only
npm run dev
```

Open http://localhost:5173

## Account Setup

The sample `examples/x402-demo/genesis.json` pre-registers **20 Hardhat accounts** with well-known deterministic keys:

| Account | Role | Address | Private Key |
|---------|------|---------|-------------|
| #0 (Alice) | Faucet | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974...f2ff80` |
| #1 (Bob) | - | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c699...8690d` |
| #2 | Treasury | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` | `0x5de411...b365a` |
| #3-#19 | Agents (17) | See genesis log | See genesis log |

**Token balances at genesis:**
- Alice: 1,000,000,000 (= 1000 tokens, 6 decimals)
- Bob: 2,000,000 (= 2 tokens, 6 decimals)

## X402 Payment Flow

1. Agent sends `POST /api/transform/reverse` with `{"input": "hello"}`
2. Server returns **402** with `PAYMENT-REQUIRED` header (base64-encoded JSON)
3. Agent parses the required amount and treasury address
4. Agent sends a **token transfer** transaction on-chain (calldata to the token contract)
5. Agent retries the request with `PAYMENT-SIGNATURE` header containing the txHash
6. Server verifies the payment on-chain, returns the result

Payments use **token calldata** (not native value transfers), because the Evolve execution layer ignores the `value` field on transactions.

## Simulator Options

```
Options:
  -a, --agents <number>    Number of agents to spawn (max 17)    [default: 10]
  -r, --rps <number>       Target requests per second             [default: 10]
  -s, --server <url>       X402 server URL                        [default: http://localhost:3000]
  -e, --evolve-rpc <url>   Evolve RPC URL                         [default: http://localhost:8545]
  -f, --funding <amount>   Funding per agent (6 decimals)         [default: 1000000 = 1 token]
  -k, --faucet-key <key>   Faucet private key                     [env: FAUCET_PRIVATE_KEY]
  -d, --duration <seconds> Run duration (0 = infinite)            [default: 0]
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `EVOLVE_RPC_URL` | `http://127.0.0.1:8545` | Evolve node JSON-RPC |
| `EVOLVE_GRPC_URL` | - | Evolve node gRPC (optional, enables gRPC client mode) |
| `TREASURY_ADDRESS` | `0x3C44Cd...93BC` | Hardhat #2, receives payments |
| `EVOLVE_NETWORK` | `evolve:1` | Network identifier for X402 |
| `EVOLVE_ASSET` | `native` | Asset identifier for X402 |
| `FAUCET_PRIVATE_KEY` | - | Alice's key, funds simulator agents |
| `PORT` | `3000` | Server listen port |
| `CORS_ORIGIN` | `*` | Allowed CORS origins |

## API Pricing

| Endpoint | Price (tokens) | Description |
|----------|---------------|-------------|
| `/api/transform/echo` | 100 | Returns input unchanged |
| `/api/transform/reverse` | 100 | Reverses input string |
| `/api/transform/uppercase` | 100 | Uppercases input |
| `/api/transform/hash` | 200 | SHA256 hash of input |

## Verification

```bash
# Check server health
curl http://localhost:3000/health

# Check treasury balance (should increase as simulator runs)
curl http://localhost:3000/api/treasury

# Check pricing
curl http://localhost:3000/api/pricing
```
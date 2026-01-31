# X402 Agent Simulator - Design Document

## Overview

Transform the X402 demo from a user-facing login flow into a multi-agent simulator that demonstrates high-throughput micropayments on the Evolve chain.

## Goals

1. Simulate N agents making paid API requests concurrently
2. Visualize agent activity and payment flow in real-time
3. Demonstrate sub-100ms payment latency with fast block times
4. Showcase the X402 protocol at scale

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Agent Simulator                             │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Agent Pool                                                │  │
│  │  - N agents with pre-funded wallets                       │  │
│  │  - Private keys in memory                                 │  │
│  │  - Timestamp-based signing (no nonce tracking)            │  │
│  │  - Configurable request rate per agent                    │  │
│  └───────────────────────────────────────────────────────────┘  │
└──────────────────────┬──────────────────────────────────────────┘
                       │ HTTP/2
┌──────────────────────▼──────────────────────────────────────────┐
│                      X402 Server                                │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Payment Verification                                      │  │
│  │  - gRPC client to Evolve node                             │  │
│  │  - SubmitAndWait for synchronous confirmation             │  │
│  │  - Replay protection via chain state                      │  │
│  └───────────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Event Emitter                                             │  │
│  │  - WebSocket server for dashboard                         │  │
│  │  - Events: payment_submitted, confirmed, request_served   │  │
│  └───────────────────────────────────────────────────────────┘  │
└──────────────────────┬──────────────────────────────────────────┘
                       │ gRPC
┌──────────────────────▼──────────────────────────────────────────┐
│                      Evolve Node                                │
│  - Configurable block time (1ms, 10ms, 100ms)                  │
│  - Timestamp-based nonceless transactions                       │
│  - gRPC interface for low-latency communication                 │
└─────────────────────────────────────────────────────────────────┘
                       │ WebSocket
┌──────────────────────▼──────────────────────────────────────────┐
│                      Dashboard                                  │
│  - Real-time agent status grid                                  │
│  - Payment stream visualization                                 │
│  - Metrics: TPS, latency histogram, success rate                │
│  - Treasury balance tracker                                     │
└─────────────────────────────────────────────────────────────────┘
```

## Components to Build

### 1. Agent Simulator (`/simulator`)

New directory containing the agent simulation logic.

#### Files

```
simulator/
├── src/
│   ├── index.ts          # Entry point, CLI
│   ├── agent.ts          # Single agent logic
│   ├── pool.ts           # Agent pool management
│   ├── signer.ts         # Timestamp-based signing
│   ├── metrics.ts        # Latency tracking, stats
│   └── config.ts         # Configuration schema
├── package.json
└── tsconfig.json
```

#### Agent Class

```typescript
interface AgentConfig {
  id: string;
  privateKey: Hex;
  address: Address;
  requestsPerSecond: number;
  endpoints: WeightedEndpoint[];
}

interface WeightedEndpoint {
  method: string;
  path: string;
  weight: number;  // probability of selection
  payload: () => unknown;  // payload generator
}

class Agent {
  private config: AgentConfig;
  private running: boolean = false;
  private metrics: AgentMetrics;

  async start(): Promise<void>;
  async stop(): Promise<void>;

  private async makeRequest(): Promise<RequestResult> {
    const endpoint = this.selectEndpoint();

    // Step 1: Make request, expect 402
    const initialResponse = await fetch(endpoint.url, {
      method: endpoint.method,
      body: JSON.stringify(endpoint.payload()),
    });

    if (initialResponse.status !== 402) {
      // Unexpected - either error or no payment required
      return this.handleNonPaymentResponse(initialResponse);
    }

    // Step 2: Parse payment requirement
    const paymentRequired = this.parsePaymentRequired(initialResponse);

    // Step 3: Sign and submit payment
    const signedTx = this.signPayment(paymentRequired);
    const txHash = await this.submitPayment(signedTx);

    // Step 4: Retry with payment proof
    const paymentSignature = this.createPaymentSignature(txHash);
    const finalResponse = await fetch(endpoint.url, {
      method: endpoint.method,
      body: JSON.stringify(endpoint.payload()),
      headers: {
        'PAYMENT-SIGNATURE': paymentSignature,
      },
    });

    return {
      success: finalResponse.ok,
      latencyMs: /* measured */,
      txHash,
    };
  }
}
```

#### Pool Manager

```typescript
interface PoolConfig {
  agentCount: number;
  fundingAmount: bigint;
  serverUrl: string;
  evolveRpcUrl: string;
  requestsPerSecond: number;  // total across all agents
}

class AgentPool {
  private agents: Agent[] = [];
  private faucetKey: Hex;

  async initialize(config: PoolConfig): Promise<void> {
    // Generate N wallets
    // Fund each from faucet
    // Create Agent instances
  }

  async start(): Promise<void> {
    // Start all agents
  }

  async stop(): Promise<void> {
    // Graceful shutdown
  }

  getMetrics(): PoolMetrics {
    // Aggregate metrics from all agents
  }
}
```

#### CLI Interface

```bash
# Start simulator with 10 agents, 100 req/s total
bun run simulator --agents 10 --rps 100 --server http://localhost:3000

# With custom funding
bun run simulator --agents 50 --rps 500 --funding 10000

# Specify block time expectation (for latency calculations)
bun run simulator --agents 10 --rps 100 --block-time 10
```

### 2. Server Changes (`/server`)

#### New: gRPC Client (`/server/src/grpc-client.ts`)

Replace viem JSON-RPC with gRPC for Evolve communication.

```typescript
import { createClient } from '@grpc/grpc-js';
import { EvolvePaymentClient } from './generated/evolve_pb';

interface EvolveGrpcClient {
  submitAndWait(signedTx: SignedTx): Promise<TxReceipt>;
  submit(signedTx: SignedTx): Promise<TxHash>;
  getBalance(address: Address): Promise<bigint>;
}

export function createEvolveGrpcClient(endpoint: string): EvolveGrpcClient {
  const client = new EvolvePaymentClient(
    endpoint,
    grpc.credentials.createInsecure()
  );

  return {
    async submitAndWait(tx) {
      return new Promise((resolve, reject) => {
        client.submitAndWait(tx, (err, response) => {
          if (err) reject(err);
          else resolve(response);
        });
      });
    },
    // ... other methods
  };
}
```

#### New: WebSocket Event Server (`/server/src/events.ts`)

```typescript
import { Hono } from 'hono';
import { createBunWebSocket } from 'hono/bun';

interface PaymentEvent {
  type: 'payment_submitted' | 'payment_confirmed' | 'request_served' | 'error';
  timestamp: number;
  agentId: string;
  txHash?: string;
  amount?: string;
  endpoint?: string;
  latencyMs?: number;
  error?: string;
}

class EventEmitter {
  private connections: Set<WebSocket> = new Set();

  addConnection(ws: WebSocket): void {
    this.connections.add(ws);
    ws.onclose = () => this.connections.delete(ws);
  }

  emit(event: PaymentEvent): void {
    const message = JSON.stringify(event);
    for (const ws of this.connections) {
      ws.send(message);
    }
  }
}

export const eventEmitter = new EventEmitter();

export function createEventRoutes(): Hono {
  const app = new Hono();
  const { upgradeWebSocket, websocket } = createBunWebSocket();

  app.get('/events', upgradeWebSocket((c) => ({
    onOpen(_, ws) {
      eventEmitter.addConnection(ws);
    },
  })));

  return app;
}
```

#### Modified: X402 Middleware (`/server/src/x402.ts`)

Update to use gRPC and emit events.

```typescript
import { eventEmitter } from './events';
import { evolveGrpcClient } from './grpc-client';

export function x402Middleware(config: X402Config): MiddlewareHandler {
  return async (c, next) => {
    // ... existing route matching logic ...

    const paymentSignature = c.req.header('PAYMENT-SIGNATURE');

    if (!paymentSignature) {
      // Return 402 as before
      return c.json(paymentRequired, 402);
    }

    // Parse and verify payment
    const payload = decodePaymentPayload(paymentSignature);
    const agentId = extractAgentId(c);  // From request or derived from tx

    eventEmitter.emit({
      type: 'payment_submitted',
      timestamp: Date.now(),
      agentId,
      txHash: payload.payload.txHash,
      amount: routeConfig.amount,
    });

    // Verify via gRPC (faster than JSON-RPC)
    const receipt = await evolveGrpcClient.getReceipt(payload.payload.txHash);

    if (!receipt.success) {
      eventEmitter.emit({
        type: 'error',
        timestamp: Date.now(),
        agentId,
        txHash: payload.payload.txHash,
        error: 'Transaction failed',
      });
      return c.json({ error: 'Payment verification failed' }, 402);
    }

    eventEmitter.emit({
      type: 'payment_confirmed',
      timestamp: Date.now(),
      agentId,
      txHash: payload.payload.txHash,
    });

    // Continue to handler
    await next();

    eventEmitter.emit({
      type: 'request_served',
      timestamp: Date.now(),
      agentId,
      endpoint: `${c.req.method} ${c.req.path}`,
    });
  };
}
```

### 3. Dashboard (`/frontend`)

Replace current user-facing UI with agent monitoring dashboard.

#### New Pages

```
frontend/src/
├── pages/
│   ├── Dashboard.tsx      # Main dashboard view
│   ├── AgentGrid.tsx      # Grid of agent status cards
│   ├── PaymentStream.tsx  # Real-time payment log
│   └── Metrics.tsx        # Charts and statistics
├── components/
│   ├── AgentCard.tsx      # Individual agent status
│   ├── LatencyChart.tsx   # Histogram of latencies
│   ├── TpsCounter.tsx     # Live TPS display
│   └── TreasuryBalance.tsx
└── hooks/
    ├── useEventStream.ts  # WebSocket connection
    └── useMetrics.ts      # Aggregated metrics
```

#### Dashboard Layout

```typescript
// Dashboard.tsx
export function Dashboard() {
  const events = useEventStream('ws://localhost:3000/events');
  const metrics = useMetrics(events);

  return (
    <div className="dashboard">
      <header>
        <h1>X402 Agent Simulator</h1>
        <TpsCounter value={metrics.currentTps} />
        <TreasuryBalance />
      </header>

      <main>
        <section className="agent-grid">
          <AgentGrid agents={metrics.agents} />
        </section>

        <section className="metrics">
          <LatencyChart data={metrics.latencyHistogram} />
          <SuccessRateGauge value={metrics.successRate} />
        </section>

        <section className="payment-stream">
          <PaymentStream events={events} />
        </section>
      </main>
    </div>
  );
}
```

#### Event Stream Hook

```typescript
// useEventStream.ts
export function useEventStream(url: string) {
  const [events, setEvents] = useState<PaymentEvent[]>([]);

  useEffect(() => {
    const ws = new WebSocket(url);

    ws.onmessage = (e) => {
      const event = JSON.parse(e.data) as PaymentEvent;
      setEvents((prev) => [...prev.slice(-999), event]);  // Keep last 1000
    };

    return () => ws.close();
  }, [url]);

  return events;
}
```

#### Agent Card Component

```typescript
// AgentCard.tsx
interface AgentCardProps {
  agent: AgentStatus;
}

export function AgentCard({ agent }: AgentCardProps) {
  return (
    <div className={`agent-card ${agent.status}`}>
      <div className="agent-id">{agent.id.slice(0, 8)}</div>
      <div className="agent-address">{agent.address.slice(0, 10)}...</div>
      <div className="agent-stats">
        <span>Requests: {agent.totalRequests}</span>
        <span>Avg Latency: {agent.avgLatencyMs}ms</span>
        <span>Balance: {formatEvo(agent.balance)}</span>
      </div>
      <div className={`status-indicator ${agent.lastStatus}`} />
    </div>
  );
}
```

### 4. Remove/Deprecate

The following components from the current implementation should be removed:

- `/frontend/src/pages/Landing.tsx` - User login flow
- `/frontend/src/pages/Wallet.tsx` - User wallet management
- `/frontend/src/pages/Playground.tsx` - Manual API testing
- `/frontend/src/hooks/usePasskey.ts` - WebAuthn hooks
- `/server/src/passkey.ts` - WebAuthn authentication
- User session management in `/server/src/wallet.ts`

Keep the wallet routes for agent funding (faucet) but remove user-specific logic.

## Configuration

### Environment Variables

```bash
# Server
EVOLVE_GRPC_URL=localhost:9090    # gRPC endpoint
EVOLVE_RPC_URL=http://localhost:8545  # Fallback JSON-RPC
TREASURY_ADDRESS=0x0000000000000000000000000000000000000001
FAUCET_PRIVATE_KEY=0x...
WS_PORT=3001                       # WebSocket for dashboard

# Simulator
SIMULATOR_AGENTS=10
SIMULATOR_RPS=100
SIMULATOR_SERVER_URL=http://localhost:3000
SIMULATOR_FUNDING_AMOUNT=10000
```

### Simulator Config File

```yaml
# simulator.config.yaml
agents: 10
requestsPerSecond: 100
server: http://localhost:3000
evolve:
  grpcUrl: localhost:9090
  chainId: 1337

funding:
  amount: 10000
  faucetKey: ${FAUCET_PRIVATE_KEY}

endpoints:
  - method: POST
    path: /api/transform/reverse
    weight: 40
    payload:
      text: "hello world"
  - method: POST
    path: /api/transform/uppercase
    weight: 30
    payload:
      text: "hello world"
  - method: POST
    path: /api/transform/hash
    weight: 20
    payload:
      text: "hello world"
  - method: POST
    path: /api/transform/echo
    weight: 10
    payload:
      text: "hello world"
```

## Metrics

### Collected Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `requests_total` | Counter | Total requests made |
| `requests_success` | Counter | Successful requests |
| `requests_failed` | Counter | Failed requests |
| `payments_total` | Counter | Total payments submitted |
| `payments_confirmed` | Counter | Confirmed payments |
| `request_latency_ms` | Histogram | End-to-end latency |
| `payment_latency_ms` | Histogram | Payment confirmation latency |
| `agent_balance` | Gauge | Per-agent balance |
| `treasury_balance` | Gauge | Treasury balance |
| `tps_current` | Gauge | Current transactions per second |

### Dashboard Displays

1. **TPS Counter**: Large real-time display of current throughput
2. **Latency Histogram**: Distribution of request latencies (p50, p95, p99)
3. **Agent Grid**: Visual grid showing all agents, color-coded by status
4. **Payment Stream**: Scrolling log of recent payments
5. **Success Rate**: Gauge showing percentage of successful requests
6. **Treasury Balance**: Live balance with incoming payment animation

## Implementation Phases

### Phase 1: Core Infrastructure
- [x] Create `/simulator` directory structure
- [x] Implement Agent and AgentPool classes
- [x] Add timestamp-based signing (prepare for nonceless)
- [x] Basic CLI to run simulator

### Phase 2: Server Updates
- [x] Add WebSocket event emitter
- [x] Integrate event emission into X402 middleware
- [x] Prepare gRPC client interface (can stub with JSON-RPC initially) - using JSON-RPC for now
- [x] Add `/events` WebSocket endpoint

### Phase 3: Dashboard
- [x] Remove old user-facing pages (simplified to Dashboard only)
- [x] Implement Dashboard layout
- [x] Add AgentGrid component
- [x] Add PaymentStream component
- [x] Add metrics charts (latency histogram, TPS counter)

### Phase 4: gRPC Integration
- [x] Create gRPC client using existing protos from `crates/rpc/grpc/proto/evolve/v1/`
- [x] Unified EvolveClient interface supporting both JSON-RPC and gRPC backends
- [x] Auto-detection: uses gRPC if `EVOLVE_GRPC_URL` is set, otherwise JSON-RPC
- [ ] Performance testing with different block times

### Phase 5: Optimization
- [ ] Implement nonceless transaction support (depends on Evolve changes)
- [ ] Connection pooling for gRPC
- [ ] Dashboard performance optimization for high event rates

## Testing

### Load Testing Scenarios

```bash
# Baseline: 10 agents, 10 req/s, 100ms blocks
bun run simulator --agents 10 --rps 10 --block-time 100

# Medium: 50 agents, 100 req/s, 10ms blocks
bun run simulator --agents 50 --rps 100 --block-time 10

# High: 100 agents, 1000 req/s, 1ms blocks
bun run simulator --agents 100 --rps 1000 --block-time 1
```

### Expected Latency Targets

| Block Time | Target p50 | Target p99 |
|------------|------------|------------|
| 100ms | <150ms | <300ms |
| 10ms | <30ms | <100ms |
| 1ms | <10ms | <30ms |

## Dependencies

### New Dependencies

```json
{
  "dependencies": {
    "@grpc/grpc-js": "^1.10.0",
    "@grpc/proto-loader": "^0.7.0",
    "recharts": "^2.12.0",
    "yaml": "^2.4.0"
  }
}
```

## Open Questions

1. Should agents have different "personalities" (aggressive vs conservative spending)?
2. Do we want to simulate agent failures/recovery?
3. Should the dashboard support replaying historical data?
4. What's the maximum agent count we want to support in the demo?

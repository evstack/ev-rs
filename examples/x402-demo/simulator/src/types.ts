import type { Address, Hash, Hex } from "viem";

// Agent configuration
export interface AgentConfig {
  id: string;
  privateKey: Hex;
  address: Address;
  requestsPerSecond: number;
  endpoints: WeightedEndpoint[];
  tokenAccountId: bigint;
  chainId: number;
}

// Weighted endpoint for random selection
export interface WeightedEndpoint {
  method: "GET" | "POST" | "PUT" | "DELETE";
  path: string;
  weight: number;
  payload: () => Record<string, unknown>;
}

// Pool configuration
export interface PoolConfig {
  agentCount: number;
  fundingAmount: bigint;
  serverUrl: string;
  evolveRpcUrl: string;
  requestsPerSecond: number;
  faucetPrivateKey: Hex;
  endpoints: WeightedEndpoint[];
}

// Result of a single request
export interface RequestResult {
  success: boolean;
  agentId: string;
  endpoint: string;
  txHash?: Hash;
  latencyMs: number;
  paymentLatencyMs?: number;
  error?: string;
  timestamp: number;
}

// Agent metrics
export interface AgentMetrics {
  agentId: string;
  address: Address;
  totalRequests: number;
  successfulRequests: number;
  failedRequests: number;
  totalLatencyMs: number;
  totalPaymentLatencyMs: number;
  balance: bigint;
  lastRequestTime: number;
}

// Pool-level aggregated metrics
export interface PoolMetrics {
  totalRequests: number;
  successfulRequests: number;
  failedRequests: number;
  avgLatencyMs: number;
  p50LatencyMs: number;
  p95LatencyMs: number;
  p99LatencyMs: number;
  currentTps: number;
  agents: AgentMetrics[];
  startTime: number;
  elapsedMs: number;
}

// X402 Protocol Types (v2 — compatible with @x402/core)
export interface PaymentRequired {
  x402Version: number;
  error?: string;
  resource: {
    url: string;
    description: string;
    mimeType: string;
  };
  accepts: PaymentRequirements[];
}

export interface PaymentRequirements {
  scheme: string;
  network: string;
  asset: string;
  amount: string;
  payTo: string;
  maxTimeoutSeconds: number;
  extra: Record<string, unknown>;
}

export interface PaymentPayload {
  x402Version: number;
  resource: {
    url: string;
    description: string;
    mimeType: string;
  };
  accepted: PaymentRequirements;
  payload: {
    txHash: Hash;
  };
}

// Event types for dashboard
export type SimulatorEventType =
  | "agent_started"
  | "agent_stopped"
  | "request_started"
  | "payment_required"
  | "payment_submitted"
  | "payment_confirmed"
  | "request_completed"
  | "request_failed"
  | "pool_started"
  | "pool_stopped"
  | "metrics_update";

export interface SimulatorEvent {
  type: SimulatorEventType;
  timestamp: number;
  agentId?: string;
  data?: Record<string, unknown>;
}

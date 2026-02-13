import type { Hex } from "viem";
import type { PoolConfig, WeightedEndpoint } from "./types.js";

// Default endpoints for the transform API
export const DEFAULT_ENDPOINTS: WeightedEndpoint[] = [
  {
    method: "POST",
    path: "/api/transform/reverse",
    weight: 40,
    payload: () => ({ input: `hello-${Date.now()}` }),
  },
  {
    method: "POST",
    path: "/api/transform/uppercase",
    weight: 30,
    payload: () => ({ input: `hello-${Date.now()}` }),
  },
  {
    method: "POST",
    path: "/api/transform/hash",
    weight: 20,
    payload: () => ({ input: `hello-${Date.now()}` }),
  },
  {
    method: "POST",
    path: "/api/transform/echo",
    weight: 10,
    payload: () => ({ input: `hello-${Date.now()}` }),
  },
];

export interface CLIOptions {
  agents: number;
  rps: number;
  server: string;
  evolveRpc: string;
  funding: string;
  faucetKey?: string;
  duration?: number;
}

export function createPoolConfig(options: CLIOptions): PoolConfig {
  const faucetKey = options.faucetKey ?? process.env.FAUCET_PRIVATE_KEY;
  if (!faucetKey) {
    throw new Error(
      "Faucet private key required. Set FAUCET_PRIVATE_KEY env var or use --faucet-key"
    );
  }

  return {
    agentCount: options.agents,
    fundingAmount: BigInt(options.funding),
    serverUrl: options.server,
    evolveRpcUrl: options.evolveRpc,
    requestsPerSecond: options.rps,
    faucetPrivateKey: faucetKey as Hex,
    endpoints: DEFAULT_ENDPOINTS,
  };
}

// Validate configuration
export function validateConfig(config: PoolConfig): string[] {
  const errors: string[] = [];

  if (config.agentCount < 1) {
    errors.push("Agent count must be at least 1");
  }
  if (config.agentCount > 5000) {
    errors.push("Agent count exceeds maximum (5000)");
  }
  if (config.requestsPerSecond < 1) {
    errors.push("Requests per second must be at least 1");
  }
  if (config.requestsPerSecond > 10000) {
    errors.push("Requests per second exceeds maximum (10000)");
  }
  if (config.fundingAmount <= 0n) {
    errors.push("Funding amount must be positive");
  }
  if (!config.serverUrl.startsWith("http")) {
    errors.push("Server URL must be a valid HTTP URL");
  }
  if (!config.evolveRpcUrl.startsWith("http")) {
    errors.push("Evolve RPC URL must be a valid HTTP URL");
  }
  if (!config.faucetPrivateKey.startsWith("0x")) {
    errors.push("Faucet private key must be a hex string starting with 0x");
  }

  const totalWeight = config.endpoints.reduce((sum, e) => sum + e.weight, 0);
  if (totalWeight <= 0) {
    errors.push("Endpoint weights must sum to a positive number");
  }

  return errors;
}

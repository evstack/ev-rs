import {
  createPublicClient,
  http,
  defineChain,
  type Address,
  type Hash,
  type PublicClient,
  type HttpTransport,
  type Chain,
} from "viem";
import { createEvolveGrpcClient, type EvolveGrpcClient } from "./grpc-client.js";

// Evolve testnet chain definition
export const evolveTestnet = defineChain({
  id: 1337, // Default dev chain ID - configure via env
  name: "Evolve Testnet",
  nativeCurrency: {
    decimals: 18,
    name: "Evolve Token",
    symbol: "EVO",
  },
  rpcUrls: {
    default: {
      http: [process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545"],
    },
  },
});

// Client mode
export type ClientMode = "jsonrpc" | "grpc";

export type EvolveClient = {
  mode: ClientMode;
  public: PublicClient<HttpTransport, Chain>;
  grpc?: EvolveGrpcClient;
};

/**
 * Creates a client configured for Evolve node
 * Supports both JSON-RPC (viem) and gRPC backends
 */
export async function createEvolveClient(options?: {
  rpcUrl?: string;
  grpcUrl?: string;
  mode?: ClientMode;
}): Promise<EvolveClient> {
  const mode = options?.mode ?? (process.env.EVOLVE_GRPC_URL ? "grpc" : "jsonrpc");
  const rpcUrl = options?.rpcUrl ?? process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545";
  const grpcUrl = options?.grpcUrl ?? process.env.EVOLVE_GRPC_URL ?? "localhost:9545";

  const transport = http(rpcUrl);
  const publicClient = createPublicClient({
    chain: evolveTestnet,
    transport,
  });

  const client: EvolveClient = {
    mode,
    public: publicClient,
  };

  // Initialize gRPC client if using gRPC mode
  if (mode === "grpc") {
    try {
      client.grpc = await createEvolveGrpcClient(grpcUrl);
      console.log(`Connected to Evolve gRPC at ${grpcUrl}`);
    } catch (err) {
      console.warn(`Failed to connect to gRPC at ${grpcUrl}, falling back to JSON-RPC:`, err);
      client.mode = "jsonrpc";
    }
  }

  return client;
}

// Sync version for backwards compatibility (JSON-RPC only)
export function createEvolveClientSync(rpcUrl?: string): EvolveClient {
  const transport = http(rpcUrl ?? process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545");
  const publicClient = createPublicClient({
    chain: evolveTestnet,
    transport,
  });

  return { mode: "jsonrpc", public: publicClient };
}

/**
 * Query token balance for an address
 */
export async function getBalance(
  client: EvolveClient,
  address: Address
): Promise<bigint> {
  if (client.mode === "grpc" && client.grpc) {
    return client.grpc.getBalance(address);
  }
  return client.public.getBalance({ address });
}

/**
 * Query transaction count (nonce) for an address
 */
export async function getNonce(
  client: EvolveClient,
  address: Address
): Promise<number> {
  if (client.mode === "grpc" && client.grpc) {
    return client.grpc.getTransactionCount(address);
  }
  return client.public.getTransactionCount({ address });
}

/**
 * Get transaction receipt by hash
 */
export async function getTransactionReceipt(
  client: EvolveClient,
  hash: Hash
): Promise<{ status: "success" | "reverted"; blockNumber: bigint; gasUsed: bigint } | null> {
  if (client.mode === "grpc" && client.grpc) {
    const receipt = await client.grpc.getTransactionReceipt(hash);
    if (!receipt) return null;
    return {
      status: receipt.success ? "success" : "reverted",
      blockNumber: BigInt(receipt.blockNumber),
      gasUsed: BigInt(receipt.gasUsed),
    };
  }

  const receipt = await client.public.getTransactionReceipt({ hash });
  if (!receipt) return null;
  return {
    status: receipt.status,
    blockNumber: receipt.blockNumber,
    gasUsed: receipt.gasUsed,
  };
}

/**
 * Wait for transaction confirmation
 */
export async function waitForTransaction(
  client: EvolveClient,
  hash: Hash,
  timeoutMs: number = 30000
): Promise<{ status: "success" | "reverted"; blockNumber: bigint }> {
  const startTime = Date.now();

  // For gRPC, poll for receipt
  if (client.mode === "grpc" && client.grpc) {
    while (Date.now() - startTime < timeoutMs) {
      const receipt = await client.grpc.getTransactionReceipt(hash);
      if (receipt) {
        return {
          status: receipt.success ? "success" : "reverted",
          blockNumber: BigInt(receipt.blockNumber),
        };
      }
      // Poll every 10ms for fast block times
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    throw new Error(`Transaction ${hash} not confirmed within ${timeoutMs}ms`);
  }

  // For JSON-RPC, use viem's built-in wait
  const receipt = await client.public.waitForTransactionReceipt({ hash });
  return {
    status: receipt.status,
    blockNumber: receipt.blockNumber,
  };
}

/**
 * Get current block number
 */
export async function getBlockNumber(client: EvolveClient): Promise<bigint> {
  if (client.mode === "grpc" && client.grpc) {
    const blockNum = await client.grpc.getBlockNumber();
    return BigInt(blockNum);
  }
  return client.public.getBlockNumber();
}

/**
 * Get chain ID
 */
export async function getChainId(client: EvolveClient): Promise<number> {
  return client.public.getChainId();
}

/**
 * Check if transaction was successful
 */
export async function isTransactionSuccessful(
  client: EvolveClient,
  hash: Hash
): Promise<boolean> {
  const receipt = await getTransactionReceipt(client, hash);
  return receipt?.status === "success";
}

/**
 * Close the client (cleanup gRPC connections)
 */
export function closeClient(client: EvolveClient): void {
  if (client.grpc) {
    client.grpc.close();
  }
}

// Custom Evolve RPC methods (evolve_* namespace)
// These use the underlying transport directly since viem doesn't know about evolve_* methods
export async function listModules(client: EvolveClient): Promise<string[]> {
  const response = await fetch(process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "evolve_listModules",
      params: [],
    }),
  });
  const data = (await response.json()) as { result?: string[]; error?: unknown };
  if (data.error) throw new Error(JSON.stringify(data.error));
  return data.result ?? [];
}

export async function getModuleSchema(
  client: EvolveClient,
  moduleId: string
): Promise<unknown> {
  const response = await fetch(process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "evolve_getModuleSchema",
      params: [moduleId],
    }),
  });
  const data = (await response.json()) as { result?: unknown; error?: unknown };
  if (data.error) throw new Error(JSON.stringify(data.error));
  return data.result;
}

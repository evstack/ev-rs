import {
  type Address,
  type Hash,
  type Hex,
  createWalletClient,
  createPublicClient,
  http,
  defineChain,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type {
  AgentConfig,
  PaymentPayload,
  PaymentRequired,
  RequestResult,
  WeightedEndpoint,
} from "./types.js";
import {
  accountIdToAddress,
  addressToAccountId,
  buildTransferData,
} from "./evolve-utils.js";

export class Agent {
  private config: AgentConfig;
  private serverUrl: string;
  private rpcUrl: string;
  private running: boolean = false;
  private requestLoop: ReturnType<typeof setTimeout> | null = null;
  private onResult: ((result: RequestResult) => void) | null = null;

  constructor(
    config: AgentConfig,
    serverUrl: string,
    rpcUrl: string
  ) {
    this.config = config;
    this.serverUrl = serverUrl;
    this.rpcUrl = rpcUrl;
  }

  get id(): string {
    return this.config.id;
  }

  get address(): Address {
    return this.config.address;
  }

  setResultHandler(handler: (result: RequestResult) => void): void {
    this.onResult = handler;
  }

  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;
    this.scheduleNextRequest();
  }

  async stop(): Promise<void> {
    this.running = false;
    if (this.requestLoop) {
      clearTimeout(this.requestLoop);
      this.requestLoop = null;
    }
  }

  private scheduleNextRequest(): void {
    if (!this.running) return;

    const delayMs = 1000 / this.config.requestsPerSecond;
    const jitter = Math.random() * delayMs * 0.2;

    this.requestLoop = setTimeout(async () => {
      try {
        const result = await this.makeRequest();
        this.onResult?.(result);
      } catch (err) {
        console.error(`Agent ${this.config.id} request error:`, err);
      }
      this.scheduleNextRequest();
    }, delayMs + jitter);
  }

  private selectEndpoint(): WeightedEndpoint {
    const endpoints = this.config.endpoints;
    const totalWeight = endpoints.reduce((sum, e) => sum + e.weight, 0);
    let random = Math.random() * totalWeight;

    for (const endpoint of endpoints) {
      random -= endpoint.weight;
      if (random <= 0) {
        return endpoint;
      }
    }
    return endpoints[endpoints.length - 1];
  }

  private async makeRequest(): Promise<RequestResult> {
    const startTime = Date.now();
    const endpoint = this.selectEndpoint();
    const url = `${this.serverUrl}${endpoint.path}`;

    try {
      // Step 1: Make initial request (expect 402)
      const initialResponse = await fetch(url, {
        method: endpoint.method,
        headers: {
          "Content-Type": "application/json",
          "X-Agent-ID": this.config.id,
        },
        body: JSON.stringify(endpoint.payload()),
      });

      if (initialResponse.status !== 402) {
        if (initialResponse.ok) {
          return {
            success: true,
            agentId: this.config.id,
            endpoint: `${endpoint.method} ${endpoint.path}`,
            latencyMs: Date.now() - startTime,
            timestamp: Date.now(),
          };
        }
        return {
          success: false,
          agentId: this.config.id,
          endpoint: `${endpoint.method} ${endpoint.path}`,
          latencyMs: Date.now() - startTime,
          error: `Unexpected status: ${initialResponse.status}`,
          timestamp: Date.now(),
        };
      }

      // Step 2: Parse payment requirement
      const paymentHeader = initialResponse.headers.get("PAYMENT-REQUIRED");
      if (!paymentHeader) {
        return {
          success: false,
          agentId: this.config.id,
          endpoint: `${endpoint.method} ${endpoint.path}`,
          latencyMs: Date.now() - startTime,
          error: "402 without PAYMENT-REQUIRED header",
          timestamp: Date.now(),
        };
      }

      const paymentRequired = JSON.parse(
        Buffer.from(paymentHeader, "base64").toString("utf-8")
      ) as PaymentRequired;

      const amount = BigInt(paymentRequired.accepts[0].amount);
      const payTo = paymentRequired.accepts[0].payTo;

      // Step 3: Submit payment via token transfer
      const paymentStartTime = Date.now();
      const txHash = await this.submitPayment(payTo, amount);
      const paymentLatencyMs = Date.now() - paymentStartTime;

      // Step 4: Retry with payment proof
      const paymentPayload: PaymentPayload = {
        x402Version: 2,
        scheme: "exact",
        network: paymentRequired.accepts[0].network,
        payload: { txHash },
      };

      const paymentSignature = Buffer.from(
        JSON.stringify(paymentPayload)
      ).toString("base64");

      const finalResponse = await fetch(url, {
        method: endpoint.method,
        headers: {
          "Content-Type": "application/json",
          "PAYMENT-SIGNATURE": paymentSignature,
          "X-Agent-ID": this.config.id,
        },
        body: JSON.stringify(endpoint.payload()),
      });

      const totalLatencyMs = Date.now() - startTime;

      if (!finalResponse.ok) {
        const errorBody = await finalResponse.text();
        return {
          success: false,
          agentId: this.config.id,
          endpoint: `${endpoint.method} ${endpoint.path}`,
          txHash,
          latencyMs: totalLatencyMs,
          paymentLatencyMs,
          error: `Payment retry failed: ${finalResponse.status} - ${errorBody}`,
          timestamp: Date.now(),
        };
      }

      return {
        success: true,
        agentId: this.config.id,
        endpoint: `${endpoint.method} ${endpoint.path}`,
        txHash,
        latencyMs: totalLatencyMs,
        paymentLatencyMs,
        timestamp: Date.now(),
      };
    } catch (err) {
      return {
        success: false,
        agentId: this.config.id,
        endpoint: `${endpoint.method} ${endpoint.path}`,
        latencyMs: Date.now() - startTime,
        error: err instanceof Error ? err.message : String(err),
        timestamp: Date.now(),
      };
    }
  }

  private async submitPayment(to: Address, amount: bigint): Promise<Hash> {
    const account = privateKeyToAccount(this.config.privateKey);

    const chain = defineChain({
      id: this.config.chainId,
      name: "Evolve Testnet",
      nativeCurrency: { decimals: 18, name: "Evolve", symbol: "EVO" },
      rpcUrls: { default: { http: [this.rpcUrl] } },
    });

    const walletClient = createWalletClient({
      account,
      chain,
      transport: http(this.rpcUrl),
    });

    // Pay via token transfer calldata
    const payToAccountId = addressToAccountId(to);
    const data = buildTransferData(payToAccountId, amount);
    const tokenAddress = accountIdToAddress(this.config.tokenAccountId);

    const txHash = await walletClient.sendTransaction({
      to: tokenAddress,
      data,
      value: 0n,
      gas: 100_000n,
    });

    const publicClient = createPublicClient({
      chain,
      transport: http(this.rpcUrl),
    });

    await publicClient.waitForTransactionReceipt({ hash: txHash });

    return txHash;
  }

  async getBalance(): Promise<bigint> {
    const publicClient = createPublicClient({
      transport: http(this.rpcUrl),
    });
    return publicClient.getBalance({ address: this.config.address });
  }
}

export function createAgentConfig(
  id: string,
  privateKey: Hex,
  requestsPerSecond: number,
  endpoints: WeightedEndpoint[],
  tokenAccountId: bigint,
  chainId: number,
): AgentConfig {
  const account = privateKeyToAccount(privateKey);
  return {
    id,
    privateKey,
    address: account.address,
    requestsPerSecond,
    endpoints,
    tokenAccountId,
    chainId,
  };
}

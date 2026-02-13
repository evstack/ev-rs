import {
  type Address,
  type Chain,
  type Hash,
  type Hex,
  type HttpTransport,
  type PublicClient,
  type WalletClient,
  createWalletClient,
  createPublicClient,
  keccak256,
  http,
  defineChain,
} from "viem";
import { privateKeyToAccount, type PrivateKeyAccount } from "viem/accounts";
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

const MAX_INFLIGHT = 5;

export class Agent {
  private config: AgentConfig;
  private serverUrl: string;
  private rpcUrl: string;
  private account: PrivateKeyAccount;
  private chain: Chain;
  private walletClient: WalletClient<HttpTransport, Chain, PrivateKeyAccount>;
  private publicClient: PublicClient<HttpTransport, Chain>;
  private nextNonce: number | null = null;
  private running: boolean = false;
  private activeWorkers: Promise<void>[] = [];
  private onResult: ((result: RequestResult) => void) | null = null;

  constructor(
    config: AgentConfig,
    serverUrl: string,
    rpcUrl: string
  ) {
    this.config = config;
    this.serverUrl = serverUrl;
    this.rpcUrl = rpcUrl;
    this.account = privateKeyToAccount(this.config.privateKey);
    this.chain = defineChain({
      id: this.config.chainId,
      name: "Evolve Testnet",
      nativeCurrency: { decimals: 18, name: "Evolve", symbol: "EVO" },
      rpcUrls: { default: { http: [this.rpcUrl] } },
    });
    this.walletClient = createWalletClient({
      account: this.account,
      chain: this.chain,
      transport: http(this.rpcUrl),
    });
    this.publicClient = createPublicClient({
      chain: this.chain,
      transport: http(this.rpcUrl, { timeout: 60_000 }),
      pollingInterval: 100,
    });
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
    for (let i = 0; i < MAX_INFLIGHT; i++) {
      this.activeWorkers.push(this.runRequestLoop());
    }
  }

  async stop(): Promise<void> {
    this.running = false;
    await Promise.all(this.activeWorkers);
    this.activeWorkers = [];
  }

  private async runRequestLoop(): Promise<void> {
    while (this.running) {
      const delayMs = (MAX_INFLIGHT * 1000) / this.config.requestsPerSecond;
      const jitter = Math.random() * delayMs * 0.2;
      await new Promise((resolve) => setTimeout(resolve, delayMs + jitter));

      if (!this.running) break;

      try {
        const result = await this.makeRequest();
        this.onResult?.(result);
      } catch (err) {
        console.error(`Agent ${this.config.id} request error:`, err);
      }
    }
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
      const payTo = paymentRequired.accepts[0].payTo as Address;

      // Step 3: Submit payment via token transfer
      const paymentStartTime = Date.now();
      const txHash = await this.submitPayment(payTo, amount);
      const paymentLatencyMs = Date.now() - paymentStartTime;

      // Step 4: Retry with payment proof (v2 format with resource + accepted)
      const paymentPayload: PaymentPayload = {
        x402Version: 2,
        resource: paymentRequired.resource,
        accepted: paymentRequired.accepts[0],
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
    // Pay via token transfer calldata.
    // We manage nonce locally because the RPC nonce endpoints can lag under load.
    if (this.nextNonce === null) {
      this.nextNonce = await this.publicClient.getTransactionCount({
        address: this.account.address,
        blockTag: "pending",
      });
    }

    const payToAccountId = addressToAccountId(to);
    const data = buildTransferData(payToAccountId, amount);
    const tokenAddress = accountIdToAddress(this.config.tokenAccountId);

    // Claim nonce synchronously before any await to prevent races between concurrent workers
    const nonce = this.nextNonce;
    this.nextNonce = nonce + 1;

    let hash: Hash;
    try {
      hash = await this.walletClient.sendTransaction({
        nonce,
        to: tokenAddress,
        data,
        value: 0n,
        gas: 100_000n,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);

      if (/transaction already in mempool|already known/i.test(msg)) {
        const rawTxMatch = msg.match(/"params":\["(0x[0-9a-fA-F]+)"\]/);
        if (rawTxMatch?.[1]) {
          hash = keccak256(rawTxMatch[1] as `0x${string}`);
          await this.publicClient.waitForTransactionReceipt({ hash, timeout: 60_000 });
          return hash;
        }
      }

      if (/nonce too high/i.test(msg)) {
        this.nextNonce = await this.publicClient.getTransactionCount({
          address: this.account.address,
          blockTag: "pending",
        });
      } else if (!/nonce/i.test(msg) && !/already in mempool/i.test(msg)) {
        // Non-nonce error: the tx was not sent, rollback the nonce
        this.nextNonce = nonce;
      }

      throw err;
    }

    try {
      await this.publicClient.waitForTransactionReceipt({ hash, timeout: 60_000 });
      return hash;
    } catch (err) {
      throw err;
    }
  }

  async getBalance(): Promise<bigint> {
    return this.publicClient.getBalance({ address: this.config.address });
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

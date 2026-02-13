import {
  type Hash,
  createWalletClient,
  createPublicClient,
  http,
  defineChain,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { Agent, createAgentConfig } from "./agent.js";
import { MetricsCollector } from "./metrics.js";
import type { PoolConfig, PoolMetrics, RequestResult } from "./types.js";
import {
  getAgentPrivateKeys,
  TOKEN_ACCOUNT_CANDIDATES,
  accountIdToAddress,
  addressToAccountId,
  buildTransferData,
} from "./evolve-utils.js";

export class AgentPool {
  private config: PoolConfig;
  private agents: Agent[] = [];
  private metrics: MetricsCollector;
  private running: boolean = false;
  private metricsInterval: ReturnType<typeof setInterval> | null = null;
  private onMetricsUpdate: ((metrics: PoolMetrics) => void) | null = null;

  constructor(config: PoolConfig) {
    this.config = config;
    this.metrics = new MetricsCollector();
  }

  setMetricsHandler(handler: (metrics: PoolMetrics) => void): void {
    this.onMetricsUpdate = handler;
  }

  async initialize(): Promise<void> {
    const agentKeys = getAgentPrivateKeys(this.config.agentCount);
    const agentCount = agentKeys.length;
    console.log(`Initializing pool with ${agentCount} agents...`);

    const publicClient = createPublicClient({
      transport: http(this.config.evolveRpcUrl),
    });

    // Get chain ID dynamically
    const chainId = await publicClient.getChainId();
    console.log(`Chain ID: ${chainId}`);

    const evolveChain = defineChain({
      id: chainId,
      name: "Evolve Testnet",
      nativeCurrency: { decimals: 18, name: "Evolve", symbol: "EVO" },
      rpcUrls: { default: { http: [this.config.evolveRpcUrl] } },
    });

    // Set up faucet wallet
    const faucetAccount = privateKeyToAccount(this.config.faucetPrivateKey);
    const faucetWallet = createWalletClient({
      account: faucetAccount,
      chain: evolveChain,
      transport: http(this.config.evolveRpcUrl),
    });

    const chainPublicClient = createPublicClient({
      chain: evolveChain,
      transport: http(this.config.evolveRpcUrl),
    });

    console.log(`Faucet address: ${faucetAccount.address}`);

    // Calculate RPS per agent
    const rpsPerAgent = this.config.requestsPerSecond / agentCount;

    // Discover token account ID by trial-funding the first agent.
    // eth_getCode is not implemented, so we try each candidate with a real transfer.
    let tokenAccountId: bigint | null = null;
    const firstKey = agentKeys[0];
    const firstAgentConfig = createAgentConfig(
      `agent-000`,
      firstKey,
      rpsPerAgent,
      this.config.endpoints,
      0n, // placeholder, will be set after discovery
      chainId,
    );
    const firstAgentAccountId = addressToAccountId(firstAgentConfig.address);

    for (const candidate of TOKEN_ACCOUNT_CANDIDATES) {
      const tokenAddress = accountIdToAddress(candidate);
      const data = buildTransferData(firstAgentAccountId, this.config.fundingAmount);
      console.log(`Trying token candidate ${candidate} (${tokenAddress})...`);
      try {
        const txHash = await faucetWallet.sendTransaction({
          to: tokenAddress,
          data,
          value: 0n,
          gas: 100_000n,
        });
        const receipt = await chainPublicClient.waitForTransactionReceipt({ hash: txHash });
        if (receipt.status === "success") {
          tokenAccountId = candidate;
          console.log(`Found token at account ID ${candidate}`);
          break;
        }
        console.warn(`Token candidate ${candidate} tx reverted`);
      } catch (err) {
        console.warn(`Token candidate ${candidate} failed:`, err);
      }
    }
    if (tokenAccountId === null) {
      throw new Error("Could not find token account. Tried all candidates.");
    }

    // First agent already funded during discovery - create it
    const fundedFirstConfig = createAgentConfig(
      `agent-000`,
      firstKey,
      rpsPerAgent,
      this.config.endpoints,
      tokenAccountId,
      chainId,
    );
    const firstAgent = new Agent(fundedFirstConfig, this.config.serverUrl, this.config.evolveRpcUrl);
    firstAgent.setResultHandler((result) => this.handleAgentResult(result));
    this.agents.push(firstAgent);
    this.metrics.registerAgent(fundedFirstConfig.id, fundedFirstConfig.address);
    console.log(`Funded agent ${fundedFirstConfig.id} (${fundedFirstConfig.address.slice(0, 10)}...) with ${this.config.fundingAmount} tokens`);

    // Fund remaining agents in parallel: batch-send all txs, then wait for the last receipt
    const fundingTxHashes: Hash[] = [];
    const pendingAgents: { config: ReturnType<typeof createAgentConfig> }[] = [];
    const tokenAddress = accountIdToAddress(tokenAccountId);

    for (let i = 1; i < agentCount; i++) {
      const privateKey = agentKeys[i];
      const agentConfig = createAgentConfig(
        `agent-${i.toString().padStart(3, "0")}`,
        privateKey,
        rpsPerAgent,
        this.config.endpoints,
        tokenAccountId,
        chainId,
      );

      const agentAccountId = addressToAccountId(agentConfig.address);
      const data = buildTransferData(agentAccountId, this.config.fundingAmount);

      console.log(
        `Funding agent ${agentConfig.id} (${agentConfig.address.slice(0, 10)}...) with ${this.config.fundingAmount} tokens`
      );

      const txHash = await faucetWallet.sendTransaction({
        to: tokenAddress,
        data,
        value: 0n,
        gas: 100_000n,
      });

      fundingTxHashes.push(txHash);
      pendingAgents.push({ config: agentConfig });
    }

    // Wait only for the last receipt - all prior ones are confirmed by then
    if (fundingTxHashes.length > 0) {
      const lastTxHash = fundingTxHashes[fundingTxHashes.length - 1];
      await chainPublicClient.waitForTransactionReceipt({ hash: lastTxHash });
    }

    // Create all agents after funding is confirmed
    for (const { config: agentConfig } of pendingAgents) {
      const agent = new Agent(
        agentConfig,
        this.config.serverUrl,
        this.config.evolveRpcUrl
      );

      agent.setResultHandler((result) => this.handleAgentResult(result));

      this.agents.push(agent);
      this.metrics.registerAgent(agentConfig.id, agentConfig.address);
    }

    console.log(`Pool initialized with ${this.agents.length} agents`);
  }

  private handleAgentResult(result: RequestResult): void {
    this.metrics.recordRequest(result);

    const status = result.success ? "OK" : "FAIL";
    const latency = `${result.latencyMs}ms`;
    const payment = result.paymentLatencyMs
      ? ` (payment: ${result.paymentLatencyMs}ms)`
      : "";
    const error = result.error ? ` - ${result.error}` : "";

    console.log(
      `[${result.agentId}] ${status} ${result.endpoint} ${latency}${payment}${error}`
    );
  }

  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;

    console.log("\nStarting agents...");
    this.metrics.start();

    // Stagger agent starts to spread load evenly
    const staggerWindowMs = 1000 / this.config.requestsPerSecond;
    await Promise.all(
      this.agents.map((agent) => {
        const delay = Math.random() * staggerWindowMs;
        return new Promise<void>((resolve) =>
          setTimeout(async () => {
            await agent.start();
            resolve();
          }, delay)
        );
      })
    );

    this.metricsInterval = setInterval(() => {
      const poolMetrics = this.metrics.getPoolMetrics();
      this.onMetricsUpdate?.(poolMetrics);
      console.log(this.metrics.formatSummary());
    }, 5000);

    console.log(`All ${this.agents.length} agents started`);
    console.log(`Target TPS: ${this.config.requestsPerSecond}`);
    console.log(`Server: ${this.config.serverUrl}`);
    console.log("");
  }

  async stop(): Promise<void> {
    if (!this.running) return;
    this.running = false;

    console.log("\nStopping agents...");

    if (this.metricsInterval) {
      clearInterval(this.metricsInterval);
      this.metricsInterval = null;
    }

    await Promise.all(this.agents.map((agent) => agent.stop()));

    console.log("All agents stopped");
    console.log(this.metrics.formatSummary());
  }

  getMetrics(): PoolMetrics {
    return this.metrics.getPoolMetrics();
  }

  async getAgentBalances(): Promise<Map<string, bigint>> {
    const balances = new Map<string, bigint>();
    await Promise.all(
      this.agents.map(async (agent) => {
        const balance = await agent.getBalance();
        balances.set(agent.id, balance);
        this.metrics.updateAgentBalance(agent.id, balance);
      })
    );
    return balances;
  }
}

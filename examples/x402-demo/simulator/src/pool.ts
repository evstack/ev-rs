import {
  createWalletClient,
  createPublicClient,
  http,
  formatEther,
  defineChain,
} from "viem";
import { privateKeyToAccount, generatePrivateKey } from "viem/accounts";
import { Agent, createAgentConfig } from "./agent.js";
import { MetricsCollector } from "./metrics.js";
import type { PoolConfig, PoolMetrics, RequestResult } from "./types.js";

// Evolve chain definition
const evolveChain = defineChain({
  id: 1337,
  name: "Evolve Testnet",
  nativeCurrency: { decimals: 18, name: "Evolve", symbol: "EVO" },
  rpcUrls: { default: { http: ["http://127.0.0.1:8545"] } },
});

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
    console.log(`Initializing pool with ${this.config.agentCount} agents...`);

    // Generate wallets and fund them
    const faucetAccount = privateKeyToAccount(this.config.faucetPrivateKey);
    const faucetWallet = createWalletClient({
      account: faucetAccount,
      chain: {
        ...evolveChain,
        rpcUrls: { default: { http: [this.config.evolveRpcUrl] } },
      },
      transport: http(this.config.evolveRpcUrl),
    });

    const publicClient = createPublicClient({
      chain: {
        ...evolveChain,
        rpcUrls: { default: { http: [this.config.evolveRpcUrl] } },
      },
      transport: http(this.config.evolveRpcUrl),
    });

    // Check faucet balance
    const faucetBalance = await publicClient.getBalance({
      address: faucetAccount.address,
    });
    const requiredFunding =
      this.config.fundingAmount * BigInt(this.config.agentCount);

    console.log(`Faucet address: ${faucetAccount.address}`);
    console.log(`Faucet balance: ${formatEther(faucetBalance)} EVO`);
    console.log(`Required funding: ${formatEther(requiredFunding)} EVO`);

    if (faucetBalance < requiredFunding) {
      throw new Error(
        `Insufficient faucet balance. Have ${formatEther(faucetBalance)}, need ${formatEther(requiredFunding)}`
      );
    }

    // Calculate RPS per agent
    const rpsPerAgent = this.config.requestsPerSecond / this.config.agentCount;

    // Create and fund agents
    for (let i = 0; i < this.config.agentCount; i++) {
      const privateKey = generatePrivateKey();
      const agentConfig = createAgentConfig(
        `agent-${i.toString().padStart(3, "0")}`,
        privateKey,
        rpsPerAgent,
        this.config.endpoints
      );

      // Fund agent
      console.log(
        `Funding agent ${agentConfig.id} (${agentConfig.address.slice(0, 10)}...) with ${formatEther(this.config.fundingAmount)} EVO`
      );

      const txHash = await faucetWallet.sendTransaction({
        to: agentConfig.address,
        value: this.config.fundingAmount,
      });

      // Wait for funding tx
      await publicClient.waitForTransactionReceipt({ hash: txHash });

      // Create agent
      const agent = new Agent(
        agentConfig,
        this.config.serverUrl,
        this.config.evolveRpcUrl
      );

      // Set up result handler
      agent.setResultHandler((result) => this.handleAgentResult(result));

      this.agents.push(agent);
      this.metrics.registerAgent(agentConfig.id, agentConfig.address);
    }

    console.log(`Pool initialized with ${this.agents.length} agents`);
  }

  private handleAgentResult(result: RequestResult): void {
    this.metrics.recordRequest(result);

    // Log individual results (verbose mode could make this optional)
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

    // Start all agents
    await Promise.all(this.agents.map((agent) => agent.start()));

    // Start metrics reporting
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

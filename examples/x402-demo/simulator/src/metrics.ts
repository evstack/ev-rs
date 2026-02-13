import type { Address } from "viem";
import type { AgentMetrics, PoolMetrics, RequestResult } from "./types.js";

// Sliding window for TPS calculation
const TPS_WINDOW_MS = 1000;

export class MetricsCollector {
  private agentMetrics: Map<string, AgentMetrics> = new Map();
  private latencies: number[] = [];
  private recentRequests: number[] = []; // timestamps for TPS calculation
  private startTime: number = 0;

  start(): void {
    this.startTime = Date.now();
    this.latencies = [];
    this.recentRequests = [];
  }

  registerAgent(agentId: string, address: Address): void {
    this.agentMetrics.set(agentId, {
      agentId,
      address,
      totalRequests: 0,
      successfulRequests: 0,
      failedRequests: 0,
      totalLatencyMs: 0,
      totalPaymentLatencyMs: 0,
      balance: 0n,
      lastRequestTime: 0,
    });
  }

  recordRequest(result: RequestResult): void {
    const metrics = this.agentMetrics.get(result.agentId);
    if (!metrics) return;

    metrics.totalRequests++;
    metrics.lastRequestTime = result.timestamp;
    metrics.totalLatencyMs += result.latencyMs;

    if (result.paymentLatencyMs) {
      metrics.totalPaymentLatencyMs += result.paymentLatencyMs;
    }

    if (result.success) {
      metrics.successfulRequests++;
    } else {
      metrics.failedRequests++;
    }

    this.latencies.push(result.latencyMs);
    this.recentRequests.push(result.timestamp);

    // Keep latencies bounded
    if (this.latencies.length > 10000) {
      this.latencies = this.latencies.slice(-5000);
    }
  }

  updateAgentBalance(agentId: string, balance: bigint): void {
    const metrics = this.agentMetrics.get(agentId);
    if (metrics) {
      metrics.balance = balance;
    }
  }

  getPoolMetrics(): PoolMetrics {
    const now = Date.now();
    const agents = Array.from(this.agentMetrics.values());

    // Calculate totals
    const totalRequests = agents.reduce((sum, a) => sum + a.totalRequests, 0);
    const successfulRequests = agents.reduce(
      (sum, a) => sum + a.successfulRequests,
      0
    );
    const failedRequests = agents.reduce((sum, a) => sum + a.failedRequests, 0);

    // Calculate latency percentiles
    const sortedLatencies = [...this.latencies].sort((a, b) => a - b);
    const avgLatencyMs =
      sortedLatencies.length > 0
        ? sortedLatencies.reduce((a, b) => a + b, 0) / sortedLatencies.length
        : 0;
    const p50LatencyMs = this.percentile(sortedLatencies, 50);
    const p95LatencyMs = this.percentile(sortedLatencies, 95);
    const p99LatencyMs = this.percentile(sortedLatencies, 99);

    // Calculate current TPS (requests in last window)
    const windowStart = now - TPS_WINDOW_MS;
    this.recentRequests = this.recentRequests.filter((t) => t > windowStart);
    const currentTps = this.recentRequests.length / (TPS_WINDOW_MS / 1000);

    return {
      totalRequests,
      successfulRequests,
      failedRequests,
      avgLatencyMs: Math.round(avgLatencyMs * 100) / 100,
      p50LatencyMs,
      p95LatencyMs,
      p99LatencyMs,
      currentTps: Math.round(currentTps * 10) / 10,
      agents,
      startTime: this.startTime,
      elapsedMs: now - this.startTime,
    };
  }

  private percentile(sorted: number[], p: number): number {
    if (sorted.length === 0) return 0;
    const idx = Math.ceil((p / 100) * sorted.length) - 1;
    return sorted[Math.max(0, idx)];
  }

  // Format metrics for console output
  formatSummary(): string {
    const m = this.getPoolMetrics();
    const successRate =
      m.totalRequests > 0
        ? ((m.successfulRequests / m.totalRequests) * 100).toFixed(1)
        : "0.0";

    const lines = [
      "",
      "=== Simulator Metrics ===",
      `Elapsed: ${(m.elapsedMs / 1000).toFixed(1)}s`,
      `Total Requests: ${m.totalRequests}`,
      `Success Rate: ${successRate}%`,
      `Current TPS: ${m.currentTps}`,
      "",
      "Latency:",
      `  Avg: ${m.avgLatencyMs}ms`,
      `  p50: ${m.p50LatencyMs}ms`,
      `  p95: ${m.p95LatencyMs}ms`,
      `  p99: ${m.p99LatencyMs}ms`,
      "",
      `Active Agents: ${m.agents.length}`,
    ];

    return lines.join("\n");
  }
}

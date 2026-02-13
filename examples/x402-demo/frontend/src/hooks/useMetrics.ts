import { useMemo, useState, useEffect, useRef } from "react";
import type { PaymentEvent } from "./useEventStream";

export interface AgentStatus {
  id: string;
  totalRequests: number;
  successfulRequests: number;
  failedRequests: number;
  totalLatencyMs: number;
  lastSeen: number;
  lastStatus: "success" | "failed" | "pending";
}

export interface Metrics {
  totalRequests: number;
  successfulRequests: number;
  failedRequests: number;
  mempoolErrors: number;
  successRate: number;
  avgLatencyMs: number;
  p50LatencyMs: number;
  p95LatencyMs: number;
  p99LatencyMs: number;
  currentTps: number;
  agents: Map<string, AgentStatus>;
  latencyHistogram: number[];
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}

function isMempoolError(message?: string): boolean {
  if (!message) return false;
  return /transaction already in mempool|already known/i.test(message);
}

export function useMetrics(events: PaymentEvent[]): Metrics {
  const [tps, setTps] = useState(0);
  const recentRequestsRef = useRef<number[]>([]);

  // Calculate TPS every 100ms
  useEffect(() => {
    const interval = setInterval(() => {
      const now = Date.now();
      const windowMs = 1000;
      recentRequestsRef.current = recentRequestsRef.current.filter(
        (t) => now - t < windowMs
      );
      setTps(recentRequestsRef.current.length);
    }, 100);

    return () => clearInterval(interval);
  }, []);

  // Track new request_served events for TPS
  useEffect(() => {
    const servedEvents = events.filter((e) => e.type === "request_served");
    if (servedEvents.length > 0) {
      const lastEvent = servedEvents[servedEvents.length - 1];
      if (
        !recentRequestsRef.current.includes(lastEvent.timestamp) &&
        Date.now() - lastEvent.timestamp < 2000
      ) {
        recentRequestsRef.current.push(lastEvent.timestamp);
      }
    }
  }, [events]);

  return useMemo(() => {
    const agents = new Map<string, AgentStatus>();
    const latencies: number[] = [];

    let totalRequests = 0;
    let successfulRequests = 0;
    let failedRequests = 0;
    let mempoolErrors = 0;

    for (const event of events) {
      if (!event.agentId) continue;

      // Initialize agent if not exists
      if (!agents.has(event.agentId)) {
        agents.set(event.agentId, {
          id: event.agentId,
          totalRequests: 0,
          successfulRequests: 0,
          failedRequests: 0,
          totalLatencyMs: 0,
          lastSeen: event.timestamp,
          lastStatus: "pending",
        });
      }

      const agent = agents.get(event.agentId)!;
      agent.lastSeen = event.timestamp;

      switch (event.type) {
        case "request_served":
          totalRequests++;
          successfulRequests++;
          agent.totalRequests++;
          agent.successfulRequests++;
          agent.lastStatus = "success";
          if (event.latencyMs) {
            latencies.push(event.latencyMs);
            agent.totalLatencyMs += event.latencyMs;
          }
          break;

        case "payment_failed":
        case "error":
          failedRequests++;
          agent.failedRequests++;
          agent.lastStatus = "failed";
          if (isMempoolError(event.error)) {
            mempoolErrors++;
          }
          break;

        case "payment_submitted":
          agent.lastStatus = "pending";
          break;
      }
    }

    // Calculate latency stats
    const sortedLatencies = [...latencies].sort((a, b) => a - b);
    const avgLatencyMs =
      sortedLatencies.length > 0
        ? sortedLatencies.reduce((a, b) => a + b, 0) / sortedLatencies.length
        : 0;

    // Create histogram buckets (0-50, 50-100, 100-200, 200-500, 500+)
    const buckets = [0, 0, 0, 0, 0];
    for (const lat of latencies) {
      if (lat < 50) buckets[0]++;
      else if (lat < 100) buckets[1]++;
      else if (lat < 200) buckets[2]++;
      else if (lat < 500) buckets[3]++;
      else buckets[4]++;
    }

    const successRate =
      totalRequests > 0 ? (successfulRequests / totalRequests) * 100 : 0;

    return {
      totalRequests,
      successfulRequests,
      failedRequests,
      mempoolErrors,
      successRate: Math.round(successRate * 10) / 10,
      avgLatencyMs: Math.round(avgLatencyMs),
      p50LatencyMs: percentile(sortedLatencies, 50),
      p95LatencyMs: percentile(sortedLatencies, 95),
      p99LatencyMs: percentile(sortedLatencies, 99),
      currentTps: tps,
      agents,
      latencyHistogram: buckets,
    };
  }, [events, tps]);
}

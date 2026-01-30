import type { Hash, Address } from "viem";

// Event types for the dashboard
export type PaymentEventType =
  | "payment_submitted"
  | "payment_confirmed"
  | "payment_failed"
  | "request_served"
  | "agent_connected"
  | "agent_disconnected"
  | "error";

export interface PaymentEvent {
  type: PaymentEventType;
  timestamp: number;
  agentId: string;
  txHash?: Hash;
  amount?: string;
  endpoint?: string;
  latencyMs?: number;
  error?: string;
  from?: Address;
  to?: Address;
}

// Global event emitter for payment events
class EventEmitter {
  private connections: Set<WebSocket> = new Set();
  private eventLog: PaymentEvent[] = [];
  private maxLogSize = 1000;

  addConnection(ws: WebSocket): void {
    this.connections.add(ws);

    // Send recent events to new connection
    if (this.eventLog.length > 0) {
      const recentEvents = this.eventLog.slice(-100);
      ws.send(JSON.stringify({ type: "history", events: recentEvents }));
    }
  }

  removeConnection(ws: WebSocket): void {
    this.connections.delete(ws);
  }

  emit(event: PaymentEvent): void {
    // Add to log
    this.eventLog.push(event);
    if (this.eventLog.length > this.maxLogSize) {
      this.eventLog = this.eventLog.slice(-this.maxLogSize / 2);
    }

    // Broadcast to all connections
    const message = JSON.stringify(event);
    for (const ws of this.connections) {
      try {
        ws.send(message);
      } catch {
        // Connection may be closed
        this.connections.delete(ws);
      }
    }
  }

  getConnectionCount(): number {
    return this.connections.size;
  }

  getRecentEvents(count: number = 100): PaymentEvent[] {
    return this.eventLog.slice(-count);
  }

  // Aggregated metrics for dashboard
  getMetrics(): {
    totalPayments: number;
    successfulPayments: number;
    failedPayments: number;
    totalRequests: number;
    uniqueAgents: Set<string>;
  } {
    const metrics = {
      totalPayments: 0,
      successfulPayments: 0,
      failedPayments: 0,
      totalRequests: 0,
      uniqueAgents: new Set<string>(),
    };

    for (const event of this.eventLog) {
      if (event.agentId) {
        metrics.uniqueAgents.add(event.agentId);
      }

      switch (event.type) {
        case "payment_submitted":
          metrics.totalPayments++;
          break;
        case "payment_confirmed":
          metrics.successfulPayments++;
          break;
        case "payment_failed":
          metrics.failedPayments++;
          break;
        case "request_served":
          metrics.totalRequests++;
          break;
      }
    }

    return metrics;
  }
}

// Singleton instance
export const eventEmitter = new EventEmitter();

// Helper to emit payment submitted event
export function emitPaymentSubmitted(
  agentId: string,
  txHash: Hash,
  amount: string,
  from?: Address,
  to?: Address
): void {
  eventEmitter.emit({
    type: "payment_submitted",
    timestamp: Date.now(),
    agentId,
    txHash,
    amount,
    from,
    to,
  });
}

// Helper to emit payment confirmed event
export function emitPaymentConfirmed(
  agentId: string,
  txHash: Hash,
  latencyMs?: number
): void {
  eventEmitter.emit({
    type: "payment_confirmed",
    timestamp: Date.now(),
    agentId,
    txHash,
    latencyMs,
  });
}

// Helper to emit payment failed event
export function emitPaymentFailed(
  agentId: string,
  txHash: Hash,
  error: string
): void {
  eventEmitter.emit({
    type: "payment_failed",
    timestamp: Date.now(),
    agentId,
    txHash,
    error,
  });
}

// Helper to emit request served event
export function emitRequestServed(
  agentId: string,
  endpoint: string,
  latencyMs: number,
  txHash?: Hash
): void {
  eventEmitter.emit({
    type: "request_served",
    timestamp: Date.now(),
    agentId,
    endpoint,
    latencyMs,
    txHash,
  });
}

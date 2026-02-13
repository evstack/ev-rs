import type { Hash } from "viem";
import type { MiddlewareHandler } from "hono";
import type { x402ResourceServer } from "@x402/core/server";
import {
  emitPaymentSubmitted,
  emitPaymentConfirmed,
  emitPaymentFailed,
  emitRequestServed,
} from "../events.js";

export { paymentMiddleware } from "@x402/hono";

// Maps txHash → agentId so x402 hooks can identify agents from HTTP context.
const txAgentMap = new Map<string, string>();
const verifyTimestamps = new Map<string, number>();

const EVICTION_AGE_MS = 300_000; // 5 min

setInterval(() => {
  const now = Date.now();
  for (const [hash, ts] of verifyTimestamps) {
    if (now - ts > EVICTION_AGE_MS) {
      verifyTimestamps.delete(hash);
      txAgentMap.delete(hash);
    }
  }
}, 60_000);

/**
 * Pre-middleware: captures X-Agent-ID from the HTTP request and maps it
 * to the txHash in the payment payload so hooks can look it up.
 */
export function captureAgentId(): MiddlewareHandler {
  return async (c, next) => {
    const agentId = c.req.header("X-Agent-ID");
    const sig = c.req.header("PAYMENT-SIGNATURE");
    if (agentId && sig) {
      try {
        const decoded = JSON.parse(Buffer.from(sig, "base64").toString());
        if (decoded?.payload?.txHash) {
          txAgentMap.set(decoded.payload.txHash, agentId);
        }
      } catch { /* ignore malformed headers */ }
    }
    await next();
  };
}

function extractTx(payload: { payload: Record<string, unknown> }) {
  const txHash = payload.payload.txHash as string;
  return { txHash, agentId: txAgentMap.get(txHash) ?? txHash.slice(0, 10) };
}

function extractEndpoint(payload: { resource?: { url?: string } }): string {
  try { return new URL(payload.resource?.url ?? "").pathname; } catch { return "unknown"; }
}

export function registerEventHooks(resourceServer: x402ResourceServer): void {
  resourceServer.onAfterVerify(async ({ paymentPayload, requirements, result }) => {
    if (!result.isValid) return;
    const { txHash, agentId } = extractTx(paymentPayload);
    verifyTimestamps.set(txHash, Date.now());
    emitPaymentSubmitted(agentId, txHash as Hash, requirements.amount, undefined, requirements.payTo as `0x${string}`);
  });

  resourceServer.onAfterSettle(async ({ paymentPayload }) => {
    const { txHash, agentId } = extractTx(paymentPayload);
    const startTime = verifyTimestamps.get(txHash);
    const latencyMs = startTime ? Date.now() - startTime : 0;
    verifyTimestamps.delete(txHash);
    emitPaymentConfirmed(agentId, txHash as Hash, latencyMs);
    emitRequestServed(agentId, extractEndpoint(paymentPayload), latencyMs, txHash as Hash);
  });

  resourceServer.onVerifyFailure(async ({ paymentPayload, error }) => {
    const { txHash, agentId } = extractTx(paymentPayload);
    emitPaymentFailed(agentId, txHash as Hash, error?.message ?? "Verification failed");
  });

  resourceServer.onSettleFailure(async ({ paymentPayload, error }) => {
    const { txHash, agentId } = extractTx(paymentPayload);
    emitPaymentFailed(agentId, txHash as Hash, error?.message ?? "Settlement failed");
  });
}

import { type Context, type MiddlewareHandler } from "hono";
import { type Address, type Hash, isHash } from "viem";
import { type EvolveClient, getTransactionReceipt } from "./evolve.js";
import {
  emitPaymentSubmitted,
  emitPaymentConfirmed,
  emitPaymentFailed,
  emitRequestServed,
} from "./events.js";

// X402 Protocol Types (compatible with @x402/core)
export type PaymentRequirements = {
  scheme: "exact";
  network: string;
  asset: string;
  amount: string;
  payTo: Address;
  maxTimeoutSeconds: number;
  extra?: Record<string, unknown>;
};

export type PaymentRequired = {
  x402Version: 2;
  error: string;
  resource: {
    url: string;
    method: string;
  };
  accepts: PaymentRequirements[];
  description?: string;
};

export type PaymentPayload = {
  x402Version: 2;
  scheme: "exact";
  network: string;
  payload: {
    txHash: Hash;
  };
};

export type PaymentSettled = {
  x402Version: 2;
  success: boolean;
  transaction?: Hash;
  error?: string;
};

// Header names (v2 protocol)
const HEADER_PAYMENT_REQUIRED = "PAYMENT-REQUIRED";
const HEADER_PAYMENT_SIGNATURE = "PAYMENT-SIGNATURE";
const HEADER_PAYMENT_RESPONSE = "PAYMENT-RESPONSE";

// Route pricing configuration
export type RouteConfig = {
  price: bigint;
  description: string;
};

export type X402Config = {
  routes: Record<string, RouteConfig>;
  payTo: Address;
  network: string;
  asset: string;
  client: EvolveClient;
  // Cache of used tx hashes for replay protection
  usedTxHashes?: Set<string>;
};

/**
 * Encode object to base64 for header
 */
function encodeHeader(obj: unknown): string {
  return Buffer.from(JSON.stringify(obj)).toString("base64");
}

/**
 * Decode base64 header to object
 */
function decodeHeader<T>(header: string): T | null {
  try {
    return JSON.parse(Buffer.from(header, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

/**
 * Create 402 Payment Required response
 */
function paymentRequiredResponse(
  c: Context,
  config: X402Config,
  routeConfig: RouteConfig
): Response {
  const paymentRequired: PaymentRequired = {
    x402Version: 2,
    error: "payment_required",
    resource: {
      url: c.req.url,
      method: c.req.method,
    },
    accepts: [
      {
        scheme: "exact",
        network: config.network,
        asset: config.asset,
        amount: routeConfig.price.toString(),
        payTo: config.payTo,
        maxTimeoutSeconds: 300,
      },
    ],
    description: routeConfig.description,
  };

  return c.json(
    {
      error: "payment_required",
      description: routeConfig.description,
      price: routeConfig.price.toString(),
      payTo: config.payTo,
    },
    402,
    {
      [HEADER_PAYMENT_REQUIRED]: encodeHeader(paymentRequired),
    }
  );
}

/**
 * Verify payment against Evolve node
 */
async function verifyPayment(
  config: X402Config,
  payload: PaymentPayload,
  expectedAmount: bigint,
  expectedRecipient: Address
): Promise<{ valid: boolean; error?: string }> {
  const { txHash } = payload.payload;

  // Check for replay
  if (config.usedTxHashes?.has(txHash)) {
    return { valid: false, error: "Transaction already used" };
  }

  try {
    const receipt = await getTransactionReceipt(config.client, txHash);

    if (!receipt) {
      return { valid: false, error: "Transaction not found" };
    }

    if (receipt.status !== "success") {
      return { valid: false, error: "Transaction failed" };
    }

    // For now, we trust the transaction exists and succeeded
    // Full verification would check:
    // 1. Transfer logs for correct amount and recipient
    // 2. Transaction sender matches authenticated user
    // 3. Transaction is recent enough (within maxTimeoutSeconds)

    // Mark tx as used for replay protection
    config.usedTxHashes?.add(txHash);

    return { valid: true };
  } catch (err) {
    console.error("Payment verification failed:", err);
    return { valid: false, error: "Verification failed" };
  }
}

/**
 * X402 middleware for Hono
 * Protects routes with payment requirements and verifies payment proofs
 */
export function x402Middleware(config: X402Config): MiddlewareHandler {
  // Initialize replay protection cache
  config.usedTxHashes = config.usedTxHashes ?? new Set();

  return async (c, next) => {
    // Build route key from method + path pattern
    const path = new URL(c.req.url).pathname;
    const routeKey = `${c.req.method} ${path}`;

    // Check if route requires payment
    let routeConfig: RouteConfig | undefined;
    for (const [pattern, cfg] of Object.entries(config.routes)) {
      // Simple pattern matching: exact match or wildcard suffix
      if (pattern === routeKey) {
        routeConfig = cfg;
        break;
      }
      // Handle wildcards like "POST /api/transform/*"
      if (pattern.endsWith("/*")) {
        const prefix = pattern.slice(0, -1);
        const [method, pathPrefix] = prefix.split(" ");
        if (c.req.method === method && path.startsWith(pathPrefix)) {
          routeConfig = cfg;
          break;
        }
      }
    }

    // Route not protected, continue
    if (!routeConfig) {
      return next();
    }

    // Check for payment proof header
    const paymentHeader = c.req.header(HEADER_PAYMENT_SIGNATURE);

    if (!paymentHeader) {
      return paymentRequiredResponse(c, config, routeConfig);
    }

    // Decode and validate payment payload
    const payload = decodeHeader<PaymentPayload>(paymentHeader);

    if (!payload) {
      return c.json({ error: "Invalid payment header" }, 400);
    }

    if (payload.x402Version !== 2) {
      return c.json({ error: "Unsupported x402 version" }, 400);
    }

    if (!payload.payload?.txHash || !isHash(payload.payload.txHash)) {
      return c.json({ error: "Invalid transaction hash" }, 400);
    }

    // Extract agent ID from request (use tx hash prefix as fallback)
    const agentId = c.req.header("X-Agent-ID") ?? payload.payload.txHash.slice(0, 10);
    const requestStartTime = Date.now();

    // Emit payment submitted event
    emitPaymentSubmitted(
      agentId,
      payload.payload.txHash,
      routeConfig.price.toString(),
      undefined,
      config.payTo
    );

    // Verify payment on-chain
    const verification = await verifyPayment(
      config,
      payload,
      routeConfig.price,
      config.payTo
    );

    if (!verification.valid) {
      emitPaymentFailed(agentId, payload.payload.txHash, verification.error ?? "Unknown error");
      return c.json(
        { error: "Payment verification failed", reason: verification.error },
        402
      );
    }

    // Emit payment confirmed event
    emitPaymentConfirmed(agentId, payload.payload.txHash);

    // Payment verified - add settlement header and continue
    const settlement: PaymentSettled = {
      x402Version: 2,
      success: true,
      transaction: payload.payload.txHash,
    };

    c.header(HEADER_PAYMENT_RESPONSE, encodeHeader(settlement));

    // Store payment info in context for handlers
    c.set("x402Payment", {
      txHash: payload.payload.txHash,
      amount: routeConfig.price,
    });

    await next();

    // Emit request served event
    const latencyMs = Date.now() - requestStartTime;
    emitRequestServed(agentId, routeKey, latencyMs, payload.payload.txHash);
  };
}

/**
 * Helper to get payment info from context
 */
export function getPaymentInfo(c: Context): { txHash: Hash; amount: bigint } | undefined {
  return c.get("x402Payment");
}

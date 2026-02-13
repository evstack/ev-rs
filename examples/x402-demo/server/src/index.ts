import { Hono } from "hono";
import { cors } from "hono/cors";
import { logger } from "hono/logger";
import {
  createEvolveClient,
  createEvolveClientSync,
  getBlockNumber,
  getChainId,
  getBalance,
  type EvolveClient,
} from "./evolve.js";
import { createWalletRoutes } from "./wallet.js";
import { createPasskeyRoutes } from "./passkey.js";
import {
  createTransformRoutes,
  TRANSFORM_ROUTES,
  TREASURY_ADDRESS,
  NETWORK,
} from "./transform.js";
import { eventEmitter } from "./events.js";
import { x402ResourceServer } from "@x402/core/server";
import { EvolveFacilitatorClient } from "./x402/evolve-facilitator.js";
import { EvolveSchemeServer } from "./x402/evolve-scheme-server.js";
import { paymentMiddleware, captureAgentId, registerEventHooks } from "./x402/hono-middleware.js";

const EVOLVE_RPC_URL = process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545";

// Global client reference (initialized async)
let evolveClient: EvolveClient;

async function getBlockTxCountFromRpc(blockNumber: bigint): Promise<number | null> {
  const blockTag = `0x${blockNumber.toString(16)}`;
  const result = await callRpc<string>("eth_getBlockTransactionCountByNumber", [blockTag]);
  return result ? Number.parseInt(result, 16) : null;
}

async function callRpc<T>(method: string, params: unknown[]): Promise<T | null> {
  try {
    const response = await fetch(EVOLVE_RPC_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method,
        params,
      }),
    });
    const data = (await response.json()) as {
      result?: T;
      error?: unknown;
    };
    if (data.error || data.result === undefined) {
      return null;
    }
    return data.result;
  } catch {
    return null;
  }
}


function createApp(client: EvolveClient) {
  const app = new Hono();

  // Middleware
  app.use("*", logger());
  app.use(
    "*",
    cors({
      origin: process.env.CORS_ORIGIN ?? "*",
      allowMethods: ["GET", "POST", "OPTIONS"],
      allowHeaders: ["Content-Type", "Authorization", "PAYMENT-SIGNATURE", "X-Agent-ID"],
      exposeHeaders: ["PAYMENT-REQUIRED", "PAYMENT-RESPONSE"],
    })
  );

  // Apply X402 middleware for protected routes
  const facilitator = new EvolveFacilitatorClient(client, NETWORK);
  const resourceServer = new x402ResourceServer(facilitator);
  resourceServer.register(NETWORK, new EvolveSchemeServer());
  registerEventHooks(resourceServer);
  app.use("/api/transform/*", captureAgentId());
  app.use("/api/transform/*", paymentMiddleware(TRANSFORM_ROUTES, resourceServer));

  // Health check with chain info
  app.get("/health", async (c) => {
    try {
      const [blockNumber, chainId] = await Promise.all([
        getBlockNumber(client),
        getChainId(client),
      ]);

      return c.json({
        status: "ok",
        mode: client.mode,
        chain: {
          id: chainId,
          blockNumber: blockNumber.toString(),
        },
        x402: {
          treasury: TREASURY_ADDRESS,
          network: NETWORK,
          asset: "native",
        },
        timestamp: new Date().toISOString(),
      });
    } catch (err) {
      console.error("Health check failed:", err);
      return c.json(
        {
          status: "error",
          error: "Cannot connect to Evolve node",
          rpcUrl: process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545",
        },
        503
      );
    }
  });

  // Mount routes
  app.route("/auth", createPasskeyRoutes());
  app.route("/wallet", createWalletRoutes(client));
  app.route("/api/transform", createTransformRoutes());

  // Pricing info endpoint
  app.get("/api/pricing", (c) => {
    const routes = TRANSFORM_ROUTES as Record<string, { accepts: { price: string }; description?: string }>;
    const pricing = Object.entries(routes).map(([route, config]) => ({
      route,
      price: String(config.accepts.price),
      description: config.description ?? "",
    }));

    return c.json({
      treasury: TREASURY_ADDRESS,
      network: NETWORK,
      asset: "native",
      endpoints: pricing,
    });
  });

  // Events endpoint - returns recent events and metrics (for non-WebSocket clients)
  app.get("/api/events", (c) => {
    const metrics = eventEmitter.getMetrics();
    return c.json({
      events: eventEmitter.getRecentEvents(100),
      metrics: {
        totalPayments: metrics.totalPayments,
        successfulPayments: metrics.successfulPayments,
        failedPayments: metrics.failedPayments,
        totalRequests: metrics.totalRequests,
        uniqueAgents: metrics.uniqueAgents.size,
      },
      wsConnections: eventEmitter.getConnectionCount(),
    });
  });

  // Chain stats endpoint for dashboard widgets
  app.get("/api/chain", async (c) => {
    try {
      const [blockNumber, chainId] = await Promise.all([
        getBlockNumber(client),
        getChainId(client),
      ]);
      const metrics = eventEmitter.getMetrics();

      let latestBlockTxCount = await getBlockTxCountFromRpc(blockNumber);
      let latestBlockTimestamp: string | null = null;

      try {
        const block = await client.public.getBlock({
          blockNumber,
          includeTransactions: false,
        });
        if (latestBlockTxCount === null) {
          latestBlockTxCount = block.transactions.length;
        }
        latestBlockTimestamp = new Date(Number(block.timestamp) * 1000).toISOString();
      } catch (err) {
        // Some dev RPC implementations may not support all block fields.
        console.warn("Failed to fetch latest block details:", err);
      }

      return c.json({
        chainId,
        blockNumber: blockNumber.toString(),
        latestBlockTxCount,
        latestBlockTimestamp,
        observedPaymentTxs: metrics.totalPayments,
        observedServedRequests: metrics.totalRequests,
      });
    } catch (err) {
      console.error("Failed to fetch chain stats:", err);
      return c.json(
        { error: "Failed to fetch chain stats from ev-node" },
        503
      );
    }
  });

  // Treasury balance endpoint
  app.get("/api/treasury", async (c) => {
    try {
      const balance = await getBalance(client, TREASURY_ADDRESS);
      return c.json({
        address: TREASURY_ADDRESS,
        balance: balance.toString(),
      });
    } catch (err) {
      return c.json({ error: "Failed to get treasury balance" }, 500);
    }
  });

  return app;
}

async function main() {
  const port = parseInt(process.env.PORT ?? "3000", 10);

  console.log(`Starting x402-demo server on port ${port}`);
  console.log(`Evolve RPC: ${process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545"}`);
  if (process.env.EVOLVE_GRPC_URL) {
    console.log(`Evolve gRPC: ${process.env.EVOLVE_GRPC_URL}`);
  }
  console.log(`Treasury: ${TREASURY_ADDRESS}`);
  console.log(`Network: ${NETWORK}`);

  // Initialize Evolve client (async for gRPC support)
  try {
    evolveClient = await createEvolveClient();
    console.log(`Client mode: ${evolveClient.mode}`);
  } catch (err) {
    console.warn("Failed to create async client, using sync JSON-RPC client:", err);
    evolveClient = createEvolveClientSync();
  }

  const app = createApp(evolveClient);

  // Bun server with WebSocket support
  const server = Bun.serve({
    port,
    fetch(req, server) {
      const url = new URL(req.url);

      // Handle WebSocket upgrade for /ws/events
      if (url.pathname === "/ws/events") {
        const upgraded = server.upgrade(req);
        if (upgraded) {
          return undefined; // Bun handles the response
        }
        return new Response("WebSocket upgrade failed", { status: 500 });
      }

      // Handle all other requests with Hono
      return app.fetch(req);
    },
    websocket: {
      open(ws) {
        console.log("WebSocket client connected");
        eventEmitter.addConnection(ws as unknown as WebSocket);
      },
      close(ws) {
        console.log("WebSocket client disconnected");
        eventEmitter.removeConnection(ws as unknown as WebSocket);
      },
      message(ws, message) {
        // Handle incoming messages (e.g., ping/pong)
        try {
          const data = JSON.parse(message.toString());
          if (data.type === "ping") {
            ws.send(JSON.stringify({ type: "pong", timestamp: Date.now() }));
          }
        } catch {
          // Ignore invalid messages
        }
      },
    },
  });

  console.log(`Server running at http://localhost:${server.port}`);
  console.log(`WebSocket events at ws://localhost:${server.port}/ws/events`);
}

main().catch(console.error);

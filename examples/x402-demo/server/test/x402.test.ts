import { describe, it, expect } from "bun:test";
import { Hono } from "hono";
import type { EvolveClient } from "../src/evolve.js";
import type { Address } from "viem";
import type { Network } from "@x402/core/types";
import type { RoutesConfig } from "@x402/core/http";
import { x402ResourceServer } from "@x402/core/server";
import { EvolveFacilitatorClient } from "../src/x402/evolve-facilitator.js";
import { EvolveSchemeServer } from "../src/x402/evolve-scheme-server.js";
import { paymentMiddleware } from "../src/x402/hono-middleware.js";

const TEST_NETWORK: Network = "evolve:1337";
const TEST_PAY_TO: Address = "0x0000000000000000000000000000000000000001";

const TEST_ROUTES: RoutesConfig = {
  "POST /api/paid": {
    accepts: {
      scheme: "exact",
      payTo: TEST_PAY_TO,
      price: "100",
      network: TEST_NETWORK,
    },
    description: "Test",
    mimeType: "application/json",
  },
};

function createMockClient(receiptStatus: "success" | "reverted" | null): EvolveClient {
  return {
    public: {
      getTransactionReceipt: async () =>
        receiptStatus === null ? null : { status: receiptStatus },
    },
  } as unknown as EvolveClient;
}

function createTestApp(client: EvolveClient) {
  const app = new Hono();
  const facilitator = new EvolveFacilitatorClient(client, TEST_NETWORK);
  const resourceServer = new x402ResourceServer(facilitator);
  resourceServer.register(TEST_NETWORK, new EvolveSchemeServer());
  app.use("/api/*", paymentMiddleware(TEST_ROUTES, resourceServer));
  app.post("/api/paid", (c) => c.json({ ok: true }));
  app.get("/api/free", (c) => c.json({ ok: true }));
  return app;
}

function encodePayment(
  txHash: string,
  resource = { url: "http://localhost/api/paid", description: "Test", mimeType: "application/json" },
  accepted = {
    scheme: "exact",
    network: TEST_NETWORK,
    asset: "native",
    amount: "100",
    payTo: TEST_PAY_TO,
    maxTimeoutSeconds: 300,
    extra: {},
  },
) {
  return Buffer.from(
    JSON.stringify({
      x402Version: 2,
      resource,
      accepted,
      payload: { txHash },
    })
  ).toString("base64");
}

describe("X402 Payment Flow", () => {
  it("returns 402 with payment requirements for protected route", async () => {
    const app = createTestApp(createMockClient("success"));
    const res = await app.request("/api/paid", { method: "POST" });

    expect(res.status).toBe(402);
    const paymentHeader = res.headers.get("PAYMENT-REQUIRED");
    expect(paymentHeader).toBeTruthy();

    // Verify the payment requirement header is valid base64 JSON with required fields
    const decoded = JSON.parse(Buffer.from(paymentHeader!, "base64").toString());
    expect(decoded.accepts).toBeDefined();
    expect(decoded.accepts.length).toBeGreaterThan(0);
    expect(decoded.accepts[0].payTo).toBeDefined();
    expect(decoded.accepts[0].network).toBe(TEST_NETWORK);
  });

  it("allows access with valid payment proof", async () => {
    const app = createTestApp(createMockClient("success"));
    const txHash = "0x" + "a".repeat(64);

    const res = await app.request("/api/paid", {
      method: "POST",
      headers: { "PAYMENT-SIGNATURE": encodePayment(txHash) },
    });

    expect(res.status).toBe(200);
    expect(res.headers.get("PAYMENT-RESPONSE")).toBeTruthy();
    const body = await res.json();
    expect(body).toEqual({ ok: true });
  });

  it("rejects when transaction not found", async () => {
    const app = createTestApp(createMockClient(null));

    const res = await app.request("/api/paid", {
      method: "POST",
      headers: { "PAYMENT-SIGNATURE": encodePayment("0x" + "b".repeat(64)) },
    });

    expect(res.status).toBe(402);
    const paymentHeader = res.headers.get("PAYMENT-REQUIRED");
    expect(paymentHeader).toBeTruthy();
  });

  it("rejects reused transaction (replay protection)", async () => {
    const app = createTestApp(createMockClient("success"));
    const txHash = "0x" + "c".repeat(64);
    const headers = { "PAYMENT-SIGNATURE": encodePayment(txHash) };

    const first = await app.request("/api/paid", { method: "POST", headers });
    expect(first.status).toBe(200);

    const res = await app.request("/api/paid", { method: "POST", headers });
    expect(res.status).toBe(402);
    const paymentHeader = res.headers.get("PAYMENT-REQUIRED");
    expect(paymentHeader).toBeTruthy();
  });

  it("passes through unprotected routes", async () => {
    const app = createTestApp(createMockClient("success"));
    const res = await app.request("/api/free");
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body).toEqual({ ok: true });
  });
});

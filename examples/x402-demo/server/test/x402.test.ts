import { describe, it, expect } from "bun:test";
import { Hono } from "hono";
import { x402Middleware, type RouteConfig } from "../src/x402.js";
import type { EvolveClient } from "../src/evolve.js";
import type { Address, Hash } from "viem";

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
  app.use(
    "/api/*",
    x402Middleware({
      routes: { "POST /api/paid": { price: 100n, description: "Test" } },
      payTo: "0x0000000000000000000000000000000000000001" as Address,
      network: "evolve:1337",
      asset: "native",
      client,
    })
  );
  app.post("/api/paid", (c) => c.json({ ok: true }));
  app.get("/api/free", (c) => c.json({ ok: true }));
  return app;
}

function encodePayment(txHash: string, version = 2) {
  return Buffer.from(
    JSON.stringify({ x402Version: version, payload: { txHash } })
  ).toString("base64");
}

describe("X402 Payment Flow", () => {
  it("returns 402 with payment requirements for protected route", async () => {
    const app = createTestApp(createMockClient("success"));
    const res = await app.request("/api/paid", { method: "POST" });

    expect(res.status).toBe(402);
    expect(res.headers.get("PAYMENT-REQUIRED")).toBeTruthy();

    const body = await res.json();
    expect(body.price).toBe("100");
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
  });

  it("rejects when transaction not found", async () => {
    const app = createTestApp(createMockClient(null));

    const res = await app.request("/api/paid", {
      method: "POST",
      headers: { "PAYMENT-SIGNATURE": encodePayment("0x" + "b".repeat(64)) },
    });

    expect(res.status).toBe(402);
    expect((await res.json()).reason).toBe("Transaction not found");
  });

  it("rejects reused transaction (replay protection)", async () => {
    const app = createTestApp(createMockClient("success"));
    const txHash = "0x" + "c".repeat(64);
    const headers = { "PAYMENT-SIGNATURE": encodePayment(txHash) };

    await app.request("/api/paid", { method: "POST", headers });
    const res = await app.request("/api/paid", { method: "POST", headers });

    expect(res.status).toBe(402);
    expect((await res.json()).reason).toBe("Transaction already used");
  });

  it("passes through unprotected routes", async () => {
    const app = createTestApp(createMockClient("success"));
    const res = await app.request("/api/free");
    expect(res.status).toBe(200);
  });
});

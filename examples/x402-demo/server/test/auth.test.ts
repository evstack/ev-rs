import { describe, it, expect } from "bun:test";
import { Hono } from "hono";
import { createPasskeyRoutes, authMiddleware } from "../src/passkey.js";

function createTestApp() {
  const app = new Hono();
  app.route("/auth", createPasskeyRoutes());
  app.get("/protected", authMiddleware(), (c) => c.json({ ok: true }));
  return app;
}

describe("Authentication", () => {
  it("generates WebAuthn registration options", async () => {
    const app = createTestApp();

    const res = await app.request("/auth/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "testuser" }),
    });

    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.options.challenge).toBeTruthy();
    expect(body.options.rp.id).toBe("localhost");
    expect(body.userId).toBeTruthy();
  });

  it("generates WebAuthn login options", async () => {
    const app = createTestApp();

    const res = await app.request("/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });

    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.options.challenge).toBeTruthy();
    expect(body.authId).toBeTruthy();
  });

  it("blocks protected routes without valid session", async () => {
    const app = createTestApp();

    const noAuth = await app.request("/protected");
    expect(noAuth.status).toBe(401);

    const badAuth = await app.request("/protected", {
      headers: { Authorization: "Bearer invalid" },
    });
    expect(badAuth.status).toBe(401);
  });
});

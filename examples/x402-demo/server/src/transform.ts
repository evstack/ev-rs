import { Hono } from "hono";
import type { Address } from "viem";
import type { Network } from "@x402/core/types";
import type { RouteConfig, RoutesConfig } from "@x402/core/http";

// Evolve payment configuration
export const TREASURY_ADDRESS: Address =
  (process.env.TREASURY_ADDRESS ??
    "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC") as Address;
export const NETWORK: Network =
  (process.env.EVOLVE_NETWORK ?? "evolve:1") as Network;

type TransformRequest = {
  input: string;
};

type TransformResponse = {
  output: string;
  operation: string;
};

type Variables = {
  transformInput: string;
};

export function createTransformRoutes() {
  const app = new Hono<{ Variables: Variables }>();

  // Validate input middleware
  app.use("*", async (c, next) => {
    if (c.req.method !== "POST") {
      return next();
    }

    try {
      const body = await c.req.json<TransformRequest>();
      if (typeof body.input !== "string") {
        return c.json({ error: "input must be a string" }, 400);
      }
      if (body.input.length > 10000) {
        return c.json({ error: "input too long (max 10000 chars)" }, 400);
      }
      c.set("transformInput", body.input);
      return next();
    } catch {
      return c.json({ error: "invalid JSON body" }, 400);
    }
  });

  /**
   * POST /api/transform/echo
   * Returns input unchanged - 100 tokens
   */
  app.post("/echo", (c) => {
    const input = c.get("transformInput");
    return c.json<TransformResponse>({ output: input, operation: "echo" });
  });

  /**
   * POST /api/transform/reverse
   * Reverses input string - 100 tokens
   */
  app.post("/reverse", (c) => {
    const input = c.get("transformInput");
    return c.json<TransformResponse>({ output: input.split("").reverse().join(""), operation: "reverse" });
  });

  /**
   * POST /api/transform/uppercase
   * Uppercases input string - 100 tokens
   */
  app.post("/uppercase", (c) => {
    const input = c.get("transformInput");
    return c.json<TransformResponse>({ output: input.toUpperCase(), operation: "uppercase" });
  });

  /**
   * POST /api/transform/hash
   * Returns SHA256 of input - 200 tokens
   */
  app.post("/hash", async (c) => {
    const input = c.get("transformInput");
    const hashBuffer = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(input));
    const hashHex = Array.from(new Uint8Array(hashBuffer)).map((b) => b.toString(16).padStart(2, "0")).join("");
    return c.json<TransformResponse>({ output: `0x${hashHex}`, operation: "hash" });
  });

  return app;
}

// Route configurations for X402 middleware (@x402/core format)
function route(price: string, description: string): RouteConfig {
  return {
    accepts: { scheme: "exact", payTo: TREASURY_ADDRESS, price, network: NETWORK },
    description,
    mimeType: "application/json",
  };
}

export const TRANSFORM_ROUTES: RoutesConfig = {
  "POST /api/transform/echo": route("100", "Echo - returns input unchanged"),
  "POST /api/transform/reverse": route("100", "Reverse - reverses input string"),
  "POST /api/transform/uppercase": route("100", "Uppercase - uppercases input string"),
  "POST /api/transform/hash": route("200", "Hash - returns SHA256 of input"),
};

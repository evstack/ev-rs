import { Hono } from "hono";
import { getPaymentInfo } from "./x402.js";

type TransformRequest = {
  input: string;
};

type TransformResponse = {
  output: string;
  operation: string;
  cost: string;
  txHash?: string;
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
    const payment = getPaymentInfo(c);

    const response: TransformResponse = {
      output: input,
      operation: "echo",
      cost: payment?.amount.toString() ?? "0",
      txHash: payment?.txHash,
    };

    return c.json(response);
  });

  /**
   * POST /api/transform/reverse
   * Reverses input string - 100 tokens
   */
  app.post("/reverse", (c) => {
    const input = c.get("transformInput");
    const payment = getPaymentInfo(c);

    const response: TransformResponse = {
      output: input.split("").reverse().join(""),
      operation: "reverse",
      cost: payment?.amount.toString() ?? "0",
      txHash: payment?.txHash,
    };

    return c.json(response);
  });

  /**
   * POST /api/transform/uppercase
   * Uppercases input string - 100 tokens
   */
  app.post("/uppercase", (c) => {
    const input = c.get("transformInput");
    const payment = getPaymentInfo(c);

    const response: TransformResponse = {
      output: input.toUpperCase(),
      operation: "uppercase",
      cost: payment?.amount.toString() ?? "0",
      txHash: payment?.txHash,
    };

    return c.json(response);
  });

  /**
   * POST /api/transform/hash
   * Returns SHA256 of input - 200 tokens
   */
  app.post("/hash", async (c) => {
    const input = c.get("transformInput");
    const payment = getPaymentInfo(c);

    // Use Web Crypto API (available in Bun)
    const encoder = new TextEncoder();
    const data = encoder.encode(input);
    const hashBuffer = await crypto.subtle.digest("SHA-256", data);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    const hashHex = hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");

    const response: TransformResponse = {
      output: `0x${hashHex}`,
      operation: "hash",
      cost: payment?.amount.toString() ?? "0",
      txHash: payment?.txHash,
    };

    return c.json(response);
  });

  return app;
}

// Route configurations for X402 middleware
export const TRANSFORM_ROUTES = {
  "POST /api/transform/echo": {
    price: 100n,
    description: "Echo - returns input unchanged",
  },
  "POST /api/transform/reverse": {
    price: 100n,
    description: "Reverse - reverses input string",
  },
  "POST /api/transform/uppercase": {
    price: 100n,
    description: "Uppercase - uppercases input string",
  },
  "POST /api/transform/hash": {
    price: 200n,
    description: "Hash - returns SHA256 of input",
  },
};

import { Hono } from "hono";
import {
  type Address,
  type Hash,
  type Hex,
  isAddress,
  formatEther,
  parseEther,
  createWalletClient,
  http,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { type EvolveClient, getBalance, getNonce, evolveTestnet } from "./evolve.js";
import { authMiddleware, getUserPrivateKey, type UserRecord } from "./passkey.js";

// Faucet configuration
const FAUCET_PRIVATE_KEY = process.env.FAUCET_PRIVATE_KEY as Hex | undefined;
const FAUCET_AMOUNT = parseEther(process.env.FAUCET_AMOUNT ?? "1000");

type Variables = {
  user: UserRecord;
};

export function createWalletRoutes(client: EvolveClient) {
  const app = new Hono<{ Variables: Variables }>();

  /**
   * GET /wallet/balance?address=0x...
   * Returns balance for a given address (no auth required for demo)
   */
  app.get("/balance", async (c) => {
    const address = c.req.query("address");

    if (!address) {
      return c.json({ error: "address query parameter required" }, 400);
    }

    if (!isAddress(address)) {
      return c.json({ error: "invalid address format" }, 400);
    }

    try {
      const balance = await getBalance(client, address as Address);
      const nonce = await getNonce(client, address as Address);

      return c.json({
        address,
        balance: balance.toString(),
        balanceFormatted: formatEther(balance),
        nonce,
      });
    } catch (err) {
      console.error("Balance query failed:", err);
      return c.json({ error: "failed to query balance" }, 500);
    }
  });

  /**
   * GET /wallet/me
   * Returns balance for authenticated user's address
   */
  app.get("/me", authMiddleware(), async (c) => {
    const user = c.get("user");

    try {
      const balance = await getBalance(client, user.address);
      const nonce = await getNonce(client, user.address);

      return c.json({
        address: user.address,
        username: user.username,
        balance: balance.toString(),
        balanceFormatted: formatEther(balance),
        nonce,
      });
    } catch (err) {
      console.error("Balance query failed:", err);
      return c.json({ error: "failed to query balance" }, 500);
    }
  });

  /**
   * POST /wallet/faucet
   * Mint demo tokens to authenticated user's address
   */
  app.post("/faucet", authMiddleware(), async (c) => {
    const user = c.get("user");

    if (!FAUCET_PRIVATE_KEY) {
      return c.json({
        error: "Faucet not configured",
        message: "Set FAUCET_PRIVATE_KEY environment variable",
      }, 501);
    }

    try {
      const faucetAccount = privateKeyToAccount(FAUCET_PRIVATE_KEY);
      const walletClient = createWalletClient({
        account: faucetAccount,
        chain: evolveTestnet,
        transport: http(process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545"),
      });

      const txHash = await walletClient.sendTransaction({
        to: user.address,
        value: FAUCET_AMOUNT,
      });

      // Wait briefly for tx to be included
      await new Promise((resolve) => setTimeout(resolve, 1000));

      const newBalance = await getBalance(client, user.address);

      return c.json({
        success: true,
        txHash,
        amount: FAUCET_AMOUNT.toString(),
        amountFormatted: formatEther(FAUCET_AMOUNT),
        newBalance: newBalance.toString(),
        newBalanceFormatted: formatEther(newBalance),
      });
    } catch (err) {
      console.error("Faucet transfer failed:", err);
      return c.json({ error: "Faucet transfer failed" }, 500);
    }
  });

  /**
   * POST /wallet/transfer
   * Transfer tokens from authenticated user's wallet
   * Body: { to: string, amount: string }
   */
  app.post("/transfer", authMiddleware(), async (c) => {
    const user = c.get("user");
    const body = await c.req.json<{ to?: string; amount?: string }>();

    if (!body.to || !isAddress(body.to)) {
      return c.json({ error: "Valid 'to' address required" }, 400);
    }

    if (!body.amount) {
      return c.json({ error: "'amount' required" }, 400);
    }

    let amountWei: bigint;
    try {
      // Accept both wei (raw bigint string) and ether (decimal string)
      if (body.amount.includes(".")) {
        amountWei = parseEther(body.amount);
      } else {
        amountWei = BigInt(body.amount);
      }
    } catch {
      return c.json({ error: "Invalid amount format" }, 400);
    }

    if (amountWei <= 0n) {
      return c.json({ error: "Amount must be positive" }, 400);
    }

    try {
      // Check balance
      const balance = await getBalance(client, user.address);
      if (balance < amountWei) {
        return c.json({
          error: "Insufficient balance",
          balance: balance.toString(),
          required: amountWei.toString(),
        }, 400);
      }

      // Create wallet client with user's server-managed key
      const privateKey = getUserPrivateKey(user);
      const account = privateKeyToAccount(privateKey);
      const walletClient = createWalletClient({
        account,
        chain: evolveTestnet,
        transport: http(process.env.EVOLVE_RPC_URL ?? "http://127.0.0.1:8545"),
      });

      // Send transaction
      const txHash = await walletClient.sendTransaction({
        to: body.to as Address,
        value: amountWei,
      });

      return c.json({
        success: true,
        txHash,
        from: user.address,
        to: body.to,
        amount: amountWei.toString(),
        amountFormatted: formatEther(amountWei),
      });
    } catch (err) {
      console.error("Transfer failed:", err);
      return c.json({ error: "Transfer failed" }, 500);
    }
  });

  return app;
}

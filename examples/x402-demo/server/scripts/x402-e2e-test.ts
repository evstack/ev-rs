#!/usr/bin/env bun
/**
 * X402 E2E Test — runs against an already-running stack (evd + evgrpc + local-da).
 *
 * This script only starts the API server and runs the X402 payment flow.
 * All accounts except the faucet are generated dynamically.
 *
 * Prerequisites (must be running):
 *   - evd (gRPC :50051, JSON-RPC :8545)
 *   - local-da (:7980)
 *   - evgrpc (consensus)
 *
 * Usage:
 *   bun run scripts/x402-e2e-test.ts
 */
import { spawn, type Subprocess } from "bun";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  DEFAULT_RPC_URL,
  DEFAULT_API_URL,
  FAUCET_PRIVATE_KEY,
  createEvolveChain,
  addressToAccountId,
  waitForRpc,
  waitForApi,
  getRpcChainId,
  decodePaymentRequired,
  expectStatus,
  createTestClients,
  encodePaymentSignature,
  submitPaymentTransaction,
  generateRandomAccount,
  fundAccount,
} from "./x402-e2e-utils.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const serverDir = resolve(__dirname, "..");

function pipeOutput(proc: Subprocess, prefix: string): void {
  proc.stdout?.pipeTo(
    new WritableStream({
      write(chunk) {
        const text = Buffer.from(chunk).toString();
        for (const line of text.split("\n")) {
          if (line.trim()) console.log(`[${prefix}] ${line}`);
        }
      },
    })
  );
  proc.stderr?.pipeTo(
    new WritableStream({
      write(chunk) {
        const text = Buffer.from(chunk).toString();
        for (const line of text.split("\n")) {
          if (line.trim()) console.error(`[${prefix}] ${line}`);
        }
      },
    })
  );
}

async function main() {
  const processes: Subprocess[] = [];

  const cleanup = () => {
    console.log("\nCleaning up processes...");
    for (const proc of processes) {
      try {
        proc.kill();
      } catch {
        // ignore
      }
    }
  };

  process.on("SIGINT", cleanup);
  process.on("SIGTERM", cleanup);

  try {
    // 1. Verify the stack is running
    console.log("Checking that evd is running...");
    await waitForRpc(DEFAULT_RPC_URL, 5_000);
    console.log("evd is ready!");

    // 2. Generate dynamic accounts
    const treasury = generateRandomAccount();
    const payer = generateRandomAccount();
    console.log(`  Treasury (dynamic): ${treasury.address}`);
    console.log(`  Payer (dynamic):    ${payer.address}`);

    // 3. Start API server with dynamic treasury
    console.log("Starting API server...");
    const apiProc = spawn({
      cmd: ["bun", "run", "src/index.ts"],
      cwd: serverDir,
      env: {
        ...process.env,
        EVOLVE_RPC_URL: DEFAULT_RPC_URL,
        TREASURY_ADDRESS: treasury.address,
      },
      stdout: "pipe",
      stderr: "pipe",
    });
    processes.push(apiProc);
    pipeOutput(apiProc, "api");

    console.log("Waiting for API server...");
    await waitForApi();
    console.log("API server is ready!");

    // 4. Get chain ID and create clients
    const chainId = await getRpcChainId();
    console.log(`  Chain ID: ${chainId}`);

    const evolveChain = createEvolveChain(chainId);
    const faucetClients = createTestClients(evolveChain, DEFAULT_RPC_URL, FAUCET_PRIVATE_KEY);
    const payerClients = createTestClients(evolveChain, DEFAULT_RPC_URL, payer.privateKey);

    // 5. Fund the payer from faucet
    const payerAccountId = addressToAccountId(payer.address as `0x${string}`);
    const fundingAmount = 10_000_000n;
    console.log(`\nStep 0: Funding payer (${payerAccountId}) with ${fundingAmount} tokens...`);
    const fundTxHash = await fundAccount(
      faucetClients.walletClient,
      faucetClients.publicClient,
      payerAccountId,
      fundingAmount,
    );
    console.log(`  Funding tx confirmed: ${fundTxHash}`);

    // 6. Run the X402 E2E test
    console.log("\n=== Starting X402 E2E Test ===\n");

    console.log("Step 1: Request protected endpoint (expect 402)...");
    const initialRes = await fetch(`${DEFAULT_API_URL}/api/transform/reverse`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ input: "hello" }),
    });

    await expectStatus(initialRes, 402, "initial 402");
    console.log("  Got 402 as expected.");

    const paymentHeader = initialRes.headers.get("PAYMENT-REQUIRED");
    if (!paymentHeader) {
      throw new Error("Missing PAYMENT-REQUIRED header");
    }

    const paymentRequired = decodePaymentRequired(paymentHeader) as {
      accepts: Array<{ amount: string; payTo: `0x${string}`; network: string }>;
    };

    const amount = BigInt(paymentRequired.accepts[0].amount);
    const payTo = paymentRequired.accepts[0].payTo;
    const network = paymentRequired.accepts[0].network;
    const payToAccountId = addressToAccountId(payTo);

    console.log(`  Payment required: ${amount} to ${payTo} (AccountId: ${payToAccountId})`);
    console.log(`  Network: ${network}`);

    // 7. Payer submits payment transaction (auto-registers as EOA on first tx)
    console.log(`\nStep 2: Payer (${payerClients.account.address}) submitting payment...`);
    const txHash = await submitPaymentTransaction(
      payerClients.walletClient,
      payerClients.publicClient,
      payToAccountId,
      amount
    );

    if (!txHash) {
      throw new Error("Unable to submit a successful payment transaction");
    }
    console.log(`  Payment tx confirmed: ${txHash}`);

    // 8. Retry with payment proof
    console.log("\nStep 3: Retrying with PAYMENT-SIGNATURE...");
    const paymentSignature = encodePaymentSignature({
      x402Version: 2,
      scheme: "exact",
      network,
      payload: { txHash },
    });

    const finalRes = await fetch(`${DEFAULT_API_URL}/api/transform/reverse`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "PAYMENT-SIGNATURE": paymentSignature,
      },
      body: JSON.stringify({ input: "hello" }),
    });

    await expectStatus(finalRes, 200, "paid 200");

    const body = await finalRes.json();
    if (body.output !== "olleh") {
      throw new Error(`Unexpected output: ${JSON.stringify(body)}`);
    }

    console.log(`  Got 200 with output: "${body.output}"`);
    console.log("\n=== X402 E2E flow passed! ===\n");
  } finally {
    cleanup();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

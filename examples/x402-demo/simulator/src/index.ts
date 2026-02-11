#!/usr/bin/env bun
import { program } from "commander";
import { AgentPool } from "./pool.js";
import { createPoolConfig, validateConfig } from "./config.js";

program
  .name("x402-simulator")
  .description("X402 payment protocol agent simulator")
  .version("0.1.0")
  .option("-a, --agents <number>", "Number of agents to spawn", "10")
  .option("-r, --rps <number>", "Target requests per second", "10")
  .option("-s, --server <url>", "X402 server URL", "http://localhost:3000")
  .option("-e, --evolve-rpc <url>", "Evolve RPC URL", "http://localhost:8545")
  .option(
    "-f, --funding <amount>",
    "Funding amount per agent (token units, 6 decimals)",
    "1000000"
  )
  .option("-k, --faucet-key <key>", "Faucet private key (or set FAUCET_PRIVATE_KEY)")
  .option("-d, --duration <seconds>", "Run duration in seconds (0 = infinite)", "0")
  .parse();

const opts = program.opts();

async function main() {
  console.log("=== X402 Agent Simulator ===\n");

  // Create config from CLI options
  const config = createPoolConfig({
    agents: parseInt(opts.agents, 10),
    rps: parseInt(opts.rps, 10),
    server: opts.server,
    evolveRpc: opts.evolveRpc,
    funding: opts.funding,
    faucetKey: opts.faucetKey,
    duration: opts.duration ? parseInt(opts.duration, 10) : undefined,
  });

  // Validate
  const errors = validateConfig(config);
  if (errors.length > 0) {
    console.error("Configuration errors:");
    errors.forEach((e) => console.error(`  - ${e}`));
    process.exit(1);
  }

  console.log("Configuration:");
  console.log(`  Agents: ${config.agentCount}`);
  console.log(`  Target RPS: ${config.requestsPerSecond}`);
  console.log(`  Server: ${config.serverUrl}`);
  console.log(`  Evolve RPC: ${config.evolveRpcUrl}`);
  console.log(`  Funding per agent: ${config.fundingAmount} tokens (6 decimals)`);
  console.log("");

  // Create and initialize pool
  const pool = new AgentPool(config);

  // Handle graceful shutdown
  let shuttingDown = false;
  const shutdown = async () => {
    if (shuttingDown) return;
    shuttingDown = true;
    console.log("\nShutting down...");
    await pool.stop();
    process.exit(0);
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  try {
    await pool.initialize();
    await pool.start();

    // Run for duration if specified
    const duration = parseInt(opts.duration, 10);
    if (duration > 0) {
      console.log(`Running for ${duration} seconds...`);
      await new Promise((resolve) => setTimeout(resolve, duration * 1000));
      await shutdown();
    } else {
      console.log("Running until interrupted (Ctrl+C to stop)...");
    }
  } catch (err) {
    console.error("Fatal error:", err);
    process.exit(1);
  }
}

main();

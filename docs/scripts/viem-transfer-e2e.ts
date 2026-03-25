import assert from "node:assert/strict";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { Socket } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inspect } from "node:util";

import {
  bytesToHex,
  type Address,
  type Hex,
  concatHex,
  createPublicClient,
  createWalletClient,
  defineChain,
  getAddress,
  hexToBytes,
  http,
  keccak256,
  parseGwei,
  stringToHex,
} from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..", "..");
const TESTAPP_BINARY = resolve(REPO_ROOT, "target", "debug", "testapp");
const TOKEN_ADDRESS_BINARY = resolve(
  REPO_ROOT,
  "target",
  "debug",
  "examples",
  "print_token_address"
);

const QUIET = process.argv.includes("--quiet");
const KEEP_TEMP = process.argv.includes("--keep-temp");

const CHAIN_ID = 1;
const RPC_READY_TIMEOUT_MS = 120_000;
const RPC_POLL_INTERVAL_MS = 1_000;
const NODE_SHUTDOWN_TIMEOUT_MS = 10_000;
const RPC_PORT_CANDIDATES = [18545, 28545, 38545, 48545];

const INITIAL_SENDER_BALANCE = 1_000n;
const INITIAL_RECIPIENT_BALANCE = 1n;
const TRANSFER_AMOUNT = 100n;
const GAS_LIMIT = 150_000n;

const DOMAIN_EOA_ETH_V1 = "eoa:eth:v1";
const DOMAIN_CONTRACT_ADDR_RUNTIME_V1 = "contract:addr:runtime:v1";
const TRANSFER_SELECTOR = hexToBytes(keccak256(stringToHex("transfer"))).subarray(0, 4);

type CargoResult = {
  stdout: string;
  stderr: string;
  combined: string;
};

type CommandResult = CargoResult;

type NodeHandle = {
  process: ChildProcess;
  getLogs: () => string;
  getSpawnError: () => Error | undefined;
};

function log(message: string): void {
  if (!QUIET) {
    console.log(message);
  }
}

function runCargo(args: string[], label: string): CargoResult {
  return runCommand("cargo", args, label);
}

function runCommand(command: string, args: string[], label: string): CommandResult {
  const result = spawnSync(command, args, {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });

  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  const combined = `${stdout}${stderr}`;

  if (result.error) {
    throw new Error(
      `${label} failed to start\n${result.error.message}\n${combined.trim() || "(no output)"}`
    );
  }

  if (result.status !== 0) {
    throw new Error(
      `${label} failed with exit code ${result.status ?? "unknown"}\n` +
        `${combined.trim() || "(no output)"}`
    );
  }

  return { stdout, stderr, combined };
}

function buildGenesisFile(path: string, sender: Address, recipient: Address): void {
  const contents = {
    token: {
      name: "Evolve",
      symbol: "EV",
      decimals: 6,
      icon_url: "https://example.com/token.png",
      description: "Viem demo token",
    },
    minter_id: 100_002,
    accounts: [
      {
        eth_address: sender,
        balance: INITIAL_SENDER_BALANCE.toString(),
      },
      {
        eth_address: recipient,
        balance: INITIAL_RECIPIENT_BALANCE.toString(),
      },
    ],
  };

  writeFileSync(path, JSON.stringify(contents, null, 2));
}

function createChain(rpcUrl: string) {
  return defineChain({
    id: CHAIN_ID,
    name: "Evolve Local Demo",
    nativeCurrency: {
      name: "Evolve",
      symbol: "EV",
      decimals: 6,
    },
    rpcUrls: {
      default: {
        http: [rpcUrl],
      },
    },
  });
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const totalLength = parts.reduce((sum, part) => sum + part.length, 0);
  const result = new Uint8Array(totalLength);

  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }

  return result;
}

function encodeU128Le(value: bigint): Uint8Array {
  assert(value >= 0n, "u128 cannot be negative");
  assert(value < 1n << 128n, "u128 overflow");

  const result = new Uint8Array(16);
  let current = value;

  for (let index = 0; index < result.length; index += 1) {
    result[index] = Number(current & 0xffn);
    current >>= 8n;
  }

  return result;
}

function deriveEthEoaAccountId(address: Address): Uint8Array {
  const input = concatHex([stringToHex(DOMAIN_EOA_ETH_V1), address]);
  return hexToBytes(keccak256(input));
}

function encodeTransferCalldata(recipientAddress: Address, amount: bigint): Hex {
  const recipientAccountId = deriveEthEoaAccountId(recipientAddress);
  const calldata = concatBytes(TRANSFER_SELECTOR, recipientAccountId, encodeU128Le(amount));
  return bytesToHex(calldata);
}

function deriveRuntimeContractAddress(accountId: Uint8Array): Address {
  const input = concatHex([
    stringToHex(DOMAIN_CONTRACT_ADDR_RUNTIME_V1),
    bytesToHex(accountId),
  ]);
  const digest = hexToBytes(keccak256(input));
  return getAddress(bytesToHex(digest.subarray(12)));
}

function parseTokenAddressFromInitOutput(output: string): Address {
  const match = output.match(/atom:\s*AccountId\(\[([^\]]+)\]\)/m);
  if (!match) {
    throw new Error(`failed to parse token account id from init output\n${output.trim()}`);
  }

  const bytes = match[1]
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
    .map((value) => Number.parseInt(value, 10));

  assert.equal(bytes.length, 32, "token account id must have 32 bytes");
  assert(
    bytes.every((value) => Number.isInteger(value) && value >= 0 && value <= 255),
    "token account id bytes must be valid u8 values"
  );

  return deriveRuntimeContractAddress(Uint8Array.from(bytes));
}

function startNode(args: string[]): NodeHandle {
  const child = spawn(TESTAPP_BINARY, args, {
    cwd: REPO_ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });

  let logs = "";
  let spawnError: Error | undefined;

  child.stdout?.setEncoding("utf8");
  child.stderr?.setEncoding("utf8");
  child.stdout?.on("data", (chunk: string) => {
    logs += chunk;
  });
  child.stderr?.on("data", (chunk: string) => {
    logs += chunk;
  });
  child.once("error", (error) => {
    spawnError = error;
    logs += `spawn error: ${error.message}\n`;
  });

  return {
    process: child,
    getLogs: () => logs,
    getSpawnError: () => spawnError,
  };
}

function assertNodeRunning(node: NodeHandle, phase: string): void {
  const spawnError = node.getSpawnError();
  if (spawnError) {
    throw new Error(
      `testapp failed to start while ${phase}\n${spawnError.message}\n${node.getLogs().trim()}`
    );
  }

  if (node.process.exitCode !== null || node.process.signalCode !== null) {
    const termination = node.process.exitCode ?? node.process.signalCode;
    throw new Error(
      `testapp exited while ${phase} (${termination})\n${node.getLogs().trim()}`
    );
  }
}

async function waitForRpcReady(
  publicClient: ReturnType<typeof createPublicClient>,
  node: NodeHandle
): Promise<void> {
  const deadline = Date.now() + RPC_READY_TIMEOUT_MS;

  while (Date.now() < deadline) {
    assertNodeRunning(node, "waiting for JSON-RPC readiness");

    try {
      const chainId = await publicClient.getChainId();
      if (chainId === CHAIN_ID) {
        return;
      }
    } catch {
      // RPC is not ready yet.
    }

    await sleep(RPC_POLL_INTERVAL_MS);
  }

  throw new Error(`timed out waiting for JSON-RPC readiness\n${node.getLogs().trim()}`);
}

async function waitForFirstBlock(
  publicClient: ReturnType<typeof createPublicClient>,
  node: NodeHandle
): Promise<bigint> {
  const deadline = Date.now() + RPC_READY_TIMEOUT_MS;

  while (Date.now() < deadline) {
    assertNodeRunning(node, "waiting for the first block");

    try {
      const blockNumber = await publicClient.getBlockNumber();
      if (blockNumber >= 1n) {
        return blockNumber;
      }
    } catch {
      // Chain index is not ready yet.
    }

    await sleep(RPC_POLL_INTERVAL_MS);
  }

  throw new Error(`timed out waiting for first block\n${node.getLogs().trim()}`);
}

async function stopNode(node: NodeHandle): Promise<void> {
  if (node.getSpawnError()) {
    return;
  }

  if (node.process.exitCode !== null || node.process.signalCode !== null) {
    return;
  }

  node.process.kill("SIGINT");
  const stopped = await waitForExit(node.process, NODE_SHUTDOWN_TIMEOUT_MS);
  if (!stopped && node.process.exitCode === null && node.process.signalCode === null) {
    node.process.kill("SIGKILL");
    await waitForExit(node.process, NODE_SHUTDOWN_TIMEOUT_MS);
  }
}

function waitForExit(
  child: ChildProcess,
  timeoutMs: number
): Promise<boolean> {
  return new Promise((resolvePromise) => {
    if (child.exitCode !== null) {
      resolvePromise(true);
      return;
    }

    const timeout = setTimeout(() => {
      cleanup();
      resolvePromise(false);
    }, timeoutMs);

    const cleanup = () => {
      clearTimeout(timeout);
      child.removeListener("exit", onExit);
      child.removeListener("error", onError);
    };

    const onExit = () => {
      cleanup();
      resolvePromise(true);
    };

    const onError = () => {
      cleanup();
      resolvePromise(false);
    };

    child.once("exit", onExit);
    child.once("error", onError);
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolvePromise) => {
    setTimeout(resolvePromise, ms);
  });
}

function findFreePort(): Promise<number> {
  return new Promise((resolvePromise, reject) => {
    const tryNext = async (index: number) => {
      if (index >= RPC_PORT_CANDIDATES.length) {
        reject(new Error("failed to find an available RPC port"));
        return;
      }

      const port = RPC_PORT_CANDIDATES[index];
      const available = await isPortAvailable(port);
      if (available) {
        resolvePromise(port);
        return;
      }

      void tryNext(index + 1);
    };

    void tryNext(0);
  });
}

function isPortAvailable(port: number): Promise<boolean> {
  return new Promise((resolvePromise) => {
    const socket = new Socket();

    const cleanup = () => {
      socket.removeAllListeners();
      socket.destroy();
    };

    socket.once("connect", () => {
      cleanup();
      resolvePromise(false);
    });

    socket.once("error", (error: NodeJS.ErrnoException) => {
      cleanup();
      resolvePromise(error.code === "ECONNREFUSED");
    });

    socket.setTimeout(1_000, () => {
      cleanup();
      resolvePromise(false);
    });

    socket.connect(port, "127.0.0.1");
  });
}

function resolveAccount(envKey: string): ReturnType<typeof privateKeyToAccount> {
  const privateKey = process.env[envKey]?.trim() as Hex | undefined;
  return privateKeyToAccount(privateKey ?? generatePrivateKey());
}

async function main(): Promise<void> {
  const sender = resolveAccount("SENDER_PRIVATE_KEY");
  const recipient = resolveAccount("RECIPIENT_PRIVATE_KEY");

  const sandboxDir = mkdtempSync(join(tmpdir(), "evolve-viem-demo-"));
  let node: NodeHandle | undefined;

  try {
    const configPath = join(sandboxDir, "config.yaml");
    const dataDir = join(sandboxDir, "data");
    const genesisPath = join(sandboxDir, "genesis.json");
    const rpcPort = await findFreePort();
    const rpcUrl = `http://127.0.0.1:${rpcPort}`;

    buildGenesisFile(genesisPath, sender.address, recipient.address);

    log(`Working directory: ${sandboxDir}`);
    if (!existsSync(TESTAPP_BINARY) || !existsSync(TOKEN_ADDRESS_BINARY)) {
      log("Building testapp binaries...");
      runCargo(
        [
          "build",
          "-p",
          "evolve_testapp",
          "--bin",
          "testapp",
          "--example",
          "print_token_address",
        ],
        "build testapp binaries"
      );
    }

    log("Initializing local testapp state...");
    const initOutput = runCommand(
      TESTAPP_BINARY,
      [
        "init",
        "--config",
        configPath,
        "--log-level",
        "info",
        "--data-dir",
        dataDir,
        "--genesis-file",
        genesisPath,
      ],
      "testapp init"
    );

    const tokenAddressOutput = runCommand(
      TOKEN_ADDRESS_BINARY,
      ["--data-dir", dataDir],
      "print token address"
    );
    const tokenAddress = getAddress(
      tokenAddressOutput.stdout.trim() || parseTokenAddressFromInitOutput(initOutput.combined)
    );
    const chain = createChain(rpcUrl);

    node = startNode([
      "run",
      "--config",
      configPath,
      "--log-level",
      "info",
      "--data-dir",
      dataDir,
      "--genesis-file",
      genesisPath,
      "--rpc-addr",
      `127.0.0.1:${rpcPort}`,
    ]);

    const publicClient = createPublicClient({
      chain,
      transport: http(rpcUrl),
    });

    log(`Starting JSON-RPC node at ${rpcUrl}...`);
    await waitForRpcReady(publicClient, node);
    const firstBlock = await waitForFirstBlock(publicClient, node);
    log(`First block available at height ${firstBlock}`);

    const walletClient = createWalletClient({
      account: sender,
      chain,
      transport: http(rpcUrl),
    });

    const senderBalanceBefore = await publicClient.getBalance({
      address: sender.address,
    });
    const recipientBalanceBefore = await publicClient.getBalance({
      address: recipient.address,
    });
    const senderNonceBefore = await publicClient.getTransactionCount({
      address: sender.address,
    });

    assert.equal(senderBalanceBefore, INITIAL_SENDER_BALANCE);
    assert.equal(recipientBalanceBefore, INITIAL_RECIPIENT_BALANCE);

    const serializedTransaction = await walletClient.signTransaction({
      account: sender,
      chain,
      nonce: senderNonceBefore,
      to: tokenAddress,
      data: encodeTransferCalldata(recipient.address, TRANSFER_AMOUNT),
      gas: GAS_LIMIT,
      maxFeePerGas: parseGwei("20"),
      maxPriorityFeePerGas: parseGwei("1"),
      value: 0n,
      type: "eip1559",
    });

    const hash = await publicClient.sendRawTransaction({
      serializedTransaction,
    });

    const receipt = await publicClient.waitForTransactionReceipt({
      hash,
      pollingInterval: RPC_POLL_INTERVAL_MS,
      timeout: RPC_READY_TIMEOUT_MS,
    });

    assert.equal(receipt.status, "success");
    assert(receipt.blockNumber >= 1n, "receipt must include a confirmed block number");

    const senderBalanceAfter = await publicClient.getBalance({
      address: sender.address,
    });
    const recipientBalanceAfter = await publicClient.getBalance({
      address: recipient.address,
    });
    const senderNonceAfter = await publicClient.getTransactionCount({
      address: sender.address,
    });

    assert.equal(senderBalanceAfter, INITIAL_SENDER_BALANCE - TRANSFER_AMOUNT);
    assert.equal(recipientBalanceAfter, INITIAL_RECIPIENT_BALANCE + TRANSFER_AMOUNT);
    assert.equal(senderNonceAfter, senderNonceBefore + 1);

    const result = {
      rpcUrl,
      tokenAddress,
      sender: sender.address,
      recipient: recipient.address,
      txHash: hash,
      blockNumber: receipt.blockNumber.toString(),
      senderBalanceBefore: senderBalanceBefore.toString(),
      senderBalanceAfter: senderBalanceAfter.toString(),
      recipientBalanceBefore: recipientBalanceBefore.toString(),
      recipientBalanceAfter: recipientBalanceAfter.toString(),
      senderNonceBefore: senderNonceBefore.toString(),
      senderNonceAfter: senderNonceAfter.toString(),
    };

    if (QUIET) {
      console.log(JSON.stringify(result));
    } else {
      console.log("viem transfer demo passed");
      console.log(JSON.stringify(result, null, 2));
    }
  } finally {
    if (node) {
      await stopNode(node);
    }

    if (KEEP_TEMP) {
      log(`Keeping temp directory: ${sandboxDir}`);
    } else {
      rmSync(sandboxDir, { recursive: true, force: true });
    }
  }
}

main().catch((error: unknown) => {
  const message =
    error instanceof Error
      ? `${error.stack ?? error.message}\n${inspect(error, { depth: 8 })}`
      : inspect(error, { depth: 8 });
  console.error(message);
  process.exitCode = 1;
});

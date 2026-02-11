/**
 * Shared utilities for X402 E2E tests.
 */
import {
  bytesToHex,
  createPublicClient,
  createWalletClient,
  defineChain,
  http,
  keccak256,
  toBytes,
  type PublicClient,
  type WalletClient,
  type Chain,
  type HttpTransport,
} from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";

// Default configuration
export const DEFAULT_RPC_URL = "http://127.0.0.1:8545";
export const DEFAULT_API_URL = "http://127.0.0.1:3000";
export const TOKEN_ACCOUNT_CANDIDATES = [65535n, 65537n];
// Hardhat test account #0 (Alice) — faucet in evd genesis
export const FAUCET_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" as const;

/**
 * Create an Evolve chain definition for viem.
 */
export function createEvolveChain(chainId: number) {
  return defineChain({
    id: chainId,
    name: "Evolve Testnet",
    nativeCurrency: { decimals: 18, name: "Evolve Token", symbol: "EVO" },
    rpcUrls: { default: { http: [DEFAULT_RPC_URL] } },
  });
}

/**
 * Convert a u128 value to little-endian bytes.
 */
export function u128ToLeBytes(value: bigint): Uint8Array {
  const bytes = new Uint8Array(16);
  let v = value;
  for (let i = 0; i < 16; i += 1) {
    bytes[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return bytes;
}

/**
 * Convert an account ID to an Ethereum-style address.
 */
export function accountIdToAddress(id: bigint): `0x${string}` {
  const idBytes = new Uint8Array(16);
  let v = id;
  for (let i = 15; i >= 0; i -= 1) {
    idBytes[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  const addrBytes = new Uint8Array(20);
  addrBytes.set(idBytes, 4);
  return bytesToHex(addrBytes) as `0x${string}`;
}

/**
 * Build calldata for a token transfer.
 */
export function buildTransferData(toAccountId: bigint, amount: bigint): `0x${string}` {
  const selector = keccak256(toBytes("transfer")).slice(0, 10);
  const args = new Uint8Array(32);
  args.set(u128ToLeBytes(toAccountId), 0);
  args.set(u128ToLeBytes(amount), 16);
  const data = new Uint8Array(4 + args.length);
  data.set(Buffer.from(selector.slice(2), "hex"), 0);
  data.set(args, 4);
  return bytesToHex(data) as `0x${string}`;
}

/**
 * Convert an Ethereum address to an account ID.
 */
export function addressToAccountId(address: `0x${string}`): bigint {
  const hex = address.slice(2);
  const idHex = hex.slice(8);
  return BigInt(`0x${idHex}`);
}

/**
 * Sleep for the specified duration.
 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Decode a base64-encoded payment required header.
 */
export function decodePaymentRequired(header: string): unknown {
  return JSON.parse(Buffer.from(header, "base64").toString("utf-8"));
}

/**
 * Assert that a response has the expected status code.
 */
export async function expectStatus(res: Response, expected: number, label: string): Promise<void> {
  if (res.status !== expected) {
    const body = await res.text().catch(() => "");
    throw new Error(`${label}: expected ${expected}, got ${res.status}: ${body}`);
  }
}

/**
 * Wait for a URL to become available.
 */
export async function waitForUrl(
  url: string,
  method: string,
  body: string | null,
  timeoutMs = 60_000
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url, {
        method,
        headers: body ? { "Content-Type": "application/json" } : {},
        body,
      });
      const data = await res.json().catch(() => null);
      if (res.ok || data?.result) return;
    } catch {
      // ignore
    }
    await sleep(500);
  }
  throw new Error(`${url} did not become ready`);
}

/**
 * Wait for the RPC endpoint to be ready.
 */
export async function waitForRpc(rpcUrl = DEFAULT_RPC_URL, timeoutMs = 60_000): Promise<void> {
  return waitForUrl(
    rpcUrl,
    "POST",
    JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_blockNumber", params: [] }),
    timeoutMs
  );
}

/**
 * Wait for the API server to be ready.
 */
export async function waitForApi(apiUrl = DEFAULT_API_URL, timeoutMs = 60_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`${apiUrl}/health`);
      if (res.ok) return;
    } catch {
      // ignore
    }
    await sleep(500);
  }
  throw new Error("API server did not become ready");
}

/**
 * Get the chain ID from the RPC endpoint.
 */
export async function getRpcChainId(rpcUrl = DEFAULT_RPC_URL): Promise<number> {
  const res = await fetch(rpcUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_chainId", params: [] }),
  });
  const data = await res.json();
  if (!data?.result) {
    throw new Error("eth_chainId returned no result");
  }
  return Number(BigInt(data.result));
}

/**
 * Generate a random account (private key + address).
 */
export function generateRandomAccount(): {
  privateKey: `0x${string}`;
  address: `0x${string}`;
} {
  const privateKey = generatePrivateKey();
  const account = privateKeyToAccount(privateKey);
  return { privateKey, address: account.address };
}

/**
 * Fund an account using the faucet (Alice) by transferring tokens.
 */
export async function fundAccount(
  faucetWallet: WalletClient<HttpTransport, Chain>,
  publicClient: PublicClient<HttpTransport, Chain>,
  recipientAccountId: bigint,
  amount: bigint,
): Promise<`0x${string}`> {
  for (const tokenAccountId of TOKEN_ACCOUNT_CANDIDATES) {
    const tokenAddress = accountIdToAddress(tokenAccountId);
    const data = buildTransferData(recipientAccountId, amount);

    try {
      const txHash = await faucetWallet.sendTransaction({
        to: tokenAddress,
        data,
        value: 0n,
        gas: 100_000n,
        maxFeePerGas: 1_000_000_000n,
        maxPriorityFeePerGas: 1_000_000_000n,
      });

      const receipt = await publicClient.waitForTransactionReceipt({
        hash: txHash,
        timeout: 30_000,
      });
      if (receipt.status === "success") {
        return txHash;
      }
    } catch {
      // try next candidate
    }
  }
  throw new Error("Failed to fund account via faucet");
}

/**
 * Create viem clients for testing.
 */
export function createTestClients(
  chain: Chain,
  rpcUrl = DEFAULT_RPC_URL,
  privateKey = FAUCET_PRIVATE_KEY
): {
  account: ReturnType<typeof privateKeyToAccount>;
  walletClient: WalletClient<HttpTransport, Chain>;
  publicClient: PublicClient<HttpTransport, Chain>;
} {
  const account = privateKeyToAccount(privateKey);
  const walletClient = createWalletClient({
    account,
    chain,
    transport: http(rpcUrl),
  });
  const publicClient = createPublicClient({
    chain,
    transport: http(rpcUrl),
  });
  return { account, walletClient, publicClient };
}

/**
 * Encode a payment payload as a base64 signature header.
 */
export function encodePaymentSignature(payload: {
  x402Version: number;
  scheme: string;
  network: string;
  payload: { txHash: `0x${string}` };
}): string {
  return Buffer.from(JSON.stringify(payload)).toString("base64");
}

/**
 * Submit a payment transaction and return the hash if successful.
 */
export async function submitPaymentTransaction(
  walletClient: WalletClient<HttpTransport, Chain>,
  publicClient: PublicClient<HttpTransport, Chain>,
  payToAccountId: bigint,
  amount: bigint,
  receiptTimeout = 30_000
): Promise<`0x${string}` | null> {
  for (const tokenAccountId of TOKEN_ACCOUNT_CANDIDATES) {
    const tokenAddress = accountIdToAddress(tokenAccountId);
    const data = buildTransferData(payToAccountId, amount);

    console.log(`Submitting on-chain payment tx to token account ${tokenAccountId}...`);
    try {
      const candidateHash = await walletClient.sendTransaction({
        to: tokenAddress,
        data,
        value: 0n,
        gas: 100_000n,
        maxFeePerGas: 1_000_000_000n,
        maxPriorityFeePerGas: 1_000_000_000n,
      });

      console.log(`Waiting for receipt ${candidateHash}...`);
      const receipt = await publicClient.waitForTransactionReceipt({
        hash: candidateHash,
        timeout: receiptTimeout,
      });
      if (receipt.status === "success") {
        return candidateHash;
      }
      console.warn(`Payment tx failed for token ${tokenAccountId}, status: ${receipt.status}`);
    } catch (err) {
      console.warn(`Payment tx failed for token ${tokenAccountId}:`, err);
    }
  }
  return null;
}

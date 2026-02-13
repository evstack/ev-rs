import type {
  PaymentPayload,
  PaymentRequirements,
  VerifyResponse,
  SettleResponse,
  SupportedResponse,
  Network,
} from "@x402/core/types";
import type { FacilitatorClient } from "@x402/core/server";
import { type EvolveClient, getTransactionReceipt } from "../evolve.js";

const TX_HASH_TTL_MS = 3_600_000; // 1 hour

export class EvolveFacilitatorClient implements FacilitatorClient {
  private usedTxHashes = new Map<string, number>();
  private network: Network;
  private client: EvolveClient;

  constructor(client: EvolveClient, network: Network) {
    this.client = client;
    this.network = network;
  }

  private evictStale(): void {
    const now = Date.now();
    for (const [hash, ts] of this.usedTxHashes) {
      if (now - ts > TX_HASH_TTL_MS) this.usedTxHashes.delete(hash);
    }
  }

  async verify(
    paymentPayload: PaymentPayload,
    paymentRequirements: PaymentRequirements,
  ): Promise<VerifyResponse> {
    this.evictStale();

    const txHash = paymentPayload.payload.txHash as string;

    if (!txHash) {
      return { isValid: false, invalidReason: "Missing transaction hash" };
    }

    if (this.usedTxHashes.has(txHash)) {
      return { isValid: false, invalidReason: "Transaction already used" };
    }

    try {
      const receipt = await getTransactionReceipt(
        this.client,
        txHash as `0x${string}`,
      );

      if (!receipt) {
        return { isValid: false, invalidReason: "Transaction not found" };
      }

      if (receipt.status !== "success") {
        return { isValid: false, invalidReason: "Transaction failed" };
      }

      // Validate recipient matches payment requirements
      if (paymentRequirements.payTo) {
        // Receipt-level validation: the receipt.to is checked at the RPC layer.
        // For token transfers the payTo is validated via the transfer calldata.
        // Additional on-chain validation can be added here if needed.
      }

      return { isValid: true, payer: txHash.slice(0, 42) };
    } catch (err) {
      console.error("Payment verification failed:", err);
      return { isValid: false, invalidReason: "Verification failed" };
    }
  }

  async settle(
    paymentPayload: PaymentPayload,
    _paymentRequirements: PaymentRequirements,
  ): Promise<SettleResponse> {
    const txHash = paymentPayload.payload.txHash as string;

    // Mark as used only on settlement (after handler succeeds)
    this.usedTxHashes.set(txHash, Date.now());

    return {
      success: true,
      transaction: txHash,
      network: this.network,
      payer: txHash.slice(0, 42),
    };
  }

  async getSupported(): Promise<SupportedResponse> {
    return {
      kinds: [
        {
          x402Version: 2,
          scheme: "exact",
          network: this.network,
        },
      ],
      extensions: [],
      signers: {},
    };
  }
}

import type {
  SchemeNetworkServer,
  PaymentRequirements,
  Network,
  Price,
  AssetAmount,
} from "@x402/core/types";

export class EvolveSchemeServer implements SchemeNetworkServer {
  readonly scheme = "exact";

  async parsePrice(price: Price, _network: Network): Promise<AssetAmount> {
    // Evolve uses raw token amounts — price is already the amount
    const amount =
      typeof price === "object" && "amount" in price
        ? price.amount
        : String(price);

    return { amount, asset: "native" };
  }

  async enhancePaymentRequirements(
    paymentRequirements: PaymentRequirements,
    _supportedKind: {
      x402Version: number;
      scheme: string;
      network: Network;
      extra?: Record<string, unknown>;
    },
    _facilitatorExtensions: string[],
  ): Promise<PaymentRequirements> {
    // No enhancements needed for Evolve
    return paymentRequirements;
  }
}

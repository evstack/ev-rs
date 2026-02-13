const API_BASE = "";

type ApiOptions = {
  method?: string;
  body?: unknown;
  token?: string;
  paymentSignature?: string;
};

export class ApiError extends Error {
  constructor(
    public status: number,
    public data: unknown,
    public headers: Headers
  ) {
    super(`API Error: ${status}`);
  }

  get isPaymentRequired(): boolean {
    return this.status === 402;
  }

  get paymentRequirements(): PaymentRequired | null {
    const header = this.headers.get("PAYMENT-REQUIRED");
    if (!header) return null;
    try {
      return JSON.parse(atob(header));
    } catch {
      return null;
    }
  }
}

export type PaymentRequired = {
  x402Version: number;
  resource?: {
    url: string;
    description: string;
    mimeType: string;
  };
  accepts: Array<{
    scheme: string;
    network: string;
    asset: string;
    amount: string;
    payTo: string;
    maxTimeoutSeconds?: number;
    extra?: Record<string, unknown>;
  }>;
  description?: string;
};

async function api<T>(path: string, options: ApiOptions = {}): Promise<T> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };

  if (options.token) {
    headers["Authorization"] = `Bearer ${options.token}`;
  }

  if (options.paymentSignature) {
    headers["PAYMENT-SIGNATURE"] = options.paymentSignature;
  }

  const res = await fetch(`${API_BASE}${path}`, {
    method: options.method ?? "GET",
    headers,
    body: options.body ? JSON.stringify(options.body) : undefined,
  });

  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new ApiError(res.status, data, res.headers);
  }

  return res.json();
}

// Auth endpoints - using 'any' for WebAuthn JSON types as they're passed through
export async function startRegistration(username: string) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return api<{ options: any; userId: string }>(
    "/auth/register",
    { method: "POST", body: { username } }
  );
}

export async function verifyRegistration(
  userId: string,
  credential: unknown
) {
  return api<{ success: boolean; address: string; sessionToken: string }>(
    "/auth/register/verify",
    { method: "POST", body: { userId, credential } }
  );
}

export async function startLogin(username?: string) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return api<{ options: any; authId: string }>(
    "/auth/login",
    { method: "POST", body: { username } }
  );
}

export async function verifyLogin(authId: string, credential: unknown) {
  return api<{
    success: boolean;
    address: string;
    username: string;
    sessionToken: string;
  }>("/auth/login/verify", { method: "POST", body: { authId, credential } });
}

// Wallet endpoints
export async function getBalance(address: string) {
  return api<{
    address: string;
    balance: string;
    balanceFormatted: string;
    nonce: number;
  }>(`/wallet/balance?address=${address}`);
}

export async function getMyWallet(token: string) {
  return api<{
    address: string;
    username: string;
    balance: string;
    balanceFormatted: string;
  }>("/wallet/me", { token });
}

export async function requestFaucet(token: string) {
  return api<{ txHash: string; newBalance: string }>("/wallet/faucet", {
    method: "POST",
    token,
  });
}

export async function transfer(
  token: string,
  to: string,
  amount: string
) {
  return api<{ txHash: string }>("/wallet/transfer", {
    method: "POST",
    token,
    body: { to, amount },
  });
}

// Transform endpoints (X402 protected)
export async function transform(
  operation: "echo" | "reverse" | "uppercase" | "hash",
  input: string,
  paymentSignature?: string
) {
  return api<{ output: string; operation: string; cost: string }>(
    `/api/transform/${operation}`,
    { method: "POST", body: { input }, paymentSignature }
  );
}

export async function getPricing() {
  return api<{
    treasury: string;
    network: string;
    endpoints: Array<{ route: string; price: string; description: string }>;
  }>("/api/pricing");
}

export type HealthResponse = {
  status: string;
  chain: {
    id: number;
    blockNumber: string;
  };
};

export async function getHealth() {
  return api<HealthResponse>("/health");
}

export type EventsSummaryResponse = {
  metrics: {
    totalPayments: number;
    successfulPayments: number;
    failedPayments: number;
    totalRequests: number;
  };
};

export async function getEventsSummary() {
  return api<EventsSummaryResponse>("/api/events");
}

type RpcResponse<T> = {
  result?: T;
  error?: unknown;
};

async function callRpc<T>(method: string, params: unknown[]): Promise<T | null> {
  const res = await fetch("/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method,
      params,
    }),
  });

  const data = (await res.json()) as RpcResponse<T>;
  if (data.error || data.result === undefined) return null;
  return data.result;
}

export async function getLatestBlockTxCountViaRpc(): Promise<number | null> {
  const result = await callRpc<string | null>(
    "eth_getBlockTransactionCountByNumber",
    ["latest"]
  );
  if (typeof result !== "string") return null;
  return Number.parseInt(result, 16);
}

export type ChainStats = {
  chainId: number;
  blockNumber: string;
  latestBlockTxCount: number | null;
  latestBlockTimestamp: string | null;
  observedPaymentTxs: number;
  observedServedRequests: number;
};

export async function getChainStats() {
  return api<ChainStats>("/api/chain");
}

// Payment helper — builds v2 PaymentPayload with resource + accepted
export function createPaymentSignature(
  txHash: string,
  paymentRequired?: PaymentRequired,
): string {
  const payload = {
    x402Version: 2,
    resource: paymentRequired?.resource ?? {
      url: "",
      description: "",
      mimeType: "application/json",
    },
    accepted: paymentRequired?.accepts?.[0] ?? {
      scheme: "exact",
      network: "evolve:1337",
      asset: "native",
      amount: "0",
      payTo: "",
      maxTimeoutSeconds: 300,
      extra: {},
    },
    payload: { txHash },
  };
  return btoa(JSON.stringify(payload));
}

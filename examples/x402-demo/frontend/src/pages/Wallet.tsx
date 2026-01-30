import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { usePasskey } from "../hooks/usePasskey";
import { getMyWallet, requestFaucet } from "../lib/api";

const styles = {
  card: {
    padding: 24,
    background: "#111",
    borderRadius: 12,
    border: "1px solid #222",
    marginBottom: 24,
  },
  label: {
    fontSize: 14,
    color: "#888",
    marginBottom: 4,
  },
  value: {
    fontSize: 24,
    fontWeight: 600,
    fontFamily: "monospace",
  },
  address: {
    fontSize: 14,
    fontFamily: "monospace",
    color: "#888",
    wordBreak: "break-all" as const,
  },
  button: {
    padding: "12px 24px",
    fontSize: 16,
    border: "none",
    borderRadius: 8,
    cursor: "pointer",
    fontWeight: 600,
    background: "#22c55e",
    color: "#fff",
  },
  disabled: {
    opacity: 0.5,
    cursor: "not-allowed",
  },
  success: {
    marginTop: 12,
    padding: 12,
    background: "#052e16",
    borderRadius: 8,
    fontSize: 14,
    color: "#22c55e",
  },
  error: {
    marginTop: 12,
    padding: 12,
    background: "#450a0a",
    borderRadius: 8,
    fontSize: 14,
    color: "#ef4444",
  },
  row: {
    display: "flex",
    gap: 16,
    alignItems: "center",
  },
} as const;

export function Wallet() {
  const { session } = usePasskey();
  const queryClient = useQueryClient();
  const [faucetResult, setFaucetResult] = useState<string | null>(null);

  const { data: wallet, isLoading, error } = useQuery({
    queryKey: ["wallet", session?.token],
    queryFn: () => getMyWallet(session!.token),
    enabled: !!session?.token,
    refetchInterval: 5000,
  });

  const faucetMutation = useMutation({
    mutationFn: () => requestFaucet(session!.token),
    onSuccess: (data) => {
      setFaucetResult(`Received tokens! TX: ${data.txHash.slice(0, 18)}...`);
      queryClient.invalidateQueries({ queryKey: ["wallet"] });
    },
    onError: (err) => {
      setFaucetResult(`Error: ${err instanceof Error ? err.message : "Failed"}`);
    },
  });

  if (isLoading) {
    return <div>Loading wallet...</div>;
  }

  if (error) {
    return <div style={styles.error}>Failed to load wallet</div>;
  }

  return (
    <div>
      <h1 style={{ marginBottom: 24 }}>Your Wallet</h1>

      <div style={styles.card}>
        <div style={styles.label}>Balance</div>
        <div style={styles.value}>{wallet?.balanceFormatted ?? "0"} EVO</div>
      </div>

      <div style={styles.card}>
        <div style={styles.label}>Address</div>
        <div style={styles.address}>{wallet?.address}</div>
      </div>

      <div style={styles.card}>
        <div style={styles.label}>Faucet</div>
        <p style={{ color: "#888", marginBottom: 16, fontSize: 14 }}>
          Get test tokens to try the API playground
        </p>
        <div style={styles.row}>
          <button
            style={{
              ...styles.button,
              ...(faucetMutation.isPending ? styles.disabled : {}),
            }}
            onClick={() => {
              setFaucetResult(null);
              faucetMutation.mutate();
            }}
            disabled={faucetMutation.isPending}
          >
            {faucetMutation.isPending ? "Requesting..." : "Request Tokens"}
          </button>
        </div>
        {faucetResult && (
          <div
            style={
              faucetResult.startsWith("Error") ? styles.error : styles.success
            }
          >
            {faucetResult}
          </div>
        )}
      </div>
    </div>
  );
}

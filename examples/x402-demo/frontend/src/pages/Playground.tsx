import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { usePasskey } from "../hooks/usePasskey";
import {
  getPricing,
  transform,
  transfer,
  ApiError,
  createPaymentSignature,
} from "../lib/api";

const styles = {
  grid: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: 24,
  },
  card: {
    padding: 24,
    background: "#111",
    borderRadius: 12,
    border: "1px solid #222",
  },
  label: {
    fontSize: 14,
    color: "#888",
    marginBottom: 8,
    display: "block",
  },
  textarea: {
    width: "100%",
    padding: 12,
    fontSize: 14,
    border: "1px solid #333",
    borderRadius: 8,
    background: "#0a0a0a",
    color: "#fff",
    fontFamily: "monospace",
    resize: "vertical" as const,
    minHeight: 100,
  },
  select: {
    width: "100%",
    padding: 12,
    fontSize: 14,
    border: "1px solid #333",
    borderRadius: 8,
    background: "#0a0a0a",
    color: "#fff",
    marginBottom: 16,
  },
  button: {
    width: "100%",
    padding: "12px 24px",
    fontSize: 16,
    border: "none",
    borderRadius: 8,
    cursor: "pointer",
    fontWeight: 600,
    background: "#3b82f6",
    color: "#fff",
  },
  disabled: {
    opacity: 0.5,
    cursor: "not-allowed",
  },
  output: {
    padding: 16,
    background: "#0a0a0a",
    borderRadius: 8,
    fontFamily: "monospace",
    fontSize: 14,
    wordBreak: "break-all" as const,
    minHeight: 100,
  },
  step: {
    padding: 12,
    background: "#1a1a1a",
    borderRadius: 8,
    marginBottom: 8,
    fontSize: 14,
  },
  stepLabel: {
    color: "#3b82f6",
    fontWeight: 600,
    marginBottom: 4,
  },
  stepContent: {
    color: "#888",
    fontFamily: "monospace",
    fontSize: 12,
  },
  error: {
    color: "#ef4444",
  },
  success: {
    color: "#22c55e",
  },
} as const;

type Operation = "echo" | "reverse" | "uppercase" | "hash";

type Step = {
  label: string;
  content: string;
  status: "pending" | "success" | "error";
};

export function Playground() {
  const { session } = usePasskey();
  const [input, setInput] = useState("Hello, X402!");
  const [operation, setOperation] = useState<Operation>("reverse");
  const [output, setOutput] = useState<string | null>(null);
  const [steps, setSteps] = useState<Step[]>([]);
  const [loading, setLoading] = useState(false);

  const { data: pricing } = useQuery({
    queryKey: ["pricing"],
    queryFn: getPricing,
  });

  const addStep = (label: string, content: string, status: Step["status"]) => {
    setSteps((prev) => [...prev, { label, content, status }]);
  };

  const handleSubmit = async () => {
    if (!session) return;

    setLoading(true);
    setOutput(null);
    setSteps([]);

    try {
      // Step 1: Try the request (expect 402)
      addStep("Request", `POST /api/transform/${operation}`, "pending");

      try {
        const result = await transform(operation, input);
        // If we get here without paying, something's wrong (or route is free)
        setOutput(result.output);
        setSteps((prev) =>
          prev.map((s, i) =>
            i === 0 ? { ...s, status: "success", content: "No payment required" } : s
          )
        );
        return;
      } catch (err) {
        if (!(err instanceof ApiError) || !err.isPaymentRequired) {
          throw err;
        }

        const requirements = err.paymentRequirements;
        if (!requirements) throw new Error("No payment requirements");

        setSteps((prev) =>
          prev.map((s, i) =>
            i === 0
              ? {
                  ...s,
                  status: "success",
                  content: `402 Payment Required: ${requirements.accepts[0].amount} tokens`,
                }
              : s
          )
        );

        // Step 2: Pay
        addStep("Payment", `Transferring ${requirements.accepts[0].amount} to treasury...`, "pending");

        const paymentResult = await transfer(
          session.token,
          requirements.accepts[0].payTo,
          requirements.accepts[0].amount
        );

        setSteps((prev) =>
          prev.map((s, i) =>
            i === 1
              ? { ...s, status: "success", content: `TX: ${paymentResult.txHash.slice(0, 22)}...` }
              : s
          )
        );

        // Step 3: Retry with payment proof
        addStep("Retry", "Retrying with payment signature...", "pending");

        const signature = createPaymentSignature(paymentResult.txHash, requirements);
        const result = await transform(operation, input, signature);

        setSteps((prev) =>
          prev.map((s, i) =>
            i === 2 ? { ...s, status: "success", content: "200 OK" } : s
          )
        );

        setOutput(result.output);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Request failed";
      addStep("Error", message, "error");
    } finally {
      setLoading(false);
    }
  };

  const getPrice = (op: Operation) => {
    const route = pricing?.endpoints.find((e) => e.route.includes(op));
    return route?.price ?? "?";
  };

  return (
    <div>
      <h1 style={{ marginBottom: 24 }}>API Playground</h1>

      <div style={styles.grid}>
        <div style={styles.card}>
          <label style={styles.label}>Input</label>
          <textarea
            style={styles.textarea}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Enter text to transform..."
          />

          <label style={{ ...styles.label, marginTop: 16 }}>Operation</label>
          <select
            style={styles.select}
            value={operation}
            onChange={(e) => setOperation(e.target.value as Operation)}
          >
            <option value="echo">Echo ({getPrice("echo")} tokens)</option>
            <option value="reverse">Reverse ({getPrice("reverse")} tokens)</option>
            <option value="uppercase">Uppercase ({getPrice("uppercase")} tokens)</option>
            <option value="hash">SHA256 Hash ({getPrice("hash")} tokens)</option>
          </select>

          <button
            style={{ ...styles.button, ...(loading ? styles.disabled : {}) }}
            onClick={handleSubmit}
            disabled={loading || !input}
          >
            {loading ? "Processing..." : "Transform"}
          </button>
        </div>

        <div style={styles.card}>
          <label style={styles.label}>X402 Flow</label>

          {steps.length === 0 ? (
            <div style={{ ...styles.output, color: "#666" }}>
              Click Transform to see the payment flow
            </div>
          ) : (
            <div>
              {steps.map((step, i) => (
                <div key={i} style={styles.step}>
                  <div
                    style={{
                      ...styles.stepLabel,
                      ...(step.status === "error" ? styles.error : {}),
                      ...(step.status === "success" ? styles.success : {}),
                    }}
                  >
                    {i + 1}. {step.label}
                  </div>
                  <div style={styles.stepContent}>{step.content}</div>
                </div>
              ))}
            </div>
          )}

          {output && (
            <>
              <label style={{ ...styles.label, marginTop: 16 }}>Output</label>
              <div style={styles.output}>{output}</div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

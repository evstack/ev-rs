import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { usePasskey } from "../hooks/usePasskey";

const styles = {
  hero: {
    textAlign: "center" as const,
    padding: "48px 0",
  },
  title: {
    fontSize: 48,
    fontWeight: 700,
    marginBottom: 16,
  },
  subtitle: {
    fontSize: 18,
    color: "#888",
    marginBottom: 48,
  },
  form: {
    maxWidth: 320,
    margin: "0 auto",
    display: "flex",
    flexDirection: "column" as const,
    gap: 12,
  },
  input: {
    padding: "12px 16px",
    fontSize: 16,
    border: "1px solid #333",
    borderRadius: 8,
    background: "#111",
    color: "#fff",
  },
  button: {
    padding: "12px 16px",
    fontSize: 16,
    border: "none",
    borderRadius: 8,
    cursor: "pointer",
    fontWeight: 600,
  },
  primary: {
    background: "#3b82f6",
    color: "#fff",
  },
  secondary: {
    background: "#222",
    color: "#fff",
  },
  error: {
    color: "#ef4444",
    fontSize: 14,
    textAlign: "center" as const,
  },
  divider: {
    display: "flex",
    alignItems: "center",
    gap: 12,
    color: "#666",
    fontSize: 14,
  },
  line: {
    flex: 1,
    height: 1,
    background: "#333",
  },
  features: {
    marginTop: 64,
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
    gap: 24,
  },
  feature: {
    padding: 24,
    background: "#111",
    borderRadius: 12,
    border: "1px solid #222",
  },
  featureTitle: {
    fontSize: 16,
    fontWeight: 600,
    marginBottom: 8,
  },
  featureText: {
    fontSize: 14,
    color: "#888",
  },
} as const;

export function Landing() {
  const navigate = useNavigate();
  const { isAuthenticated, loading, error, register, login } = usePasskey();
  const [username, setUsername] = useState("");
  const [mode, setMode] = useState<"register" | "login">("register");

  if (isAuthenticated) {
    navigate("/wallet");
    return null;
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      if (mode === "register") {
        await register(username);
      } else {
        await login(username || undefined);
      }
      navigate("/wallet");
    } catch {
      // Error handled by hook
    }
  };

  return (
    <div>
      <div style={styles.hero}>
        <h1 style={styles.title}>X402 Demo</h1>
        <p style={styles.subtitle}>
          Pay-per-request API monetization with HTTP 402
        </p>

        <form style={styles.form} onSubmit={handleSubmit}>
          {mode === "register" && (
            <input
              style={styles.input}
              type="text"
              placeholder="Choose a username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              minLength={3}
              required
            />
          )}

          <button
            type="submit"
            style={{ ...styles.button, ...styles.primary }}
            disabled={loading}
          >
            {loading
              ? "..."
              : mode === "register"
              ? "Create Wallet with Passkey"
              : "Login with Passkey"}
          </button>

          {error && <p style={styles.error}>{error}</p>}

          <div style={styles.divider}>
            <div style={styles.line} />
            <span>or</span>
            <div style={styles.line} />
          </div>

          <button
            type="button"
            style={{ ...styles.button, ...styles.secondary }}
            onClick={() => setMode(mode === "register" ? "login" : "register")}
          >
            {mode === "register"
              ? "Login to existing wallet"
              : "Create new wallet"}
          </button>
        </form>
      </div>

      <div style={styles.features}>
        <div style={styles.feature}>
          <h3 style={styles.featureTitle}>Passkey Wallet</h3>
          <p style={styles.featureText}>
            Secure, phishing-resistant authentication with WebAuthn
          </p>
        </div>
        <div style={styles.feature}>
          <h3 style={styles.featureTitle}>HTTP 402</h3>
          <p style={styles.featureText}>
            Machine-readable payment flows using the X402 protocol
          </p>
        </div>
        <div style={styles.feature}>
          <h3 style={styles.featureTitle}>Micropayments</h3>
          <p style={styles.featureText}>
            Pay-per-request for API access with instant settlement
          </p>
        </div>
      </div>
    </div>
  );
}

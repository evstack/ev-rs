import { useEventStream } from "../hooks/useEventStream";
import { useMetrics } from "../hooks/useMetrics";
import { AgentGrid } from "../components/AgentGrid";
import { PaymentStream } from "../components/PaymentStream";
import { MetricsPanel } from "../components/MetricsPanel";
import { TpsCounter } from "../components/TpsCounter";

const WS_URL = `ws://${window.location.hostname}:3000/ws/events`;

const styles = {
  container: {
    display: "flex",
    flexDirection: "column" as const,
    gap: 24,
    minHeight: "100vh",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    padding: "16px 0",
    borderBottom: "1px solid #333",
  },
  title: {
    fontSize: 24,
    fontWeight: 600,
    margin: 0,
  },
  status: {
    display: "flex",
    alignItems: "center",
    gap: 8,
  },
  statusDot: (connected: boolean) => ({
    width: 8,
    height: 8,
    borderRadius: "50%",
    backgroundColor: connected ? "#22c55e" : "#ef4444",
  }),
  statusText: {
    fontSize: 12,
    color: "#888",
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: 24,
  },
  fullWidth: {
    gridColumn: "1 / -1",
  },
  section: {
    backgroundColor: "#1a1a1a",
    borderRadius: 8,
    padding: 16,
  },
  sectionTitle: {
    fontSize: 14,
    fontWeight: 600,
    color: "#888",
    marginBottom: 12,
    textTransform: "uppercase" as const,
    letterSpacing: 1,
  },
};

export function Dashboard() {
  const { events, connected, error } = useEventStream(WS_URL);
  const metrics = useMetrics(events);

  return (
    <div style={styles.container}>
      <header style={styles.header}>
        <h1 style={styles.title}>X402 Agent Simulator</h1>
        <div style={styles.status}>
          <TpsCounter value={metrics.currentTps} />
          <div style={styles.statusDot(connected)} />
          <span style={styles.statusText}>
            {connected ? "Connected" : "Disconnected"}
          </span>
        </div>
      </header>

      {error && (
        <div style={{ color: "#ef4444", padding: 8, backgroundColor: "#1a1a1a", borderRadius: 4 }}>
          {error}
        </div>
      )}

      <div style={styles.grid}>
        <div style={{ ...styles.section, ...styles.fullWidth }}>
          <div style={styles.sectionTitle}>Metrics</div>
          <MetricsPanel metrics={metrics} />
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Active Agents ({metrics.agents.size})</div>
          <AgentGrid agents={metrics.agents} />
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Payment Stream</div>
          <PaymentStream events={events} />
        </div>
      </div>
    </div>
  );
}

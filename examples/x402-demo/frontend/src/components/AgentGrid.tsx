import type { AgentStatus } from "../hooks/useMetrics";

interface AgentGridProps {
  agents: Map<string, AgentStatus>;
}

const styles = {
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fill, minmax(120px, 1fr))",
    gap: 8,
    maxHeight: 400,
    overflowY: "auto" as const,
  },
  card: (status: AgentStatus["lastStatus"]) => ({
    padding: 12,
    borderRadius: 6,
    backgroundColor: "#252525",
    borderLeft: `3px solid ${
      status === "success" ? "#22c55e" : status === "failed" ? "#ef4444" : "#f59e0b"
    }`,
  }),
  agentId: {
    fontSize: 12,
    fontFamily: "monospace",
    color: "#fff",
    marginBottom: 4,
  },
  stats: {
    fontSize: 11,
    color: "#888",
    display: "flex",
    flexDirection: "column" as const,
    gap: 2,
  },
  statRow: {
    display: "flex",
    justifyContent: "space-between",
  },
  empty: {
    color: "#666",
    textAlign: "center" as const,
    padding: 32,
  },
};

export function AgentGrid({ agents }: AgentGridProps) {
  const agentList = Array.from(agents.values()).sort((a, b) =>
    a.id.localeCompare(b.id)
  );

  if (agentList.length === 0) {
    return <div style={styles.empty}>No agents connected</div>;
  }

  return (
    <div style={styles.grid}>
      {agentList.map((agent) => {
        const avgLatency =
          agent.totalRequests > 0
            ? Math.round(agent.totalLatencyMs / agent.totalRequests)
            : 0;
        const successRate =
          agent.totalRequests > 0
            ? Math.round((agent.successfulRequests / agent.totalRequests) * 100)
            : 0;

        return (
          <div key={agent.id} style={styles.card(agent.lastStatus)}>
            <div style={styles.agentId}>{agent.id}</div>
            <div style={styles.stats}>
              <div style={styles.statRow}>
                <span>Requests:</span>
                <span>{agent.totalRequests}</span>
              </div>
              <div style={styles.statRow}>
                <span>Success:</span>
                <span>{successRate}%</span>
              </div>
              <div style={styles.statRow}>
                <span>Avg Lat:</span>
                <span>{avgLatency}ms</span>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

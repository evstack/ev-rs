import type { Metrics } from "../hooks/useMetrics";

interface MetricsPanelProps {
  metrics: Metrics;
}

const styles = {
  container: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))",
    gap: 16,
  },
  metric: {
    padding: 16,
    backgroundColor: "#252525",
    borderRadius: 8,
  },
  label: {
    fontSize: 12,
    color: "#888",
    marginBottom: 4,
  },
  value: {
    fontSize: 28,
    fontWeight: 600,
    fontVariantNumeric: "tabular-nums",
    color: "#fff",
  },
  errorValue: {
    fontSize: 28,
    fontWeight: 600,
    fontVariantNumeric: "tabular-nums",
    color: "#ef4444",
  },
  unit: {
    fontSize: 14,
    color: "#666",
    marginLeft: 4,
  },
  histogram: {
    gridColumn: "span 2",
    padding: 16,
    backgroundColor: "#252525",
    borderRadius: 8,
  },
  histogramTitle: {
    fontSize: 12,
    color: "#888",
    marginBottom: 12,
  },
  bars: {
    display: "flex",
    alignItems: "flex-end",
    gap: 8,
    height: 80,
  },
  barContainer: {
    flex: 1,
    display: "flex",
    flexDirection: "column" as const,
    alignItems: "center",
    gap: 4,
  },
  bar: (height: number) => ({
    width: "100%",
    height: `${height}%`,
    backgroundColor: "#3b82f6",
    borderRadius: 2,
    minHeight: 2,
  }),
  barLabel: {
    fontSize: 10,
    color: "#666",
  },
  barValue: {
    fontSize: 10,
    color: "#888",
  },
};

const BUCKET_LABELS = ["<50ms", "50-100", "100-200", "200-500", "500+"];

export function MetricsPanel({ metrics }: MetricsPanelProps) {
  const maxBucket = Math.max(...metrics.latencyHistogram, 1);

  return (
    <div style={styles.container}>
      <div style={styles.metric}>
        <div style={styles.label}>Total Requests</div>
        <div style={styles.value}>{metrics.totalRequests.toLocaleString()}</div>
      </div>

      <div style={styles.metric}>
        <div style={styles.label}>Success Rate</div>
        <div style={styles.value}>
          {metrics.successRate}
          <span style={styles.unit}>%</span>
        </div>
      </div>

      <div style={styles.metric}>
        <div style={styles.label}>Avg Latency</div>
        <div style={styles.value}>
          {metrics.avgLatencyMs}
          <span style={styles.unit}>ms</span>
        </div>
      </div>

      <div style={styles.metric}>
        <div style={styles.label}>p95 Latency</div>
        <div style={styles.value}>
          {metrics.p95LatencyMs}
          <span style={styles.unit}>ms</span>
        </div>
      </div>

      <div style={styles.metric}>
        <div style={styles.label}>p99 Latency</div>
        <div style={styles.value}>
          {metrics.p99LatencyMs}
          <span style={styles.unit}>ms</span>
        </div>
      </div>

      <div style={styles.metric}>
        <div style={styles.label}>Active Agents</div>
        <div style={styles.value}>{metrics.agents.size}</div>
      </div>

      <div style={styles.histogram}>
        <div style={styles.histogramTitle}>Latency Distribution</div>
        <div style={styles.bars}>
          {metrics.latencyHistogram.map((count, idx) => (
            <div key={idx} style={styles.barContainer}>
              <div style={styles.barValue}>{count}</div>
              <div style={styles.bar((count / maxBucket) * 100)} />
              <div style={styles.barLabel}>{BUCKET_LABELS[idx]}</div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

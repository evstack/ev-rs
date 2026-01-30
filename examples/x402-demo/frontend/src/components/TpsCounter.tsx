interface TpsCounterProps {
  value: number;
}

const styles = {
  container: {
    display: "flex",
    alignItems: "baseline",
    gap: 4,
    padding: "8px 16px",
    backgroundColor: "#1a1a1a",
    borderRadius: 8,
  },
  value: {
    fontSize: 32,
    fontWeight: 700,
    fontVariantNumeric: "tabular-nums",
    color: "#3b82f6",
  },
  label: {
    fontSize: 14,
    color: "#888",
  },
};

export function TpsCounter({ value }: TpsCounterProps) {
  return (
    <div style={styles.container}>
      <span style={styles.value}>{value.toFixed(1)}</span>
      <span style={styles.label}>TPS</span>
    </div>
  );
}

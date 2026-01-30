import { useRef, useEffect } from "react";
import type { PaymentEvent } from "../hooks/useEventStream";

interface PaymentStreamProps {
  events: PaymentEvent[];
}

const styles = {
  container: {
    maxHeight: 400,
    overflowY: "auto" as const,
    fontFamily: "monospace",
    fontSize: 12,
  },
  event: (type: PaymentEvent["type"]) => ({
    padding: "6px 8px",
    borderBottom: "1px solid #333",
    display: "flex",
    alignItems: "center",
    gap: 8,
    backgroundColor:
      type === "payment_failed" || type === "error"
        ? "rgba(239, 68, 68, 0.1)"
        : type === "request_served"
          ? "rgba(34, 197, 94, 0.05)"
          : "transparent",
  }),
  timestamp: {
    color: "#666",
    flexShrink: 0,
    width: 80,
  },
  type: (type: PaymentEvent["type"]) => ({
    color:
      type === "payment_submitted"
        ? "#f59e0b"
        : type === "payment_confirmed"
          ? "#3b82f6"
          : type === "request_served"
            ? "#22c55e"
            : type === "payment_failed" || type === "error"
              ? "#ef4444"
              : "#888",
    flexShrink: 0,
    width: 110,
  }),
  agent: {
    color: "#888",
    flexShrink: 0,
    width: 90,
  },
  details: {
    color: "#aaa",
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap" as const,
  },
  empty: {
    color: "#666",
    textAlign: "center" as const,
    padding: 32,
  },
};

function formatTime(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function formatEventDetails(event: PaymentEvent): string {
  switch (event.type) {
    case "payment_submitted":
      return `${event.amount} wei → ${event.to?.slice(0, 10)}...`;
    case "payment_confirmed":
      return event.txHash ? `tx: ${event.txHash.slice(0, 14)}...` : "";
    case "request_served":
      return `${event.endpoint} (${event.latencyMs}ms)`;
    case "payment_failed":
    case "error":
      return event.error ?? "Unknown error";
    default:
      return "";
  }
}

export function PaymentStream({ events }: PaymentStreamProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const shouldScrollRef = useRef(true);

  // Auto-scroll to bottom when new events arrive
  useEffect(() => {
    if (containerRef.current && shouldScrollRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [events]);

  // Detect if user has scrolled up
  const handleScroll = () => {
    if (containerRef.current) {
      const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
      shouldScrollRef.current = scrollHeight - scrollTop - clientHeight < 50;
    }
  };

  // Show only recent events (last 100)
  const recentEvents = events.slice(-100);

  if (recentEvents.length === 0) {
    return <div style={styles.empty}>Waiting for events...</div>;
  }

  return (
    <div ref={containerRef} style={styles.container} onScroll={handleScroll}>
      {recentEvents.map((event, idx) => (
        <div key={`${event.timestamp}-${idx}`} style={styles.event(event.type)}>
          <span style={styles.timestamp}>{formatTime(event.timestamp)}</span>
          <span style={styles.type(event.type)}>{event.type}</span>
          <span style={styles.agent}>{event.agentId}</span>
          <span style={styles.details}>{formatEventDetails(event)}</span>
        </div>
      ))}
    </div>
  );
}

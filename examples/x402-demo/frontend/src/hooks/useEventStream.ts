import { useState, useEffect, useRef, useCallback } from "react";

export interface PaymentEvent {
  type:
    | "payment_submitted"
    | "payment_confirmed"
    | "payment_failed"
    | "request_served"
    | "agent_connected"
    | "agent_disconnected"
    | "error";
  timestamp: number;
  agentId: string;
  txHash?: string;
  amount?: string;
  endpoint?: string;
  latencyMs?: number;
  error?: string;
  from?: string;
  to?: string;
}

interface HistoryMessage {
  type: "history";
  events: PaymentEvent[];
}

type WebSocketMessage = PaymentEvent | HistoryMessage;

export interface UseEventStreamReturn {
  events: PaymentEvent[];
  connected: boolean;
  error: string | null;
  clearEvents: () => void;
}

export function useEventStream(url: string): UseEventStreamReturn {
  const [events, setEvents] = useState<PaymentEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout>>();

  const connect = useCallback(() => {
    if (wsRef.current?.readyState === WebSocket.OPEN) return;

    try {
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        setConnected(true);
        setError(null);
        console.log("WebSocket connected");
      };

      ws.onclose = () => {
        setConnected(false);
        console.log("WebSocket disconnected, reconnecting...");
        // Reconnect after 2 seconds
        reconnectTimeoutRef.current = setTimeout(connect, 2000);
      };

      ws.onerror = (e) => {
        setError("WebSocket connection error");
        console.error("WebSocket error:", e);
      };

      ws.onmessage = (e) => {
        try {
          const message = JSON.parse(e.data) as WebSocketMessage;

          if ("type" in message && message.type === "history") {
            // Initial history load
            setEvents(message.events);
          } else {
            // Single event
            setEvents((prev) => {
              const newEvents = [...prev, message as PaymentEvent];
              // Keep last 500 events
              return newEvents.slice(-500);
            });
          }
        } catch (err) {
          console.error("Failed to parse WebSocket message:", err);
        }
      };
    } catch (err) {
      setError("Failed to connect to WebSocket");
      console.error("WebSocket connection failed:", err);
    }
  }, [url]);

  useEffect(() => {
    connect();

    return () => {
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, [connect]);

  const clearEvents = useCallback(() => {
    setEvents([]);
  }, []);

  return { events, connected, error, clearEvents };
}

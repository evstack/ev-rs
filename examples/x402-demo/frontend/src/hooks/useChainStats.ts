import { useEffect, useState } from "react";
import {
  ApiError,
  getChainStats,
  getEventsSummary,
  getHealth,
  getLatestBlockTxCountViaRpc,
  type ChainStats,
} from "../lib/api";

export interface UseChainStatsResult {
  data: ChainStats | null;
  error: string | null;
  loading: boolean;
}

export function useChainStats(intervalMs: number = 1000): UseChainStatsResult {
  const [data, setData] = useState<ChainStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    const fetchStats = async () => {
      try {
        let stats: ChainStats;
        try {
          stats = await getChainStats();
        } catch (err) {
          // Backward compatibility with older server versions lacking /api/chain.
          if (!(err instanceof ApiError) || err.status !== 404) {
            throw err;
          }

          const [health, events, latestBlockTxCount] = await Promise.all([
            getHealth(),
            getEventsSummary(),
            getLatestBlockTxCountViaRpc(),
          ]);
          stats = {
            chainId: health.chain.id,
            blockNumber: health.chain.blockNumber,
            latestBlockTxCount,
            latestBlockTimestamp: null,
            observedPaymentTxs: events.metrics.totalPayments,
            observedServedRequests: events.metrics.totalRequests,
          };
        }

        if (!cancelled) {
          setData(stats);
          setError(null);
          setLoading(false);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to fetch chain stats");
          setLoading(false);
        }
      }
    };

    fetchStats();
    const timer = setInterval(fetchStats, intervalMs);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [intervalMs]);

  return { data, error, loading };
}

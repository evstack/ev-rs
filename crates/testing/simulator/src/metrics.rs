//! Performance metrics collection for simulation.
//!
//! This module provides comprehensive metrics tracking for simulation runs,
//! including block/transaction statistics, timing information, and resource usage.

use crate::time::PerformanceTimer;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Configuration for metrics collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Maximum number of block metrics to retain (for sliding window).
    pub max_block_history: usize,
    /// Whether to track detailed per-transaction metrics.
    pub track_tx_details: bool,
    /// Whether to track storage operation counts.
    pub track_storage_ops: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            max_block_history: 1000,
            track_tx_details: true,
            track_storage_ops: true,
        }
    }
}

/// Metrics for a single block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockMetrics {
    /// Block height.
    pub height: u64,
    /// Number of transactions in the block.
    pub tx_count: u64,
    /// Number of successful transactions.
    pub tx_success: u64,
    /// Number of failed transactions.
    pub tx_failed: u64,
    /// Total gas consumed in the block.
    pub gas_consumed: u64,
    /// Wall-clock execution time in nanoseconds.
    pub execution_time_ns: u64,
    /// Number of storage reads.
    pub storage_reads: u64,
    /// Number of storage writes.
    pub storage_writes: u64,
    /// Number of events emitted.
    pub events_emitted: u64,
    /// State size delta in bytes (positive = growth, negative = shrink).
    pub state_size_delta: i64,
}

/// Metrics for a single transaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TxMetrics {
    /// Transaction index within block.
    pub index: u64,
    /// Whether the transaction succeeded.
    pub success: bool,
    /// Gas consumed by this transaction.
    pub gas_consumed: u64,
    /// Execution time in nanoseconds.
    pub execution_time_ns: u64,
    /// Number of storage reads.
    pub storage_reads: u64,
    /// Number of storage writes.
    pub storage_writes: u64,
    /// Number of events emitted.
    pub events_emitted: u64,
}

/// Aggregated simulation metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimMetrics {
    /// Total blocks executed.
    pub blocks_executed: u64,
    /// Total transactions executed.
    pub txs_executed: u64,
    /// Total successful transactions.
    pub txs_success: u64,
    /// Total failed transactions.
    pub txs_failed: u64,
    /// Total gas consumed.
    pub gas_consumed: u64,
    /// Total storage reads.
    pub storage_reads: u64,
    /// Total storage writes.
    pub storage_writes: u64,
    /// Total events emitted.
    pub events_emitted: u64,
    /// Total wall-clock execution time in nanoseconds.
    pub execution_time_ns: u64,
    /// Current state size in bytes.
    pub state_size_bytes: u64,
    /// Peak state size in bytes.
    pub peak_state_size_bytes: u64,
    /// Simulated time elapsed in milliseconds.
    pub simulated_time_ms: u64,
    /// Per-block metrics history.
    #[serde(skip)]
    block_metrics: VecDeque<BlockMetrics>,
    /// Configuration.
    #[serde(skip)]
    config: MetricsConfig,
}

impl SimMetrics {
    /// Creates a new metrics collector with default configuration.
    pub fn new() -> Self {
        Self::with_config(MetricsConfig::default())
    }

    /// Creates a new metrics collector with the given configuration.
    pub fn with_config(config: MetricsConfig) -> Self {
        Self {
            block_metrics: VecDeque::with_capacity(config.max_block_history),
            config,
            ..Default::default()
        }
    }

    /// Records metrics for a completed block.
    pub fn record_block(&mut self, metrics: BlockMetrics) {
        // Update totals
        self.blocks_executed = self.blocks_executed.saturating_add(1);
        self.txs_executed = self.txs_executed.saturating_add(metrics.tx_count);
        self.txs_success = self.txs_success.saturating_add(metrics.tx_success);
        self.txs_failed = self.txs_failed.saturating_add(metrics.tx_failed);
        self.gas_consumed = self.gas_consumed.saturating_add(metrics.gas_consumed);
        self.storage_reads = self.storage_reads.saturating_add(metrics.storage_reads);
        self.storage_writes = self.storage_writes.saturating_add(metrics.storage_writes);
        self.events_emitted = self.events_emitted.saturating_add(metrics.events_emitted);
        self.execution_time_ns = self
            .execution_time_ns
            .saturating_add(metrics.execution_time_ns);

        // Update state size
        if metrics.state_size_delta >= 0 {
            self.state_size_bytes = self
                .state_size_bytes
                .saturating_add(metrics.state_size_delta as u64);
        } else {
            self.state_size_bytes = self
                .state_size_bytes
                .saturating_sub((-metrics.state_size_delta) as u64);
        }
        self.peak_state_size_bytes = self.peak_state_size_bytes.max(self.state_size_bytes);

        // Store block metrics
        if self.block_metrics.len() >= self.config.max_block_history {
            self.block_metrics.pop_front();
        }
        self.block_metrics.push_back(metrics);
    }

    /// Updates simulated time.
    pub fn set_simulated_time_ms(&mut self, ms: u64) {
        self.simulated_time_ms = ms;
    }

    /// Returns the block metrics history.
    pub fn block_history(&self) -> &VecDeque<BlockMetrics> {
        &self.block_metrics
    }

    /// Returns the most recent block metrics.
    pub fn last_block(&self) -> Option<&BlockMetrics> {
        self.block_metrics.back()
    }

    /// Generates a performance report.
    pub fn generate_report(&self) -> PerformanceReport {
        let blocks = self.blocks_executed.max(1) as f64;
        let txs = self.txs_executed.max(1) as f64;
        let exec_time_secs = self.execution_time_ns as f64 / 1_000_000_000.0;

        // Calculate percentiles for block times
        let mut block_times: Vec<u64> = self
            .block_metrics
            .iter()
            .map(|b| b.execution_time_ns)
            .collect();
        block_times.sort_unstable();

        let p50_block_time_ns = percentile(&block_times, 0.50);
        let p99_block_time_ns = percentile(&block_times, 0.99);

        PerformanceReport {
            total_blocks: self.blocks_executed,
            total_txs: self.txs_executed,
            success_rate: self.txs_success as f64 / txs,
            avg_txs_per_block: self.txs_executed as f64 / blocks,
            avg_gas_per_tx: self.gas_consumed as f64 / txs,
            avg_block_time_ms: (self.execution_time_ns as f64 / blocks) / 1_000_000.0,
            p50_block_time_ms: p50_block_time_ns as f64 / 1_000_000.0,
            p99_block_time_ms: p99_block_time_ns as f64 / 1_000_000.0,
            state_size_bytes: self.state_size_bytes,
            peak_state_size_bytes: self.peak_state_size_bytes,
            storage_ops_per_tx: (self.storage_reads + self.storage_writes) as f64 / txs,
            events_per_tx: self.events_emitted as f64 / txs,
            wall_clock_secs: exec_time_secs,
            simulated_time_secs: self.simulated_time_ms as f64 / 1000.0,
            time_acceleration: if exec_time_secs > 0.0 {
                (self.simulated_time_ms as f64 / 1000.0) / exec_time_secs
            } else {
                0.0
            },
            blocks_per_second: if exec_time_secs > 0.0 {
                blocks / exec_time_secs
            } else {
                0.0
            },
            txs_per_second: if exec_time_secs > 0.0 {
                txs / exec_time_secs
            } else {
                0.0
            },
        }
    }

    /// Resets all metrics.
    pub fn reset(&mut self) {
        *self = Self::with_config(self.config.clone());
    }

    /// Creates a snapshot of current metrics.
    pub fn snapshot(&self) -> SimMetricsSnapshot {
        SimMetricsSnapshot {
            blocks_executed: self.blocks_executed,
            txs_executed: self.txs_executed,
            txs_success: self.txs_success,
            txs_failed: self.txs_failed,
            gas_consumed: self.gas_consumed,
            execution_time_ns: self.execution_time_ns,
            state_size_bytes: self.state_size_bytes,
        }
    }
}

/// A snapshot of key metrics for comparison.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimMetricsSnapshot {
    pub blocks_executed: u64,
    pub txs_executed: u64,
    pub txs_success: u64,
    pub txs_failed: u64,
    pub gas_consumed: u64,
    pub execution_time_ns: u64,
    pub state_size_bytes: u64,
}

impl SimMetricsSnapshot {
    /// Computes the delta between this snapshot and another.
    pub fn delta(&self, other: &SimMetricsSnapshot) -> SimMetricsSnapshot {
        SimMetricsSnapshot {
            blocks_executed: self.blocks_executed.saturating_sub(other.blocks_executed),
            txs_executed: self.txs_executed.saturating_sub(other.txs_executed),
            txs_success: self.txs_success.saturating_sub(other.txs_success),
            txs_failed: self.txs_failed.saturating_sub(other.txs_failed),
            gas_consumed: self.gas_consumed.saturating_sub(other.gas_consumed),
            execution_time_ns: self.execution_time_ns.saturating_sub(other.execution_time_ns),
            state_size_bytes: self.state_size_bytes,
        }
    }
}

/// Summary performance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    /// Total blocks processed.
    pub total_blocks: u64,
    /// Total transactions processed.
    pub total_txs: u64,
    /// Transaction success rate (0.0 to 1.0).
    pub success_rate: f64,
    /// Average transactions per block.
    pub avg_txs_per_block: f64,
    /// Average gas consumed per transaction.
    pub avg_gas_per_tx: f64,
    /// Average block processing time in milliseconds.
    pub avg_block_time_ms: f64,
    /// 50th percentile block processing time in milliseconds.
    pub p50_block_time_ms: f64,
    /// 99th percentile block processing time in milliseconds.
    pub p99_block_time_ms: f64,
    /// Current state size in bytes.
    pub state_size_bytes: u64,
    /// Peak state size in bytes.
    pub peak_state_size_bytes: u64,
    /// Average storage operations per transaction.
    pub storage_ops_per_tx: f64,
    /// Average events emitted per transaction.
    pub events_per_tx: f64,
    /// Total wall-clock time in seconds.
    pub wall_clock_secs: f64,
    /// Total simulated time in seconds.
    pub simulated_time_secs: f64,
    /// Time acceleration factor (simulated/wall-clock).
    pub time_acceleration: f64,
    /// Blocks processed per second (wall-clock).
    pub blocks_per_second: f64,
    /// Transactions processed per second (wall-clock).
    pub txs_per_second: f64,
}

impl PerformanceReport {
    /// Formats the report as a human-readable string.
    pub fn to_string_pretty(&self) -> String {
        format!(
            r#"Performance Report
==================
Blocks:          {} ({:.1} blocks/sec)
Transactions:    {} ({:.1} tx/sec, {:.1}% success)
Avg Txs/Block:   {:.1}
Avg Gas/Tx:      {:.0}
Block Time:      {:.2}ms avg, {:.2}ms p50, {:.2}ms p99
State Size:      {} bytes (peak: {} bytes)
Storage Ops/Tx:  {:.1}
Events/Tx:       {:.1}
Wall Clock:      {:.2}s
Simulated Time:  {:.2}s
Time Accel:      {:.0}x"#,
            self.total_blocks,
            self.blocks_per_second,
            self.total_txs,
            self.txs_per_second,
            self.success_rate * 100.0,
            self.avg_txs_per_block,
            self.avg_gas_per_tx,
            self.avg_block_time_ms,
            self.p50_block_time_ms,
            self.p99_block_time_ms,
            self.state_size_bytes,
            self.peak_state_size_bytes,
            self.storage_ops_per_tx,
            self.events_per_tx,
            self.wall_clock_secs,
            self.simulated_time_secs,
            self.time_acceleration
        )
    }

    /// Exports to CSV format.
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{},{},{:.4},{:.2},{:.0},{:.4},{:.4},{:.4},{},{},{:.2},{:.2},{:.2},{:.0},{:.2},{:.2}",
            self.total_blocks,
            self.total_txs,
            (self.success_rate * self.total_txs as f64) as u64,
            self.success_rate,
            self.avg_txs_per_block,
            self.avg_gas_per_tx,
            self.avg_block_time_ms,
            self.p50_block_time_ms,
            self.p99_block_time_ms,
            self.state_size_bytes,
            self.peak_state_size_bytes,
            self.storage_ops_per_tx,
            self.events_per_tx,
            self.wall_clock_secs,
            self.time_acceleration,
            self.blocks_per_second,
            self.txs_per_second
        )
    }

    /// Returns CSV header line.
    pub fn csv_header() -> &'static str {
        "blocks,txs,txs_success,success_rate,avg_txs_per_block,avg_gas_per_tx,avg_block_time_ms,p50_block_time_ms,p99_block_time_ms,state_size_bytes,peak_state_size_bytes,storage_ops_per_tx,events_per_tx,wall_clock_secs,time_acceleration,blocks_per_sec,txs_per_sec"
    }
}

/// Helper to build block metrics during execution.
#[derive(Debug, Default)]
pub struct BlockMetricsBuilder {
    metrics: BlockMetrics,
    timer: Option<PerformanceTimer>,
}

impl BlockMetricsBuilder {
    /// Creates a new builder for the given block height.
    pub fn new(height: u64) -> Self {
        Self {
            metrics: BlockMetrics {
                height,
                ..Default::default()
            },
            timer: Some(PerformanceTimer::start()),
        }
    }

    /// Records a successful transaction.
    pub fn record_tx_success(&mut self, gas: u64, events: u64) {
        self.metrics.tx_count = self.metrics.tx_count.saturating_add(1);
        self.metrics.tx_success = self.metrics.tx_success.saturating_add(1);
        self.metrics.gas_consumed = self.metrics.gas_consumed.saturating_add(gas);
        self.metrics.events_emitted = self.metrics.events_emitted.saturating_add(events);
    }

    /// Records a failed transaction.
    pub fn record_tx_failure(&mut self, gas: u64) {
        self.metrics.tx_count = self.metrics.tx_count.saturating_add(1);
        self.metrics.tx_failed = self.metrics.tx_failed.saturating_add(1);
        self.metrics.gas_consumed = self.metrics.gas_consumed.saturating_add(gas);
    }

    /// Records storage operations.
    pub fn record_storage_ops(&mut self, reads: u64, writes: u64) {
        self.metrics.storage_reads = self.metrics.storage_reads.saturating_add(reads);
        self.metrics.storage_writes = self.metrics.storage_writes.saturating_add(writes);
    }

    /// Sets the state size delta.
    pub fn set_state_size_delta(&mut self, delta: i64) {
        self.metrics.state_size_delta = delta;
    }

    /// Finishes building and returns the completed metrics.
    pub fn finish(mut self) -> BlockMetrics {
        if let Some(timer) = self.timer.take() {
            self.metrics.execution_time_ns = timer.elapsed_ns();
        }
        self.metrics
    }
}

/// Calculates percentile from sorted values.
fn percentile(sorted_values: &[u64], p: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let idx = ((sorted_values.len() as f64 * p) as usize).min(sorted_values.len() - 1);
    sorted_values[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_accumulation() {
        let mut metrics = SimMetrics::new();

        let block1 = BlockMetrics {
            height: 1,
            tx_count: 10,
            tx_success: 8,
            tx_failed: 2,
            gas_consumed: 1000,
            ..Default::default()
        };

        let block2 = BlockMetrics {
            height: 2,
            tx_count: 5,
            tx_success: 5,
            tx_failed: 0,
            gas_consumed: 500,
            ..Default::default()
        };

        metrics.record_block(block1);
        metrics.record_block(block2);

        assert_eq!(metrics.blocks_executed, 2);
        assert_eq!(metrics.txs_executed, 15);
        assert_eq!(metrics.txs_success, 13);
        assert_eq!(metrics.txs_failed, 2);
        assert_eq!(metrics.gas_consumed, 1500);
    }

    #[test]
    fn test_performance_report() {
        let mut metrics = SimMetrics::new();

        for i in 0..10 {
            metrics.record_block(BlockMetrics {
                height: i,
                tx_count: 100,
                tx_success: 95,
                tx_failed: 5,
                gas_consumed: 10000,
                execution_time_ns: 1_000_000, // 1ms
                ..Default::default()
            });
        }
        metrics.set_simulated_time_ms(10_000);

        let report = metrics.generate_report();
        assert_eq!(report.total_blocks, 10);
        assert_eq!(report.total_txs, 1000);
        assert!((report.success_rate - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_block_metrics_builder() {
        let mut builder = BlockMetricsBuilder::new(1);

        builder.record_tx_success(100, 2);
        builder.record_tx_success(150, 3);
        builder.record_tx_failure(50);
        builder.record_storage_ops(10, 5);

        let metrics = builder.finish();

        assert_eq!(metrics.height, 1);
        assert_eq!(metrics.tx_count, 3);
        assert_eq!(metrics.tx_success, 2);
        assert_eq!(metrics.tx_failed, 1);
        assert_eq!(metrics.gas_consumed, 300);
        assert_eq!(metrics.events_emitted, 5);
        assert_eq!(metrics.storage_reads, 10);
        assert_eq!(metrics.storage_writes, 5);
    }

    #[test]
    fn test_metrics_snapshot_delta() {
        let snapshot1 = SimMetricsSnapshot {
            blocks_executed: 10,
            txs_executed: 100,
            txs_success: 90,
            txs_failed: 10,
            gas_consumed: 10000,
            execution_time_ns: 1000000,
            state_size_bytes: 5000,
        };

        let snapshot2 = SimMetricsSnapshot {
            blocks_executed: 5,
            txs_executed: 50,
            txs_success: 45,
            txs_failed: 5,
            gas_consumed: 5000,
            execution_time_ns: 500000,
            state_size_bytes: 3000,
        };

        let delta = snapshot1.delta(&snapshot2);
        assert_eq!(delta.blocks_executed, 5);
        assert_eq!(delta.txs_executed, 50);
        assert_eq!(delta.gas_consumed, 5000);
    }
}

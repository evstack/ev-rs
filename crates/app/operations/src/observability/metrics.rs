//! Prometheus-compatible metrics collection.
//!
//! This module provides a metrics registry for tracking node performance
//! and exporting to Prometheus format.

use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicI64;

/// Node-level metrics for monitoring.
pub struct NodeMetrics {
    /// Total blocks processed.
    pub blocks_processed: Counter,
    /// Current block height.
    pub block_height: Gauge,
    /// Block execution duration in seconds.
    pub block_execution_duration_seconds: Histogram,
    /// Total transactions processed.
    pub txs_processed: Counter,
    /// Total gas used across all transactions.
    pub gas_used: Counter,
    /// Total storage read operations.
    pub storage_reads: Counter,
    /// Total storage write operations.
    pub storage_writes: Counter,
}

impl Default for NodeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeMetrics {
    /// Create a new set of node metrics.
    pub fn new() -> Self {
        // Buckets for block execution: 1ms to 10s
        let duration_buckets = exponential_buckets(0.001, 2.0, 14);

        Self {
            blocks_processed: Counter::default(),
            block_height: Gauge::<i64, AtomicI64>::default(),
            block_execution_duration_seconds: Histogram::new(duration_buckets),
            txs_processed: Counter::default(),
            gas_used: Counter::default(),
            storage_reads: Counter::default(),
            storage_writes: Counter::default(),
        }
    }

    /// Record metrics from a block execution.
    pub fn record_block(
        &self,
        height: u64,
        tx_count: u64,
        gas_used: u64,
        duration_secs: f64,
        storage_reads: u64,
        storage_writes: u64,
    ) {
        self.blocks_processed.inc();
        self.block_height.set(height as i64);
        self.block_execution_duration_seconds.observe(duration_secs);
        self.txs_processed.inc_by(tx_count);
        self.gas_used.inc_by(gas_used);
        self.storage_reads.inc_by(storage_reads);
        self.storage_writes.inc_by(storage_writes);
    }
}

/// Central metrics registry for the node.
pub struct MetricsRegistry {
    registry: Registry,
    /// Node-level metrics.
    pub node: NodeMetrics,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    /// Create a new metrics registry with all metrics registered.
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let node = NodeMetrics::new();

        // Register all metrics with the registry
        registry.register(
            "evolve_blocks_processed",
            "Total number of blocks processed",
            node.blocks_processed.clone(),
        );

        registry.register(
            "evolve_block_height",
            "Current block height",
            node.block_height.clone(),
        );

        registry.register(
            "evolve_block_execution_duration_seconds",
            "Block execution duration in seconds",
            node.block_execution_duration_seconds.clone(),
        );

        registry.register(
            "evolve_txs_processed",
            "Total number of transactions processed",
            node.txs_processed.clone(),
        );

        registry.register("evolve_gas_used", "Total gas used", node.gas_used.clone());

        registry.register(
            "evolve_storage_reads",
            "Total storage read operations",
            node.storage_reads.clone(),
        );

        registry.register(
            "evolve_storage_writes",
            "Total storage write operations",
            node.storage_writes.clone(),
        );

        Self { registry, node }
    }

    /// Encode all metrics in Prometheus text format.
    pub fn encode_prometheus(&self) -> String {
        let mut buffer = String::new();
        if encode(&mut buffer, &self.registry).is_err() {
            return String::from("# Error encoding metrics\n");
        }
        buffer
    }

    /// Get a reference to the underlying registry for custom metric registration.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Get a mutable reference to the underlying registry.
    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_creation() {
        let registry = MetricsRegistry::new();
        let encoded = registry.encode_prometheus();

        // Should contain our registered metrics
        assert!(encoded.contains("evolve_blocks_processed"));
        assert!(encoded.contains("evolve_block_height"));
        assert!(encoded.contains("evolve_txs_processed"));
    }

    #[test]
    fn test_record_block() {
        let registry = MetricsRegistry::new();

        registry.node.record_block(
            100,   // height
            5,     // tx_count
            21000, // gas_used
            0.05,  // duration_secs
            100,   // storage_reads
            50,    // storage_writes
        );

        let encoded = registry.encode_prometheus();
        assert!(encoded.contains("100")); // block height
    }
}

//! Storage metrics for monitoring storage layer performance.
//!
//! This module provides Prometheus-compatible metrics for the storage layer,
//! tracking read/write latencies, cache performance, and batch operations.

// Instant is used for performance metrics, not consensus-affecting logic.
#![allow(clippy::disallowed_types)]

use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Instant;

/// Storage-specific metrics for monitoring the storage layer.
///
/// Tracks:
/// - Read/write/commit latencies
/// - Cache hit/miss rates
/// - Batch operation sizes
/// - Current cache size
#[derive(Clone)]
pub struct StorageMetrics {
    /// Read operation latency in seconds.
    pub read_latency_seconds: Histogram,
    /// Write/batch operation latency in seconds.
    pub write_latency_seconds: Histogram,
    /// Commit operation latency in seconds.
    pub commit_latency_seconds: Histogram,
    /// Cache hit count.
    pub cache_hits: Counter,
    /// Cache miss count.
    pub cache_misses: Counter,
    /// Current cache size (number of entries).
    pub cache_size: Gauge,
    /// Batch size histogram (number of operations per batch).
    pub batch_size: Histogram,
    /// Total keys stored (estimated).
    pub keys_total: Counter,
    /// Total keys deleted.
    pub keys_deleted: Counter,
}

impl Default for StorageMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageMetrics {
    /// Create a new set of storage metrics.
    pub fn new() -> Self {
        // Buckets for storage operations: 10us to 1s
        let read_buckets = exponential_buckets(0.00001, 2.0, 18);
        let write_buckets = exponential_buckets(0.00001, 2.0, 18);
        let commit_buckets = exponential_buckets(0.0001, 2.0, 16);
        // Buckets for batch sizes: 1 to 16384
        let batch_buckets = exponential_buckets(1.0, 2.0, 15);

        Self {
            read_latency_seconds: Histogram::new(read_buckets),
            write_latency_seconds: Histogram::new(write_buckets),
            commit_latency_seconds: Histogram::new(commit_buckets),
            cache_hits: Counter::default(),
            cache_misses: Counter::default(),
            cache_size: Gauge::<i64, AtomicI64>::default(),
            batch_size: Histogram::new(batch_buckets),
            keys_total: Counter::default(),
            keys_deleted: Counter::default(),
        }
    }

    /// Record a cache hit.
    #[inline]
    pub fn record_cache_hit(&self) {
        self.cache_hits.inc();
    }

    /// Record a cache miss.
    #[inline]
    pub fn record_cache_miss(&self) {
        self.cache_misses.inc();
    }

    /// Record a read operation latency.
    #[inline]
    pub fn record_read_latency(&self, latency_secs: f64) {
        self.read_latency_seconds.observe(latency_secs);
    }

    /// Record a batch write operation.
    #[inline]
    pub fn record_batch(&self, latency_secs: f64, ops_count: usize, sets: usize, deletes: usize) {
        self.write_latency_seconds.observe(latency_secs);
        self.batch_size.observe(ops_count as f64);
        self.keys_total.inc_by(sets as u64);
        self.keys_deleted.inc_by(deletes as u64);
    }

    /// Record a commit operation.
    #[inline]
    pub fn record_commit(&self, latency_secs: f64) {
        self.commit_latency_seconds.observe(latency_secs);
    }

    /// Update the cache size gauge.
    #[inline]
    pub fn set_cache_size(&self, size: usize) {
        self.cache_size.set(size as i64);
    }

    /// Register all metrics with a Prometheus registry.
    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "evolve_storage_read_latency_seconds",
            "Storage read operation latency in seconds",
            self.read_latency_seconds.clone(),
        );

        registry.register(
            "evolve_storage_write_latency_seconds",
            "Storage write/batch operation latency in seconds",
            self.write_latency_seconds.clone(),
        );

        registry.register(
            "evolve_storage_commit_latency_seconds",
            "Storage commit operation latency in seconds",
            self.commit_latency_seconds.clone(),
        );

        registry.register(
            "evolve_storage_cache_hits_total",
            "Total storage cache hits",
            self.cache_hits.clone(),
        );

        registry.register(
            "evolve_storage_cache_misses_total",
            "Total storage cache misses",
            self.cache_misses.clone(),
        );

        registry.register(
            "evolve_storage_cache_size",
            "Current storage cache size (entries)",
            self.cache_size.clone(),
        );

        registry.register(
            "evolve_storage_batch_size",
            "Storage batch operation size",
            self.batch_size.clone(),
        );

        registry.register(
            "evolve_storage_keys_total",
            "Total keys written to storage",
            self.keys_total.clone(),
        );

        registry.register(
            "evolve_storage_keys_deleted_total",
            "Total keys deleted from storage",
            self.keys_deleted.clone(),
        );
    }

    /// Encode metrics in Prometheus text format.
    pub fn encode_prometheus(&self) -> String {
        let mut registry = Registry::default();
        self.register(&mut registry);

        let mut buffer = String::new();
        if encode(&mut buffer, &registry).is_err() {
            return String::from("# Error encoding metrics\n");
        }
        buffer
    }
}

/// A guard that measures elapsed time and records it when dropped.
pub struct LatencyGuard<'a, F: Fn(f64)> {
    start: Instant,
    record_fn: &'a F,
}

impl<'a, F: Fn(f64)> LatencyGuard<'a, F> {
    /// Create a new latency guard.
    pub fn new(record_fn: &'a F) -> Self {
        Self {
            start: Instant::now(),
            record_fn,
        }
    }
}

impl<F: Fn(f64)> Drop for LatencyGuard<'_, F> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        (self.record_fn)(elapsed);
    }
}

/// Wrapper for optional metrics that provides no-op implementations when metrics are disabled.
#[derive(Clone)]
pub struct OptionalMetrics(Option<Arc<StorageMetrics>>);

impl OptionalMetrics {
    /// Create with metrics enabled.
    pub fn enabled(metrics: Arc<StorageMetrics>) -> Self {
        Self(Some(metrics))
    }

    /// Create with metrics disabled (no-op).
    pub fn disabled() -> Self {
        Self(None)
    }

    /// Check if metrics are enabled.
    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    /// Get the inner metrics if enabled.
    pub fn inner(&self) -> Option<&Arc<StorageMetrics>> {
        self.0.as_ref()
    }

    #[inline]
    pub fn record_cache_hit(&self) {
        if let Some(m) = &self.0 {
            m.record_cache_hit();
        }
    }

    #[inline]
    pub fn record_cache_miss(&self) {
        if let Some(m) = &self.0 {
            m.record_cache_miss();
        }
    }

    #[inline]
    pub fn record_read_latency(&self, latency_secs: f64) {
        if let Some(m) = &self.0 {
            m.record_read_latency(latency_secs);
        }
    }

    #[inline]
    pub fn record_batch(&self, latency_secs: f64, ops_count: usize, sets: usize, deletes: usize) {
        if let Some(m) = &self.0 {
            m.record_batch(latency_secs, ops_count, sets, deletes);
        }
    }

    #[inline]
    pub fn record_commit(&self, latency_secs: f64) {
        if let Some(m) = &self.0 {
            m.record_commit(latency_secs);
        }
    }

    #[inline]
    pub fn set_cache_size(&self, size: usize) {
        if let Some(m) = &self.0 {
            m.set_cache_size(size);
        }
    }
}

impl Default for OptionalMetrics {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_operations() {
        let metrics = StorageMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        metrics.record_read_latency(0.001);
        metrics.record_batch(0.01, 100, 80, 20);
        metrics.record_commit(0.1);
        metrics.set_cache_size(500);

        let encoded = metrics.encode_prometheus();
        // prometheus-client adds _total suffix automatically to counters
        assert!(encoded.contains("evolve_storage_cache_hits_total"));
        assert!(encoded.contains("evolve_storage_cache_misses_total"));
        assert!(encoded.contains("evolve_storage_cache_size"));
    }

    #[test]
    fn test_optional_metrics_disabled() {
        let metrics = OptionalMetrics::disabled();
        assert!(!metrics.is_enabled());

        // These should be no-ops
        metrics.record_cache_hit();
        metrics.record_read_latency(0.001);
        metrics.record_batch(0.01, 100, 80, 20);
    }

    #[test]
    fn test_optional_metrics_enabled() {
        let inner = Arc::new(StorageMetrics::new());
        let metrics = OptionalMetrics::enabled(inner.clone());
        assert!(metrics.is_enabled());

        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let encoded = inner.encode_prometheus();
        // Verify metrics are recorded - prometheus-client may encode values as floats
        assert!(encoded.contains("evolve_storage_cache_hits_total"));
        assert!(encoded.contains("evolve_storage_cache_misses_total"));
    }

    #[test]
    fn test_latency_guard() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let recorded = Arc::new(AtomicU64::new(0));
        let recorded_clone = recorded.clone();

        let record_fn = move |latency: f64| {
            // Store latency as microseconds to avoid floating point in atomic
            recorded_clone.store((latency * 1_000_000.0) as u64, Ordering::SeqCst);
        };

        {
            let _guard = LatencyGuard::new(&record_fn);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Should have recorded something >= 10ms (10000us)
        let recorded_us = recorded.load(Ordering::SeqCst);
        assert!(
            recorded_us >= 10000,
            "Expected >= 10000us, got {recorded_us}us"
        );
    }
}

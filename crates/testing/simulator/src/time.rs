//! Simulated time management for deterministic testing.
//!
//! This module provides a virtual clock that can be controlled programmatically,
//! enabling time-based testing without real-world delays.

use serde::{Deserialize, Serialize};

/// Configuration for simulated time behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeConfig {
    /// Number of ticks per simulated block.
    pub ticks_per_block: u64,
    /// Simulated milliseconds per tick.
    pub tick_ms: u64,
    /// Initial block timestamp in milliseconds since epoch.
    pub initial_timestamp_ms: u64,
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            ticks_per_block: 100,
            tick_ms: 10, // 10ms per tick = 1s per block at 100 ticks
            initial_timestamp_ms: 1_700_000_000_000, // ~Nov 2023
        }
    }
}

/// Simulated time with tick-based progression.
///
/// Time advances in discrete ticks, with blocks occurring at fixed tick intervals.
/// This allows for deterministic time-based testing and time acceleration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedTime {
    /// Current tick count (monotonically increasing).
    current_tick: u64,
    /// Current block height.
    current_block: u64,
    /// Configuration for time progression.
    config: TimeConfig,
}

impl SimulatedTime {
    /// Creates a new simulated time with the given configuration.
    pub fn new(config: TimeConfig) -> Self {
        Self {
            current_tick: 0,
            current_block: 0,
            config,
        }
    }

    /// Creates a new simulated time with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(TimeConfig::default())
    }

    /// Returns the current tick count.
    #[inline]
    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// Returns the current block height.
    #[inline]
    pub fn block_height(&self) -> u64 {
        self.current_block
    }

    /// Returns the current simulated timestamp in milliseconds.
    #[inline]
    pub fn now_ms(&self) -> u64 {
        self.config
            .initial_timestamp_ms
            .saturating_add(self.current_tick.saturating_mul(self.config.tick_ms))
    }

    /// Returns the current simulated timestamp in seconds.
    #[inline]
    pub fn now_secs(&self) -> u64 {
        self.now_ms() / 1000
    }

    /// Advances time by one tick.
    ///
    /// Returns true if this tick completed a block boundary.
    pub fn advance_tick(&mut self) -> bool {
        self.current_tick = self.current_tick.saturating_add(1);

        // Check if we crossed a block boundary
        let ticks_in_block = self.current_tick % self.config.ticks_per_block;
        ticks_in_block == 0 && self.current_tick > 0
    }

    /// Advances time by multiple ticks.
    ///
    /// Returns the number of block boundaries crossed.
    pub fn advance_ticks(&mut self, n: u64) -> u64 {
        let old_block_tick = self.current_tick / self.config.ticks_per_block;
        self.current_tick = self.current_tick.saturating_add(n);
        let new_block_tick = self.current_tick / self.config.ticks_per_block;
        new_block_tick.saturating_sub(old_block_tick)
    }

    /// Advances to the next block, updating both tick and block counters.
    pub fn advance_block(&mut self) {
        self.current_block = self.current_block.saturating_add(1);

        // Advance ticks to align with block boundary
        let target_tick = self.current_block.saturating_mul(self.config.ticks_per_block);
        if self.current_tick < target_tick {
            self.current_tick = target_tick;
        }
    }

    /// Sets the block height directly.
    ///
    /// Also advances ticks to be consistent with the new block height.
    pub fn set_block_height(&mut self, height: u64) {
        self.current_block = height;
        let min_tick = height.saturating_mul(self.config.ticks_per_block);
        if self.current_tick < min_tick {
            self.current_tick = min_tick;
        }
    }

    /// Returns the tick at which the given block starts.
    #[inline]
    pub fn block_start_tick(&self, block: u64) -> u64 {
        block.saturating_mul(self.config.ticks_per_block)
    }

    /// Returns the tick at which the given block ends.
    #[inline]
    pub fn block_end_tick(&self, block: u64) -> u64 {
        (block.saturating_add(1)).saturating_mul(self.config.ticks_per_block)
    }

    /// Returns the milliseconds per block.
    #[inline]
    pub fn block_time_ms(&self) -> u64 {
        self.config.ticks_per_block.saturating_mul(self.config.tick_ms)
    }

    /// Returns a snapshot of the current time state.
    pub fn snapshot(&self) -> TimeSnapshot {
        TimeSnapshot {
            tick: self.current_tick,
            block: self.current_block,
            timestamp_ms: self.now_ms(),
        }
    }

    /// Restores from a snapshot.
    pub fn restore(&mut self, snapshot: &TimeSnapshot) {
        self.current_tick = snapshot.tick;
        self.current_block = snapshot.block;
    }

    /// Returns the time configuration.
    pub fn config(&self) -> &TimeConfig {
        &self.config
    }
}

/// A snapshot of simulated time state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeSnapshot {
    /// Tick count at snapshot.
    pub tick: u64,
    /// Block height at snapshot.
    pub block: u64,
    /// Timestamp in milliseconds at snapshot.
    pub timestamp_ms: u64,
}

/// Measures wall-clock time during simulation for performance tracking.
#[derive(Debug, Clone)]
pub struct PerformanceTimer {
    start: instant::Instant,
    elapsed_ns: u64,
    running: bool,
}

impl PerformanceTimer {
    /// Creates and starts a new timer.
    pub fn start() -> Self {
        Self {
            start: instant::Instant::now(),
            elapsed_ns: 0,
            running: true,
        }
    }

    /// Pauses the timer.
    pub fn pause(&mut self) {
        if self.running {
            self.elapsed_ns = self
                .elapsed_ns
                .saturating_add(self.start.elapsed().as_nanos() as u64);
            self.running = false;
        }
    }

    /// Resumes the timer.
    pub fn resume(&mut self) {
        if !self.running {
            self.start = instant::Instant::now();
            self.running = true;
        }
    }

    /// Returns total elapsed nanoseconds.
    pub fn elapsed_ns(&self) -> u64 {
        if self.running {
            self.elapsed_ns
                .saturating_add(self.start.elapsed().as_nanos() as u64)
        } else {
            self.elapsed_ns
        }
    }

    /// Returns elapsed milliseconds.
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ns() as f64 / 1_000_000.0
    }

    /// Returns elapsed seconds.
    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed_ns() as f64 / 1_000_000_000.0
    }
}

impl Default for PerformanceTimer {
    fn default() -> Self {
        Self::start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_progression() {
        let mut time = SimulatedTime::with_defaults();

        assert_eq!(time.current_tick(), 0);
        assert_eq!(time.block_height(), 0);

        // Advance one tick
        time.advance_tick();
        assert_eq!(time.current_tick(), 1);

        // Advance to next block
        time.advance_block();
        assert_eq!(time.block_height(), 1);
    }

    #[test]
    fn test_timestamp_calculation() {
        let config = TimeConfig {
            ticks_per_block: 10,
            tick_ms: 100,
            initial_timestamp_ms: 0,
        };
        let mut time = SimulatedTime::new(config);

        assert_eq!(time.now_ms(), 0);

        time.advance_tick();
        assert_eq!(time.now_ms(), 100);

        time.advance_ticks(9);
        assert_eq!(time.now_ms(), 1000);
    }

    #[test]
    fn test_block_boundaries() {
        let config = TimeConfig {
            ticks_per_block: 5,
            tick_ms: 10,
            initial_timestamp_ms: 0,
        };
        let mut time = SimulatedTime::new(config);

        // Advance until we cross a block boundary
        for i in 0..4 {
            let crossed = time.advance_tick();
            assert!(!crossed, "tick {i} should not cross boundary");
        }

        let crossed = time.advance_tick();
        assert!(crossed, "tick 5 should cross boundary");
    }

    #[test]
    fn test_snapshot_restore() {
        let mut time = SimulatedTime::with_defaults();

        time.advance_ticks(50);
        time.advance_block();

        let snapshot = time.snapshot();

        time.advance_ticks(100);
        time.advance_block();

        time.restore(&snapshot);

        assert_eq!(time.current_tick(), snapshot.tick);
        assert_eq!(time.block_height(), snapshot.block);
    }

    #[test]
    fn test_set_block_height() {
        let config = TimeConfig {
            ticks_per_block: 10,
            tick_ms: 10,
            initial_timestamp_ms: 0,
        };
        let mut time = SimulatedTime::new(config);

        time.set_block_height(5);
        assert_eq!(time.block_height(), 5);
        assert!(time.current_tick() >= 50);
    }
}

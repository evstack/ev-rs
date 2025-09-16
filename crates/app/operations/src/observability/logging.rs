//! Structured logging with runtime-adjustable log levels.
//!
//! Uses `slog` for performant structured logging with JSON output.
//! The `slog-stdlog` bridge captures existing `log::*!` macro calls.

use slog::{o, Drain, Level};
use slog_async::OverflowStrategy;
use std::sync::Arc;

pub use slog::Logger;
pub use slog_atomic::AtomicSwitch;

/// Handle for runtime log level adjustment.
pub struct LogLevelSwitch {
    ctrl: Arc<slog_atomic::AtomicSwitchCtrl<(), slog::Never>>,
    level: std::sync::atomic::AtomicU8,
}

impl LogLevelSwitch {
    /// Set the minimum log level at runtime.
    pub fn set_level(&self, level: Level) {
        use std::sync::atomic::Ordering;
        self.level.store(level_to_u8(level), Ordering::SeqCst);
        let new_drain = LevelFilterDrain::new(slog::Discard, level);
        self.ctrl.set(new_drain);
    }

    /// Get the current log level.
    pub fn level(&self) -> Level {
        use std::sync::atomic::Ordering;
        u8_to_level(self.level.load(Ordering::SeqCst))
    }
}

/// Log format for output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// JSON format for machine parsing (production).
    #[default]
    Json,
    /// Human-readable format (development).
    Pretty,
}

impl std::str::FromStr for LogFormat {
    type Err = std::convert::Infallible;

    /// Parse from string, case-insensitive. Defaults to Json for unknown values.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "pretty" | "text" | "human" => Self::Pretty,
            _ => Self::Json,
        })
    }
}

/// Parse log level from string.
pub fn parse_level(s: &str) -> Level {
    match s.to_lowercase().as_str() {
        "trace" => Level::Trace,
        "debug" => Level::Debug,
        "info" => Level::Info,
        "warn" | "warning" => Level::Warning,
        "error" => Level::Error,
        "critical" | "crit" => Level::Critical,
        _ => Level::Info,
    }
}

fn level_to_u8(level: Level) -> u8 {
    match level {
        Level::Critical => 0,
        Level::Error => 1,
        Level::Warning => 2,
        Level::Info => 3,
        Level::Debug => 4,
        Level::Trace => 5,
    }
}

fn u8_to_level(val: u8) -> Level {
    match val {
        0 => Level::Critical,
        1 => Level::Error,
        2 => Level::Warning,
        3 => Level::Info,
        4 => Level::Debug,
        _ => Level::Trace,
    }
}

/// A level filter drain that wraps any drain.
struct LevelFilterDrain<D> {
    drain: D,
    level: Level,
}

impl<D> LevelFilterDrain<D> {
    fn new(drain: D, level: Level) -> Self {
        Self { drain, level }
    }
}

impl<D: Drain<Ok = (), Err = slog::Never>> Drain for LevelFilterDrain<D> {
    type Ok = ();
    type Err = slog::Never;

    fn log(
        &self,
        record: &slog::Record,
        values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        if record.level().is_at_least(self.level) {
            self.drain.log(record, values)
        } else {
            Ok(())
        }
    }
}

/// Initialize the logging system from configuration strings.
///
/// Convenience wrapper that parses level and format from strings.
///
/// # Example
///
/// ```no_run
/// use evolve_operations::observability::logging::init_logging_from_config;
/// use slog::info;
///
/// let (logger, level_switch) = init_logging_from_config("info", "json");
/// info!(logger, "Node started"; "version" => "0.1.0");
/// ```
pub fn init_logging_from_config(level: &str, format: &str) -> (Logger, LogLevelSwitch) {
    let level = parse_level(level);
    let format: LogFormat = format.parse().unwrap_or_default();
    init_logging(level, format)
}

/// Initialize the logging system.
///
/// Returns a root logger and a handle to adjust log levels at runtime.
///
/// # Arguments
///
/// * `level` - Initial log level (e.g., "info", "debug")
/// * `format` - Output format ("json" or "pretty")
///
/// # Example
///
/// ```no_run
/// use evolve_operations::observability::logging::{init_logging, LogFormat};
/// use slog::{info, Level};
///
/// let (logger, level_switch) = init_logging(Level::Info, LogFormat::Json);
/// info!(logger, "Node started"; "version" => "0.1.0");
///
/// // Adjust level at runtime
/// level_switch.set_level(Level::Debug);
/// ```
pub fn init_logging(level: Level, format: LogFormat) -> (Logger, LogLevelSwitch) {
    use std::sync::atomic::AtomicU8;

    // Build the base drain based on format
    let base_drain: Box<dyn Drain<Ok = (), Err = slog::Never> + Send> = match format {
        LogFormat::Json => {
            let drain = slog_json::Json::new(std::io::stdout())
                .add_default_keys()
                .build()
                .fuse();
            Box::new(drain)
        }
        LogFormat::Pretty => {
            let decorator = slog_term::TermDecorator::new().build();
            let drain = slog_term::FullFormat::new(decorator).build().fuse();
            Box::new(drain)
        }
    };

    // Wrap in mutex for thread-safety (required for Box<dyn>)
    let mutex_drain = std::sync::Mutex::new(base_drain);
    let safe_drain = slog::Fuse(MutexDrain(mutex_drain));

    // Wrap with atomic switch for runtime level adjustment
    let switch = AtomicSwitch::new(safe_drain);
    let ctrl = Arc::new(switch.ctrl());

    // Make it async with bounded channel
    let async_drain = slog_async::Async::new(switch.fuse())
        .chan_size(4096)
        .overflow_strategy(OverflowStrategy::DropAndReport)
        .build()
        .fuse();

    // Create root logger
    let logger = Logger::root(async_drain, o!("service" => "evolve"));

    // Bridge standard `log` crate to slog
    let bridge_result = slog_stdlog::init_with_level(to_log_level(level));
    if let Err(e) = bridge_result {
        eprintln!("Warning: Failed to set up log bridge: {}", e);
    }

    let switch = LogLevelSwitch {
        ctrl,
        level: AtomicU8::new(level_to_u8(level)),
    };

    (logger, switch)
}

/// Wrapper to make Mutex<Box<dyn Drain>> implement Drain.
struct MutexDrain(std::sync::Mutex<Box<dyn Drain<Ok = (), Err = slog::Never> + Send>>);

impl Drain for MutexDrain {
    type Ok = ();
    type Err = slog::Never;

    fn log(
        &self,
        record: &slog::Record,
        values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        if let Ok(guard) = self.0.lock() {
            let _ = guard.log(record, values);
        }
        Ok(())
    }
}

/// Convert slog Level to log::Level for the bridge.
fn to_log_level(level: Level) -> log::Level {
    match level {
        Level::Critical | Level::Error => log::Level::Error,
        Level::Warning => log::Level::Warn,
        Level::Info => log::Level::Info,
        Level::Debug => log::Level::Debug,
        Level::Trace => log::Level::Trace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_level() {
        assert_eq!(parse_level("trace"), Level::Trace);
        assert_eq!(parse_level("DEBUG"), Level::Debug);
        assert_eq!(parse_level("Info"), Level::Info);
        assert_eq!(parse_level("WARN"), Level::Warning);
        assert_eq!(parse_level("warning"), Level::Warning);
        assert_eq!(parse_level("error"), Level::Error);
        assert_eq!(parse_level("crit"), Level::Critical);
        assert_eq!(parse_level("unknown"), Level::Info);
    }

    #[test]
    fn test_log_format_from_str() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("JSON".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("pretty".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
        assert_eq!("text".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
        assert_eq!("unknown".parse::<LogFormat>().unwrap(), LogFormat::Json);
    }

    #[test]
    fn test_level_roundtrip() {
        for level in [
            Level::Critical,
            Level::Error,
            Level::Warning,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            assert_eq!(u8_to_level(level_to_u8(level)), level);
        }
    }
}

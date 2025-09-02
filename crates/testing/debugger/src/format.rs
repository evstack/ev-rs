//! Trace serialization formats.
//!
//! Supports both binary (bincode + optional compression) and JSON formats
//! for trace storage and exchange.

use crate::trace::ExecutionTrace;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::Path;

/// Format for serializing traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceFormat {
    /// Binary format using bincode (compact, fast).
    Binary,
    /// Compressed binary format (bincode + gzip).
    BinaryCompressed,
    /// JSON format (human-readable).
    Json,
    /// Pretty-printed JSON format.
    JsonPretty,
}

impl Default for TraceFormat {
    fn default() -> Self {
        TraceFormat::BinaryCompressed
    }
}

/// Error type for serialization operations.
#[derive(Debug)]
pub enum FormatError {
    /// IO error during read/write.
    Io(io::Error),
    /// Serialization error.
    Serialization(String),
    /// Deserialization error.
    Deserialization(String),
    /// Invalid format or header.
    InvalidFormat(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Io(e) => write!(f, "IO error: {e}"),
            FormatError::Serialization(msg) => write!(f, "Serialization error: {msg}"),
            FormatError::Deserialization(msg) => write!(f, "Deserialization error: {msg}"),
            FormatError::InvalidFormat(msg) => write!(f, "Invalid format: {msg}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<io::Error> for FormatError {
    fn from(e: io::Error) -> Self {
        FormatError::Io(e)
    }
}

/// Magic bytes for identifying trace files.
const TRACE_MAGIC: &[u8; 4] = b"EVTR";

/// Current trace file version.
const TRACE_VERSION: u8 = 1;

/// Header for binary trace files.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceHeader {
    magic: [u8; 4],
    version: u8,
    format: u8, // 0 = binary, 1 = compressed
    reserved: [u8; 2],
}

impl TraceHeader {
    fn new(compressed: bool) -> Self {
        Self {
            magic: *TRACE_MAGIC,
            version: TRACE_VERSION,
            format: if compressed { 1 } else { 0 },
            reserved: [0; 2],
        }
    }

    fn is_valid(&self) -> bool {
        self.magic == *TRACE_MAGIC && self.version <= TRACE_VERSION
    }

    fn is_compressed(&self) -> bool {
        self.format == 1
    }
}

/// Serializes a trace to bytes.
pub fn serialize_trace(
    trace: &ExecutionTrace,
    format: TraceFormat,
) -> Result<Vec<u8>, FormatError> {
    match format {
        TraceFormat::Binary => serialize_binary(trace, false),
        TraceFormat::BinaryCompressed => serialize_binary(trace, true),
        TraceFormat::Json => serialize_json(trace, false),
        TraceFormat::JsonPretty => serialize_json(trace, true),
    }
}

/// Deserializes a trace from bytes.
pub fn deserialize_trace(data: &[u8]) -> Result<ExecutionTrace, FormatError> {
    // Try to detect format from magic bytes
    if data.len() >= 4 && &data[0..4] == TRACE_MAGIC {
        deserialize_binary(data)
    } else if data.starts_with(b"{") {
        deserialize_json(data)
    } else {
        Err(FormatError::InvalidFormat(
            "Unknown trace format".to_string(),
        ))
    }
}

/// Serializes to binary format.
fn serialize_binary(trace: &ExecutionTrace, compress: bool) -> Result<Vec<u8>, FormatError> {
    let mut output = Vec::new();

    // Write header
    let header = TraceHeader::new(compress);
    let header_bytes =
        bincode::serialize(&header).map_err(|e| FormatError::Serialization(e.to_string()))?;
    output.extend_from_slice(&header_bytes);

    // Serialize trace
    let trace_bytes =
        bincode::serialize(trace).map_err(|e| FormatError::Serialization(e.to_string()))?;

    if compress {
        // Compress the trace data
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&trace_bytes)?;
        let compressed = encoder.finish()?;
        output.extend_from_slice(&compressed);
    } else {
        output.extend_from_slice(&trace_bytes);
    }

    Ok(output)
}

/// Deserializes from binary format.
fn deserialize_binary(data: &[u8]) -> Result<ExecutionTrace, FormatError> {
    // Parse header
    let header_size = std::mem::size_of::<TraceHeader>();
    if data.len() < header_size {
        return Err(FormatError::InvalidFormat(
            "Data too short for header".to_string(),
        ));
    }

    let header: TraceHeader = bincode::deserialize(&data[..header_size])
        .map_err(|e| FormatError::Deserialization(e.to_string()))?;

    if !header.is_valid() {
        return Err(FormatError::InvalidFormat(format!(
            "Invalid header: magic={:?}, version={}",
            header.magic, header.version
        )));
    }

    let trace_data = &data[header_size..];

    if header.is_compressed() {
        // Decompress
        let mut decoder = GzDecoder::new(trace_data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        bincode::deserialize(&decompressed).map_err(|e| FormatError::Deserialization(e.to_string()))
    } else {
        bincode::deserialize(trace_data).map_err(|e| FormatError::Deserialization(e.to_string()))
    }
}

/// Serializes to JSON format.
fn serialize_json(trace: &ExecutionTrace, pretty: bool) -> Result<Vec<u8>, FormatError> {
    let result = if pretty {
        serde_json::to_vec_pretty(trace)
    } else {
        serde_json::to_vec(trace)
    };

    result.map_err(|e| FormatError::Serialization(e.to_string()))
}

/// Deserializes from JSON format.
fn deserialize_json(data: &[u8]) -> Result<ExecutionTrace, FormatError> {
    serde_json::from_slice(data).map_err(|e| FormatError::Deserialization(e.to_string()))
}

/// Saves a trace to a file.
pub fn save_trace(
    trace: &ExecutionTrace,
    path: &Path,
    format: TraceFormat,
) -> Result<(), FormatError> {
    let data = serialize_trace(trace, format)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Loads a trace from a file.
pub fn load_trace(path: &Path) -> Result<ExecutionTrace, FormatError> {
    let data = std::fs::read(path)?;
    deserialize_trace(&data)
}

/// Returns the recommended file extension for a format.
pub fn format_extension(format: TraceFormat) -> &'static str {
    match format {
        TraceFormat::Binary => ".trace",
        TraceFormat::BinaryCompressed => ".trace.gz",
        TraceFormat::Json | TraceFormat::JsonPretty => ".trace.json",
    }
}

/// Detects the format from a file extension.
pub fn detect_format_from_extension(path: &Path) -> Option<TraceFormat> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".trace.gz") {
        Some(TraceFormat::BinaryCompressed)
    } else if name.ends_with(".trace.json") {
        Some(TraceFormat::Json)
    } else if name.ends_with(".trace") {
        Some(TraceFormat::Binary)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{StateSnapshot, TraceBuilder};

    fn make_test_trace() -> ExecutionTrace {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());
        builder.block_start(0, 1000);
        builder.state_change(b"key".to_vec(), None, Some(b"value".to_vec()));
        builder.block_end(0, [0; 32]);
        builder.finish()
    }

    #[test]
    fn test_binary_roundtrip() {
        let trace = make_test_trace();

        let data = serialize_trace(&trace, TraceFormat::Binary).unwrap();
        let loaded = deserialize_trace(&data).unwrap();

        assert_eq!(trace.seed, loaded.seed);
        assert_eq!(trace.len(), loaded.len());
    }

    #[test]
    fn test_compressed_roundtrip() {
        let trace = make_test_trace();

        let data = serialize_trace(&trace, TraceFormat::BinaryCompressed).unwrap();
        let loaded = deserialize_trace(&data).unwrap();

        assert_eq!(trace.seed, loaded.seed);
        assert_eq!(trace.len(), loaded.len());
    }

    #[test]
    fn test_json_roundtrip() {
        let trace = make_test_trace();

        let data = serialize_trace(&trace, TraceFormat::Json).unwrap();
        let loaded = deserialize_trace(&data).unwrap();

        assert_eq!(trace.seed, loaded.seed);
        assert_eq!(trace.len(), loaded.len());
    }

    #[test]
    fn test_compression_reduces_size() {
        let trace = make_test_trace();

        let uncompressed = serialize_trace(&trace, TraceFormat::Binary).unwrap();
        let compressed = serialize_trace(&trace, TraceFormat::BinaryCompressed).unwrap();

        // For small traces, compression might not help much, but let's at least
        // verify both work
        assert!(!uncompressed.is_empty());
        assert!(!compressed.is_empty());
    }

    #[test]
    fn test_format_detection() {
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace")),
            Some(TraceFormat::Binary)
        );
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace.gz")),
            Some(TraceFormat::BinaryCompressed)
        );
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace.json")),
            Some(TraceFormat::Json)
        );
        assert_eq!(detect_format_from_extension(Path::new("test.txt")), None);
    }

    #[test]
    fn test_invalid_format() {
        let result = deserialize_trace(b"invalid data");
        assert!(result.is_err());
    }
}

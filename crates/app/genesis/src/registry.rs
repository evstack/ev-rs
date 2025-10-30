//! Message registry for genesis transaction encoding.
//!
//! This module provides the mechanism for registering message types
//! that can be used in genesis files, along with their JSON-to-borsh
//! encoding logic.

use crate::error::GenesisError;
use evolve_core::InvokeRequest;
use std::collections::HashMap;

/// Trait for encoding genesis messages from JSON to InvokeRequest.
pub trait MessageRegistry {
    /// Encode a message from JSON to an InvokeRequest.
    ///
    /// # Arguments
    ///
    /// * `msg_type` - The message type identifier (e.g., "token/initialize")
    /// * `value` - The JSON payload
    ///
    /// # Returns
    ///
    /// Returns an InvokeRequest ready to be sent, or an error if encoding fails.
    fn encode_message(
        &self,
        msg_type: &str,
        value: &serde_json::Value,
    ) -> Result<InvokeRequest, GenesisError>;

    /// List all registered message types.
    fn list_message_types(&self) -> Vec<MessageTypeInfo>;
}

/// Information about a registered message type.
#[derive(Debug, Clone)]
pub struct MessageTypeInfo {
    /// The message type identifier (e.g., "token/initialize")
    pub type_name: String,
    /// Human-readable description
    pub description: String,
    /// JSON schema for the message (optional)
    pub schema: Option<serde_json::Value>,
}

/// A message encoder function.
pub type MessageEncoder =
    Box<dyn Fn(&serde_json::Value) -> Result<InvokeRequest, GenesisError> + Send + Sync>;

/// A simple in-memory message registry.
pub struct SimpleRegistry {
    encoders: HashMap<String, (MessageEncoder, MessageTypeInfo)>,
}

impl SimpleRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            encoders: HashMap::new(),
        }
    }

    /// Register a message type with its encoder.
    pub fn register<F>(&mut self, type_name: &str, description: &str, encoder: F)
    where
        F: Fn(&serde_json::Value) -> Result<InvokeRequest, GenesisError> + Send + Sync + 'static,
    {
        let info = MessageTypeInfo {
            type_name: type_name.to_string(),
            description: description.to_string(),
            schema: None,
        };
        self.encoders
            .insert(type_name.to_string(), (Box::new(encoder), info));
    }

    /// Register a message type with schema information.
    pub fn register_with_schema<F>(
        &mut self,
        type_name: &str,
        description: &str,
        schema: serde_json::Value,
        encoder: F,
    ) where
        F: Fn(&serde_json::Value) -> Result<InvokeRequest, GenesisError> + Send + Sync + 'static,
    {
        let info = MessageTypeInfo {
            type_name: type_name.to_string(),
            description: description.to_string(),
            schema: Some(schema),
        };
        self.encoders
            .insert(type_name.to_string(), (Box::new(encoder), info));
    }
}

impl Default for SimpleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageRegistry for SimpleRegistry {
    fn encode_message(
        &self,
        msg_type: &str,
        value: &serde_json::Value,
    ) -> Result<InvokeRequest, GenesisError> {
        let (encoder, _) = self
            .encoders
            .get(msg_type)
            .ok_or_else(|| GenesisError::UnknownMessageType(msg_type.to_string()))?;
        encoder(value)
    }

    fn list_message_types(&self) -> Vec<MessageTypeInfo> {
        self.encoders
            .values()
            .map(|(_, info)| info.clone())
            .collect()
    }
}

/// Helper macro to register a borsh-serializable message type.
///
/// Usage:
/// ```ignore
/// register_message!(registry, "token/initialize", "Create a new token", TokenInitialize);
/// ```
#[macro_export]
macro_rules! register_message {
    ($registry:expr, $type_name:expr, $description:expr, $msg_type:ty) => {
        $registry.register($type_name, $description, |value| {
            let msg: $msg_type = serde_json::from_value(value.clone())
                .map_err(|e| $crate::GenesisError::EncodeError(e.to_string()))?;
            evolve_core::InvokeRequest::new(&msg)
                .map_err(|e| $crate::GenesisError::EncodeError(format!("{:?}", e)))
        });
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_registry() {
        let mut registry = SimpleRegistry::new();

        registry.register("test/message", "A test message", |_value| {
            Err(GenesisError::EncodeError("not implemented".to_string()))
        });

        let types = registry.list_message_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].type_name, "test/message");
    }

    #[test]
    fn test_unknown_message_type() {
        let registry = SimpleRegistry::new();
        let result = registry.encode_message("unknown", &serde_json::json!({}));
        assert!(matches!(result, Err(GenesisError::UnknownMessageType(_))));
    }
}

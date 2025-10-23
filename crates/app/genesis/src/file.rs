//! Genesis file parsing and reference resolution.

use crate::error::GenesisError;
use crate::registry::MessageRegistry;
use crate::types::GenesisTx;
use crate::SYSTEM_ACCOUNT_ID;
use evolve_core::AccountId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A genesis file in JSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisFile {
    /// Chain identifier
    pub chain_id: String,

    /// Optional genesis time as unix timestamp (milliseconds)
    #[serde(default)]
    pub genesis_time: u64,

    /// Ordered list of genesis transactions
    pub transactions: Vec<GenesisTxJson>,
}

/// A genesis transaction in JSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisTxJson {
    /// Optional identifier for this transaction.
    /// The created account can be referenced in later transactions as `$id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Sender specification.
    /// - "system": Use the system account (AccountId::new(0))
    /// - "$ref": Reference a previously created account
    /// - number: Use a specific account ID
    #[serde(default = "default_sender")]
    pub sender: SenderSpec,

    /// Recipient specification.
    /// - "runtime": Use the runtime account for account creation
    /// - "$ref": Reference a previously created account
    /// - number: Use a specific account ID
    pub recipient: RecipientSpec,

    /// Message type identifier (e.g., "token/initialize", "eoa/initialize")
    pub msg_type: String,

    /// Message payload as JSON
    pub msg: serde_json::Value,
}

fn default_sender() -> SenderSpec {
    SenderSpec::System
}

/// Specification for the sender of a genesis transaction.
#[derive(Debug, Clone, Serialize)]
pub enum SenderSpec {
    /// Use the system account
    System,
    /// Reference a previously created account (starts with $)
    Reference(String),
    /// Use a specific account ID
    AccountId(u128),
}

impl Default for SenderSpec {
    fn default() -> Self {
        Self::System
    }
}

impl<'de> serde::Deserialize<'de> for SenderSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) if s == "system" => Ok(SenderSpec::System),
            serde_json::Value::String(s) => Ok(SenderSpec::Reference(s)),
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(|v| SenderSpec::AccountId(v as u128))
                .or_else(|| n.as_i64().map(|v| SenderSpec::AccountId(v as u128)))
                .ok_or_else(|| D::Error::custom("invalid account ID number")),
            _ => Err(D::Error::custom("expected string or number for sender")),
        }
    }
}

/// Specification for the recipient of a genesis transaction.
#[derive(Debug, Clone, Serialize)]
pub enum RecipientSpec {
    /// Use the runtime account for account creation
    Runtime,
    /// Reference a previously created account (starts with $)
    Reference(String),
    /// Use a specific account ID
    AccountId(u128),
}

impl<'de> serde::Deserialize<'de> for RecipientSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) if s == "runtime" => Ok(RecipientSpec::Runtime),
            serde_json::Value::String(s) => Ok(RecipientSpec::Reference(s)),
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(|v| RecipientSpec::AccountId(v as u128))
                .or_else(|| n.as_i64().map(|v| RecipientSpec::AccountId(v as u128)))
                .ok_or_else(|| D::Error::custom("invalid account ID number")),
            _ => Err(D::Error::custom("expected string or number for recipient")),
        }
    }
}

impl GenesisFile {
    /// Load a genesis file from a path.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GenesisError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse a genesis file from a JSON string.
    pub fn parse(json: &str) -> Result<Self, GenesisError> {
        serde_json::from_str(json).map_err(GenesisError::JsonError)
    }

    /// Validate the genesis file structure.
    pub fn validate(&self) -> Result<(), GenesisError> {
        let mut seen_ids: HashMap<String, usize> = HashMap::new();

        for (idx, tx) in self.transactions.iter().enumerate() {
            // Check for duplicate IDs
            if let Some(id) = &tx.id {
                if let Some(prev_idx) = seen_ids.get(id) {
                    return Err(GenesisError::DuplicateId(format!(
                        "'{}' at index {} (first seen at {})",
                        id, idx, prev_idx
                    )));
                }
                seen_ids.insert(id.clone(), idx);
            }

            // Validate references point to earlier transactions
            if let SenderSpec::Reference(ref r) = tx.sender {
                if r.starts_with('$') {
                    let ref_id = &r[1..];
                    if !seen_ids.contains_key(ref_id) {
                        return Err(GenesisError::InvalidReference(ref_id.to_string(), idx));
                    }
                }
            }

            if let RecipientSpec::Reference(ref r) = tx.recipient {
                if r.starts_with('$') {
                    let ref_id = &r[1..];
                    if !seen_ids.contains_key(ref_id) {
                        return Err(GenesisError::InvalidReference(ref_id.to_string(), idx));
                    }
                }
            }
        }

        Ok(())
    }

    /// Convert to a list of GenesisTx, resolving references.
    ///
    /// This requires a callback that returns the AccountId created by each transaction.
    /// The callback is called in order, and its results are used to resolve $references.
    pub fn to_transactions(
        &self,
        registry: &dyn MessageRegistry,
    ) -> Result<Vec<GenesisTx>, GenesisError> {
        self.validate()?;

        let mut txs = Vec::with_capacity(self.transactions.len());
        let mut id_to_account: HashMap<String, AccountId> = HashMap::new();
        let mut next_account_id: u128 = 1; // Start from 1, 0 is system

        for (idx, tx_json) in self.transactions.iter().enumerate() {
            // Resolve sender
            let sender = match &tx_json.sender {
                SenderSpec::System => SYSTEM_ACCOUNT_ID,
                SenderSpec::Reference(r) if r.starts_with('$') => {
                    let ref_id = &r[1..];
                    *id_to_account
                        .get(ref_id)
                        .ok_or_else(|| GenesisError::InvalidReference(ref_id.to_string(), idx))?
                }
                SenderSpec::Reference(r) => {
                    // Treat as account ID string
                    AccountId::new(r.parse().map_err(|_| {
                        GenesisError::ParseError(format!("invalid sender: {}", r))
                    })?)
                }
                SenderSpec::AccountId(id) => AccountId::new(*id),
            };

            // Resolve recipient
            let recipient = match &tx_json.recipient {
                RecipientSpec::Runtime => evolve_core::runtime_api::RUNTIME_ACCOUNT_ID,
                RecipientSpec::Reference(r) if r.starts_with('$') => {
                    let ref_id = &r[1..];
                    *id_to_account
                        .get(ref_id)
                        .ok_or_else(|| GenesisError::InvalidReference(ref_id.to_string(), idx))?
                }
                RecipientSpec::Reference(r) => {
                    // Treat as account ID string
                    AccountId::new(r.parse().map_err(|_| {
                        GenesisError::ParseError(format!("invalid recipient: {}", r))
                    })?)
                }
                RecipientSpec::AccountId(id) => AccountId::new(*id),
            };

            // Resolve $references in the message payload
            let resolved_msg = resolve_references_in_value(&tx_json.msg, &id_to_account)?;

            // Encode the message using the registry
            let request = registry.encode_message(&tx_json.msg_type, &resolved_msg)?;

            let genesis_tx = if let Some(id) = &tx_json.id {
                GenesisTx::with_id(id.clone(), sender, recipient, request)
            } else {
                GenesisTx::new(sender, recipient, request)
            };

            // If this transaction has an ID, assign it the next account ID
            // This assumes the transaction creates an account
            if let Some(id) = &tx_json.id {
                let account_id = AccountId::new(next_account_id);
                id_to_account.insert(id.clone(), account_id);
                next_account_id += 1;
            }

            txs.push(genesis_tx);
        }

        Ok(txs)
    }
}

/// Recursively resolve $references in a JSON value.
fn resolve_references_in_value(
    value: &serde_json::Value,
    id_to_account: &HashMap<String, AccountId>,
) -> Result<serde_json::Value, GenesisError> {
    match value {
        serde_json::Value::String(s) if s.starts_with('$') => {
            let ref_id = &s[1..];
            let account_id = id_to_account
                .get(ref_id)
                .ok_or_else(|| GenesisError::InvalidReference(ref_id.to_string(), 0))?;
            // Use string representation for u128 since JSON numbers are limited to i64/u64
            Ok(serde_json::Value::String(account_id.inner().to_string()))
        }
        serde_json::Value::Array(arr) => {
            let resolved: Result<Vec<_>, _> = arr
                .iter()
                .map(|v| resolve_references_in_value(v, id_to_account))
                .collect();
            Ok(serde_json::Value::Array(resolved?))
        }
        serde_json::Value::Object(obj) => {
            let resolved: Result<serde_json::Map<String, serde_json::Value>, _> = obj
                .iter()
                .map(|(k, v)| {
                    resolve_references_in_value(v, id_to_account).map(|rv| (k.clone(), rv))
                })
                .collect();
            Ok(serde_json::Value::Object(resolved?))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_genesis_file() {
        let json = r#"{
            "chain_id": "test-chain",
            "transactions": [
                {
                    "id": "alice",
                    "recipient": "runtime",
                    "msg_type": "eoa/initialize",
                    "msg": {}
                }
            ]
        }"#;

        let genesis = GenesisFile::parse(json).unwrap();
        assert_eq!(genesis.chain_id, "test-chain");
        assert_eq!(genesis.transactions.len(), 1);
        assert_eq!(genesis.transactions[0].id, Some("alice".to_string()));
    }

    #[test]
    fn test_validate_duplicate_id() {
        let json = r#"{
            "chain_id": "test-chain",
            "transactions": [
                { "id": "alice", "recipient": "runtime", "msg_type": "eoa/initialize", "msg": {} },
                { "id": "alice", "recipient": "runtime", "msg_type": "eoa/initialize", "msg": {} }
            ]
        }"#;

        let genesis = GenesisFile::parse(json).unwrap();
        assert!(genesis.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_reference() {
        let json = r#"{
            "chain_id": "test-chain",
            "transactions": [
                { "recipient": "$nonexistent", "msg_type": "test", "msg": {} }
            ]
        }"#;

        let genesis = GenesisFile::parse(json).unwrap();
        assert!(genesis.validate().is_err());
    }

    #[test]
    fn test_resolve_references_in_value() {
        let mut id_to_account = HashMap::new();
        id_to_account.insert("alice".to_string(), AccountId::new(1));

        let value = serde_json::json!({
            "account": "$alice",
            "nested": {
                "ref": "$alice"
            },
            "list": ["$alice", "literal"]
        });

        let resolved = resolve_references_in_value(&value, &id_to_account).unwrap();

        // Account IDs are resolved as strings (for u128 compatibility)
        assert_eq!(resolved["account"], "1");
        assert_eq!(resolved["nested"]["ref"], "1");
        assert_eq!(resolved["list"][0], "1");
        assert_eq!(resolved["list"][1], "literal");
    }

    #[test]
    fn test_to_transactions_with_references() {
        use crate::registry::SimpleRegistry;
        use evolve_core::InvokeRequest;

        // Create a mock registry that just creates empty requests
        let mut registry = SimpleRegistry::new();
        registry.register("test/init", "Test initialization", |_value| {
            // Create a minimal valid InvokeRequest
            #[derive(Clone, borsh::BorshSerialize)]
            struct EmptyMsg;
            impl evolve_core::InvokableMessage for EmptyMsg {
                const FUNCTION_IDENTIFIER: u64 = 0;
                const FUNCTION_IDENTIFIER_NAME: &'static str = "init";
            }
            InvokeRequest::new(&EmptyMsg).map_err(|e| GenesisError::EncodeError(format!("{:?}", e)))
        });

        let json = r#"{
            "chain_id": "test-chain",
            "transactions": [
                {
                    "id": "alice",
                    "recipient": "runtime",
                    "msg_type": "test/init",
                    "msg": {}
                },
                {
                    "id": "token",
                    "recipient": "runtime",
                    "msg_type": "test/init",
                    "msg": { "owner": "$alice" }
                }
            ]
        }"#;

        let genesis = GenesisFile::parse(json).unwrap();
        let txs = genesis.to_transactions(&registry).unwrap();

        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].id, Some("alice".to_string()));
        assert_eq!(txs[1].id, Some("token".to_string()));
        // Both should target the runtime account
        assert_eq!(txs[0].recipient, evolve_core::runtime_api::RUNTIME_ACCOUNT_ID);
        assert_eq!(txs[1].recipient, evolve_core::runtime_api::RUNTIME_ACCOUNT_ID);
    }

    #[test]
    fn test_to_transactions_unknown_message_type() {
        use crate::registry::SimpleRegistry;

        let registry = SimpleRegistry::new(); // Empty registry

        let json = r#"{
            "chain_id": "test-chain",
            "transactions": [
                {
                    "recipient": "runtime",
                    "msg_type": "unknown/type",
                    "msg": {}
                }
            ]
        }"#;

        let genesis = GenesisFile::parse(json).unwrap();
        let result = genesis.to_transactions(&registry);

        assert!(matches!(result, Err(GenesisError::UnknownMessageType(_))));
    }

    #[test]
    fn test_sender_spec_parsing() {
        // Test system sender
        let json = r#"{
            "chain_id": "test",
            "transactions": [
                { "sender": "system", "recipient": "1", "msg_type": "test", "msg": {} }
            ]
        }"#;
        let genesis = GenesisFile::parse(json).unwrap();
        assert!(matches!(
            genesis.transactions[0].sender,
            SenderSpec::System
        ));

        // Test numeric sender
        let json = r#"{
            "chain_id": "test",
            "transactions": [
                { "sender": 123, "recipient": "1", "msg_type": "test", "msg": {} }
            ]
        }"#;
        let genesis = GenesisFile::parse(json).unwrap();
        assert!(matches!(
            genesis.transactions[0].sender,
            SenderSpec::AccountId(123)
        ));
    }

    #[test]
    fn test_recipient_spec_parsing() {
        // Test runtime recipient
        let json = r#"{
            "chain_id": "test",
            "transactions": [
                { "recipient": "runtime", "msg_type": "test", "msg": {} }
            ]
        }"#;
        let genesis = GenesisFile::parse(json).unwrap();
        assert!(matches!(
            genesis.transactions[0].recipient,
            RecipientSpec::Runtime
        ));

        // Test numeric recipient
        let json = r#"{
            "chain_id": "test",
            "transactions": [
                { "recipient": 456, "msg_type": "test", "msg": {} }
            ]
        }"#;
        let genesis = GenesisFile::parse(json).unwrap();
        assert!(matches!(
            genesis.transactions[0].recipient,
            RecipientSpec::AccountId(456)
        ));
    }
}

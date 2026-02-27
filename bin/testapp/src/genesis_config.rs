use alloy_primitives::Address;
use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::AccountId;
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_node::HasTokenAccountId;
use serde::Deserialize;

/// Evd genesis configuration loaded from JSON.
#[derive(Deserialize)]
pub struct EvdGenesisConfig {
    pub token: TokenConfig,
    pub minter_id: u128,
    pub accounts: Vec<AccountConfig>,
}

#[derive(Deserialize)]
pub struct TokenConfig {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub icon_url: String,
    pub description: String,
}

#[derive(Deserialize)]
pub struct AccountConfig {
    pub eth_address: String,
    pub balance: u128,
}

/// Persisted genesis result (replaces testapp's GenesisAccounts for evd).
#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize)]
pub struct EvdGenesisResult {
    pub token: AccountId,
    pub scheduler: AccountId,
}

impl HasTokenAccountId for EvdGenesisResult {
    fn token_account_id(&self) -> AccountId {
        self.token
    }
}

impl EvdGenesisConfig {
    /// Load genesis config from a JSON file.
    pub fn load(path: &str) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read genesis file '{}': {}", path, e))?;
        let config: Self =
            serde_json::from_str(&data).map_err(|e| format!("invalid genesis JSON: {}", e))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.accounts.is_empty() {
            return Err("genesis config must have at least one account".into());
        }
        for (i, acc) in self.accounts.iter().enumerate() {
            acc.parse_address()
                .map_err(|e| format!("account[{}]: {}", i, e))?;
        }
        Ok(())
    }
}

impl TokenConfig {
    pub fn to_metadata(&self) -> FungibleAssetMetadata {
        FungibleAssetMetadata {
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            decimals: self.decimals,
            icon_url: self.icon_url.clone(),
            description: self.description.clone(),
        }
    }
}

impl AccountConfig {
    pub fn parse_address(&self) -> Result<Address, String> {
        self.eth_address
            .parse::<Address>()
            .map_err(|e| format!("invalid eth address '{}': {}", self.eth_address, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_genesis_file(contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!("evd-genesis-{}-{id}.json", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn load_fails_for_missing_file() {
        let path = "/definitely/not/present/evd-genesis.json";
        let err = EvdGenesisConfig::load(path)
            .err()
            .expect("missing file must fail");
        assert!(err.contains("failed to read genesis file"));
    }

    #[test]
    fn load_fails_for_empty_accounts() {
        let path = temp_genesis_file(
            r#"{
                "token": {
                    "name": "Evolve",
                    "symbol": "EV",
                    "decimals": 6,
                    "icon_url": "https://example.com/icon.png",
                    "description": "token"
                },
                "minter_id": 100,
                "accounts": []
            }"#,
        );

        let err = EvdGenesisConfig::load(path.to_str().unwrap())
            .err()
            .expect("empty accounts fail");
        assert!(err.contains("at least one account"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_fails_for_invalid_account_address() {
        let path = temp_genesis_file(
            r#"{
                "token": {
                    "name": "Evolve",
                    "symbol": "EV",
                    "decimals": 6,
                    "icon_url": "https://example.com/icon.png",
                    "description": "token"
                },
                "minter_id": 100,
                "accounts": [
                    { "eth_address": "not-an-address", "balance": 1 }
                ]
            }"#,
        );

        let err = EvdGenesisConfig::load(path.to_str().unwrap())
            .err()
            .expect("invalid address must fail");
        assert!(err.contains("account[0]"));
        assert!(err.contains("invalid eth address"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_succeeds_for_valid_config() {
        let path = temp_genesis_file(
            r#"{
                "token": {
                    "name": "Evolve",
                    "symbol": "EV",
                    "decimals": 6,
                    "icon_url": "https://example.com/icon.png",
                    "description": "token"
                },
                "minter_id": 100,
                "accounts": [
                    {
                        "eth_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                        "balance": 1000
                    }
                ]
            }"#,
        );

        let cfg = EvdGenesisConfig::load(path.to_str().unwrap()).expect("valid config should load");
        assert_eq!(cfg.minter_id, 100);
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.accounts[0].balance, 1000);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parse_address_rejects_invalid_and_accepts_valid() {
        let bad = AccountConfig {
            eth_address: "bad-address".to_string(),
            balance: 1,
        };
        assert!(bad.parse_address().is_err());

        let good = AccountConfig {
            eth_address: "0x0000000000000000000000000000000000000001".to_string(),
            balance: 1,
        };
        assert!(good.parse_address().is_ok());
    }

    #[test]
    fn token_to_metadata_maps_all_fields() {
        let token = TokenConfig {
            name: "Evolve".to_string(),
            symbol: "EV".to_string(),
            decimals: 6,
            icon_url: "https://example.com/token.png".to_string(),
            description: "Sample token".to_string(),
        };
        let metadata = token.to_metadata();

        assert_eq!(metadata.name, "Evolve");
        assert_eq!(metadata.symbol, "EV");
        assert_eq!(metadata.decimals, 6);
        assert_eq!(metadata.icon_url, "https://example.com/token.png");
        assert_eq!(metadata.description, "Sample token");
    }
}

use alloy_primitives::Address;
use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::AccountId;
use evolve_fungible_asset::FungibleAssetMetadata;
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

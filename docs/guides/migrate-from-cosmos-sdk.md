# Migrating from a Cosmos-SDK chain

This guide provides instructions for migrating state from a Cosmos-SDK based blockchain to Evolve. The migration process involves transforming the state format from Protobuf (used by Cosmos-SDK) to Borsh (used by Evolve) while maintaining data integrity and compatibility.

## Important Notes

⚠️ **State Format Differences**: The migration is not byte-compatible due to different serialization formats:

- **Cosmos-SDK**: Uses Protobuf for state serialization
- **Evolve**: Uses Borsh for state serialization

⚠️ **State Layout**: Evolve uses a different state key layout compared to Cosmos-SDK, which may require state key transformations during migration.

⚠️ **Account Model**: Evolve uses a different account model where each token has its own account instance and address.

## Supported Modules

| Module       | Status       | Notes                  |
| ------------ | ------------ | ---------------------- |
| Bank         | ✅ Supported | Full migration support |
| BeginBlock   | ✅ Supported | -                      |
| EndBlock     | ✅ Supported | -                      |
| Staking      | 🔄 Planned   | -                      |
| Distribution | 🔄 Planned   | -                      |
| Gov          | 🔄 Planned   | -                      |
| IBC          | 🔄 Planned   | -                      |
| Auth         | 🔄 Planned   | –                      |

## Migration: Bank Module

The Bank module manages token balances and transfers. The migration process transforms Cosmos-SDK's account-centric balance model to Evolve's token-centric account model.

### Genesis State Structure

Based on the Cosmos-SDK bank genesis proto, the state contains:

```protobuf
message GenesisState {
  Params params = 1;
  repeated Balance balances = 2;
  repeated cosmos.base.v1beta1.Coin supply = 3;
  repeated Metadata denom_metadata = 4;
  repeated SendEnabled send_enabled = 5;
}

message Balance {
  string address = 1;
  repeated cosmos.base.v1beta1.Coin coins = 2;
}
```

### Key Differences

#### Cosmos-SDK Model

- **Account-centric**: Each account holds multiple coin balances
- **Single address**: One address per account across all denominations
- **Nested structure**: `account_address -> [denom1: amount1, denom2: amount2, ...]`

#### Evolve Model

- **Token-centric**: Each token denomination is a separate account/contract
- **Token initialization**: Tokens are created with metadata and initial balance distributions
- **Account balances**: Regular accounts hold balances of different token contracts

### Migration Process

1. **Extract Genesis Data**: Parse the Cosmos-SDK genesis file
2. **Transform Balances**: Group balances by denomination and collect initial holders
3. **Migrate Metadata**: Transform denomination metadata to FungibleAssetMetadata
4. **Create Tokens**: Initialize each token with metadata and initial balances
5. **Setup Services**: Create required system accounts (NameService, etc.)

### Example Rust Migration Script

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use evolve_core::{AccountId, Environment, SdkResult};
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_token::account::TokenRef;
use evolve_ns::account::NameServiceRef;

// Cosmos-SDK structures (simplified)
#[derive(Deserialize)]
struct CosmosGenesis {
    app_state: AppState,
}

#[derive(Deserialize)]
struct AppState {
    bank: BankGenesis,
}

#[derive(Deserialize)]
struct BankGenesis {
    params: BankParams,
    balances: Vec<Balance>,
    supply: Vec<Coin>,
    denom_metadata: Vec<Metadata>,
    send_enabled: Vec<SendEnabled>,
}

#[derive(Deserialize)]
struct Balance {
    address: String,
    coins: Vec<Coin>,
}

#[derive(Deserialize)]
struct Coin {
    denom: String,
    amount: String,
}

#[derive(Deserialize)]
struct Metadata {
    description: String,
    denom_units: Vec<DenomUnit>,
    base: String,
    display: String,
    name: String,
    symbol: String,
}

#[derive(Deserialize)]
struct DenomUnit {
    denom: String,
    exponent: u32,
    aliases: Vec<String>,
}

#[derive(Deserialize)]
struct SendEnabled {
    denom: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct BankParams {
    send_enabled: Vec<SendEnabled>,
    default_send_enabled: bool,
}

// Migration structures
struct TokenInitData {
    metadata: FungibleAssetMetadata,
    initial_balances: Vec<(AccountId, u128)>,
    minter: Option<AccountId>,
}

struct AddressMapping {
    cosmos_to_evolved: HashMap<String, AccountId>,
    next_account_id: u64,
}

impl AddressMapping {
    fn new(start_id: u64) -> Self {
        Self {
            cosmos_to_evolved: HashMap::new(),
            next_account_id: start_id,
        }
    }

    fn get_or_create_account_id(&mut self, cosmos_address: &str) -> AccountId {
        if let Some(&account_id) = self.cosmos_to_evolved.get(cosmos_address) {
            account_id
        } else {
            let account_id = AccountId::new(self.next_account_id);
            self.cosmos_to_evolved.insert(cosmos_address.to_string(), account_id);
            self.next_account_id += 1;
            account_id
        }
    }
}

fn parse_cosmos_genesis(genesis_json: &str) -> Result<BankGenesis, Box<dyn std::error::Error>> {
    let genesis: CosmosGenesis = serde_json::from_str(genesis_json)?;
    Ok(genesis.app_state.bank)
}

fn prepare_token_init_data(
    bank_state: &BankGenesis,
    address_mapping: &mut AddressMapping
) -> Result<HashMap<String, TokenInitData>, Box<dyn std::error::Error>> {
    let mut token_data: HashMap<String, TokenInitData> = HashMap::new();

    // First, collect metadata for each denomination
    for metadata in &bank_state.denom_metadata {
        let decimals = metadata.denom_units
            .iter()
            .find(|unit| unit.denom == metadata.display)
            .map(|unit| unit.exponent as u8)
            .unwrap_or(6); // Default to 6 decimals

        let asset_metadata = FungibleAssetMetadata {
            name: metadata.base.clone(), // Use base denom as name
            symbol: metadata.symbol.clone(),
            decimals,
            icon_url: "".to_string(), // Cosmos metadata doesn't have icon_url
            description: metadata.description.clone(),
        };

        token_data.insert(metadata.base.clone(), TokenInitData {
            metadata: asset_metadata,
            initial_balances: Vec::new(),
            minter: None, // Will be set separately if needed
        });
    }

    // Handle tokens without metadata (create default metadata)
    for balance in &bank_state.balances {
        for coin in &balance.coins {
            if !token_data.contains_key(&coin.denom) {
                let asset_metadata = FungibleAssetMetadata {
                    name: coin.denom.clone(),
                    symbol: coin.denom.to_uppercase(),
                    decimals: 6, // Default decimals
                    icon_url: "".to_string(),
                    description: format!("Migrated token: {}", coin.denom),
                };

                token_data.insert(coin.denom.clone(), TokenInitData {
                    metadata: asset_metadata,
                    initial_balances: Vec::new(),
                    minter: None,
                });
            }
        }
    }

    // Now collect initial balances for each token
    for balance in &bank_state.balances {
        let account_id = address_mapping.get_or_create_account_id(&balance.address);

        for coin in &balance.coins {
            let amount = coin.amount.parse::<u128>()?;
            if let Some(token_init) = token_data.get_mut(&coin.denom) {
                token_init.initial_balances.push((account_id, amount));
            }
        }
    }

    Ok(token_data)
}

fn perform_genesis_migration(
    bank_state: BankGenesis,
    env: &mut dyn Environment,
) -> SdkResult<(NameServiceRef, HashMap<String, AccountId>)> {
    let mut address_mapping = AddressMapping::new(100_000); // Start account IDs from 100,000
    let mut created_tokens = HashMap::new();

    // Create name service first (required for other accounts)
    let ns_acc = NameServiceRef::initialize(vec![], env)?.0;

    // Prepare token initialization data
    let token_init_data = prepare_token_init_data(&bank_state, &mut address_mapping)
        .map_err(|_| evolve_core::ERR_ENCODING)?;

    // Create each token
    for (denom, init_data) in token_init_data {
        // Skip tokens with no initial balances
        if init_data.initial_balances.is_empty() {
            continue;
        }

        let token_ref = TokenRef::initialize(
            init_data.metadata,
            init_data.initial_balances,
            init_data.minter,
            env,
        )?;

        created_tokens.insert(denom.clone(), token_ref.0.id());

        // Register token in name service
        ns_acc.updates_names(
            vec![(denom, token_ref.0.id())],
            env,
        )?;
    }

    Ok((ns_acc, created_tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosmos_genesis_parsing() {
        let cosmos_genesis = r#"
        {
            "app_state": {
                "bank": {
                    "params": {
                        "send_enabled": [],
                        "default_send_enabled": true
                    },
                    "balances": [
                        {
                            "address": "cosmos1abc123",
                            "coins": [
                                {"denom": "uatom", "amount": "1000000"},
                                {"denom": "ustake", "amount": "500000"}
                            ]
                        },
                        {
                            "address": "cosmos1def456",
                            "coins": [
                                {"denom": "uatom", "amount": "2000000"}
                            ]
                        }
                    ],
                    "supply": [
                        {"denom": "uatom", "amount": "3000000"},
                        {"denom": "ustake", "amount": "500000"}
                    ],
                    "denom_metadata": [
                        {
                            "description": "The native staking token of Cosmos Hub",
                            "denom_units": [
                                {"denom": "uatom", "exponent": 0, "aliases": []},
                                {"denom": "atom", "exponent": 6, "aliases": []}
                            ],
                            "base": "uatom",
                            "display": "atom",
                            "name": "Cosmos Hub",
                            "symbol": "ATOM"
                        }
                    ],
                    "send_enabled": []
                }
            }
        }
        "#;

        let bank_state = parse_cosmos_genesis(cosmos_genesis).unwrap();
        let mut address_mapping = AddressMapping::new(100_000);
        let token_data = prepare_token_init_data(&bank_state, &mut address_mapping).unwrap();

        // Should have two tokens: uatom and ustake
        assert_eq!(token_data.len(), 2);

        // Check uatom token
        let uatom_data = &token_data["uatom"];
        assert_eq!(uatom_data.metadata.symbol, "ATOM");
        assert_eq!(uatom_data.metadata.decimals, 6);
        assert_eq!(uatom_data.initial_balances.len(), 2); // Two holders

        // Check ustake token (no metadata, should use defaults)
        let ustake_data = &token_data["ustake"];
        assert_eq!(ustake_data.metadata.symbol, "USTAKE");
        assert_eq!(ustake_data.metadata.decimals, 6);
        assert_eq!(ustake_data.initial_balances.len(), 1); // One holder

        // Verify total amounts
        let uatom_total: u128 = uatom_data.initial_balances.iter().map(|(_, amount)| amount).sum();
        assert_eq!(uatom_total, 3_000_000);

        let ustake_total: u128 = ustake_data.initial_balances.iter().map(|(_, amount)| amount).sum();
        assert_eq!(ustake_total, 500_000);
    }
}

// Example usage in a real migration
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Cosmos-SDK to Evolve Bank Migration Tool");

    // This would be part of your actual genesis setup
    // You'd integrate this into your STF's genesis function

    let cosmos_genesis = std::fs::read_to_string("cosmos_genesis.json")?;
    let bank_state = parse_cosmos_genesis(&cosmos_genesis)?;

    println!("Parsed Cosmos genesis:");
    println!("- {} token balances found", bank_state.balances.len());
    println!("- {} denomination metadata found", bank_state.denom_metadata.len());

    // In a real scenario, you'd call perform_genesis_migration inside your STF genesis
    // let (name_service, created_tokens) = perform_genesis_migration(bank_state, env)?;

    Ok(())
}
```

### Migration Steps

1. **Integrate into STF Genesis**:
   Add the migration logic to your STF's genesis function:

```rust
   pub fn do_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
       stf: &CustomStf,
       codes: &'a A,
       storage: &'a S,
       cosmos_genesis_json: &str,
   ) -> SdkResult<ExecutionState<'a, S>> {
       let genesis_height = 0;
       let genesis_time_unix_ms = 0;

       let (_, state) = stf.sudo(storage, codes, genesis_height, |env| {
           // Parse Cosmos genesis
           let bank_state = parse_cosmos_genesis(cosmos_genesis_json)
               .map_err(|_| evolve_core::ERR_ENCODING)?;

           // Perform migration
           let (ns_acc, created_tokens) = perform_genesis_migration(bank_state, env)?;

           // Create other system accounts (scheduler, gas service, etc.)
           // ... (same as original genesis)

           Ok(())
       })?;

       Ok(state)
   }
```

2. **Add Dependencies**:

   ```toml
   [dependencies]
   serde = { version = "1.0", features = ["derive"] }
   serde_json = "1.0"
   evolve-core = { path = "../../sdk/core" }
   evolve-token = { path = "../../sdk/x/token" }
   evolve-ns = { path = "../../sdk/x/ns" }
   evolve-fungible-asset = { path = "../../sdk/standards/evolve_fungible_asset" }
   ```

3. **Run Genesis**:
   ```bash
   # Your Cosmos genesis.json should be available
   cargo run --bin your-app -- --cosmos-genesis genesis.json
   ```

### Validation

After migration, validate the state by:

1. **Token Creation**: Verify each denomination created a corresponding token account
2. **Balance Distribution**: Ensure all original account balances are correctly distributed
3. **Metadata Integrity**: Confirm token metadata (name, symbol, decimals) is preserved
4. **Total Supply**: Check that the sum of all token holder balances matches original supply
5. **Name Registration**: Verify tokens are properly registered in the name service

### Considerations

- **Address Mapping**: Cosmos addresses need to be mapped to Evolve AccountIds consistently
- **Token Accounts**: Each Cosmos denomination becomes a separate token account in Evolve
- **Precision**: Ensure no precision loss during amount conversions (u128 supports large numbers)
- **Missing Metadata**: Tokens without metadata in Cosmos need default values in Evolve
- **Minter Rights**: Consider if any tokens need minting capabilities post-migration
- **System Integration**: Ensure migrated tokens work with other Evolve modules (gas, scheduler, etc.)

This migration approach transforms Cosmos-SDK's account-centric bank model to Evolve's token-centric model, where each token is its own account/contract with proper metadata and initial balance distributions.

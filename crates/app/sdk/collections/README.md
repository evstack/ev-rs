# Evolve Collections

The `evolve_collections` crate provides persistent collection types for the Evolve blockchain framework. These
collections allow smart contracts to store and retrieve data in a structured way.

## Available Collections

The crate currently provides two main collection types:

- **Map**: A key-value store that allows storing and retrieving values by key.
- **Item**: A single-value store that allows storing and retrieving a single value.

## Usage

### Map

The `Map` collection provides a key-value store with the following operations:

- `new`: Create a new map with a specific prefix
- `set`: Store a value for a key
- `get`: Retrieve a value for a key
- `update`: Update a value for a key using a callback function

#### Example

```rust
use evolve_collections::map::Map;
use evolve_core::{Environment, SdkResult};
use borsh::{BorshSerialize, BorshDeserialize};

// Define a struct to store in the map
#[derive(BorshSerialize, BorshDeserialize)]
struct UserProfile {
    username: String,
    reputation: u32,
}

// Create a map to store user profiles by account ID
// The prefix (0) is used to distinguish this map from other maps in storage
let user_profiles: Map<AccountId, UserProfile> = Map::new(0);

// Store a user profile
fn store_profile(
    account_id: &AccountId,
    profile: &UserProfile,
    env: &mut dyn Environment
) -> SdkResult<()> {
    user_profiles.set(account_id, profile, env)
}

// Retrieve a user profile
fn get_profile(
    account_id: &AccountId,
    env: &dyn Environment
) -> SdkResult<Option<UserProfile>> {
    user_profiles.get(account_id, env)
}

// Update a user's reputation
fn update_reputation(
    account_id: &AccountId,
    new_reputation: u32,
    env: &mut dyn Environment
) -> SdkResult<UserProfile> {
    user_profiles.update(account_id, |profile| {
        match profile {
            Some(mut existing_profile) => {
                existing_profile.reputation = new_reputation;
                Ok(existing_profile)
            }
            None => {
                // Create a new profile if it doesn't exist
                Ok(UserProfile {
                    username: "new_user".to_string(),
                    reputation: new_reputation,
                })
            }
        }
    }, env)
}
```

### Item

The `Item` collection provides a single-value store with the following operations:

- `new`: Create a new item with a specific prefix
- `set`: Store a value
- `get`: Retrieve the value
- `update`: Update the value using a callback function

#### Example

```rust
use evolve_collections::item::Item;
use evolve_core::{Environment, SdkResult};
use borsh::{BorshSerialize, BorshDeserialize};

// Define a struct to store as a configuration
#[derive(BorshSerialize, BorshDeserialize)]
struct ContractConfig {
    admin: AccountId,
    paused: bool,
    fee_percentage: u8,
}

// Create an item to store the contract configuration
// The prefix (1) is used to distinguish this item from other items in storage
let config: Item<ContractConfig> = Item::new(1);

// Initialize the configuration
fn initialize_config(
    admin: AccountId,
    env: &mut dyn Environment
) -> SdkResult<()> {
    let initial_config = ContractConfig {
        admin,
        paused: false,
        fee_percentage: 5,
    };

    config.set(&initial_config, env)
}

// Get the current configuration
fn get_config(env: &dyn Environment) -> SdkResult<Option<ContractConfig>> {
    config.get(env)
}

// Update the configuration
fn update_config(
    paused: bool,
    fee_percentage: u8,
    env: &mut dyn Environment
) -> SdkResult<ContractConfig> {
    config.update(|existing_config| {
        match existing_config {
            Some(mut cfg) => {
                cfg.paused = paused;
                cfg.fee_percentage = fee_percentage;
                Ok(cfg)
            }
            None => {
                // Return an error if the config doesn't exist
                Err(1) // Use appropriate error code
            }
        }
    }, env)
}
```

## Best Practices

1. **Use unique prefixes**: Each Map or Item should have a unique prefix to avoid key collisions in storage.

2. **Error handling**: Always handle errors returned by the collection methods, as storage operations may fail.

3. **Serialization**: The collections require types that implement `Encodable` and `Decodable` traits. The easiest way
   to achieve this is by using Borsh serialization with `BorshSerialize` and `BorshDeserialize` derive macros.

4. **Atomic updates**: Use the `update` method for atomic read-modify-write operations to avoid race conditions.

## Implementation Details

Both `Map` and `Item` collections store data in the blockchain's key-value storage. The `Map` collection prefixes each
key with a byte to distinguish between different maps. The `Item` collection is implemented as a `Map` with a unit type
`()` as the key.

The collections interact with the blockchain's storage through the `Environment` trait, which provides methods for
querying and executing storage operations.

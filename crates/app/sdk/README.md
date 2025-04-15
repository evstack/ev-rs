# Evolve

Evolve is an SDK designed to help you build powerful and flexible blockchain applications.

## Core Principles

At the heart of Evolve is the `AccountCode` trait, which specifies how an account responds to execution requests and
queries. Notably, this account is stateless and does not alter its own internal state. You typically do not need to
implement `AccountCode` manually; Evolve’s macros handle it for you.

In Evolve, every entity is an account. Each account is defined by an identifier (`AccountID`) and the `AccountCode` that
governs its behavior. An account’s state is encapsulated behind its unique `AccountID`.

### No Sudo

In Evolve, there is no concept of “sudo.” Whenever something is executed, it is because one account (which could be the
runtime itself, using a well-known account ID) has requested it. The account receiving the request then decides whether
or not to process it. This design ensures that each account retains complete control over permissions, making it harder
to inadvertently break invariants—unlike in certain other frameworks (e.g., Cosmos-SDK) where there are no strict
execution guarantees.

### Composition and Extensibility

Accounts in Evolve can be composed into higher-level primitives or “pre-compiles.” The SDK aims to keep its API and
trait surfaces minimal and intuitive, allowing even less experienced Rust developers to onboard quickly. Given the
evolving nature of blockchain technology, Evolve avoids over-parameterizing its core interfaces; instead, custom logic
is addressed by creating specialized accounts.

For example, there is no native “block trait” within the core interfaces. If block-related data needs to be made
available, you could create a dedicated block info account that other accounts can query. The same pattern applies to
validators or any specialized consensus engine information.

## Writing an Account

Below is a **merged** guide on creating an account in Evolve with `init`, `exec`, and `query` methods, along with
details about **auto-generated references** (`AccountRef`) and the broader set of **collection types** available. This
single document covers:

1. **Core Principles** (No sudo, macro-based trait implementation)
2. **Collection Types** (e.g., `Item`, `Map`, etc.)
3. **AccountRef Generation** (client interface for inter-account communication)
4. **Step-by-Step Explanation** of the example code

---

## Evolve Overview

Evolve is an SDK designed for building powerful and flexible blockchain applications in Rust.

- **No Sudo**: There is no concept of a “super-user.” Every transaction or method call is triggered by an account.
- **Macros for AccountCode**: You don’t directly implement `AccountCode`; instead, Evolve provides macros—`#[init]`,
  `#[exec]`, and `#[query]`—that automatically generate the necessary trait logic.
- **Everything Is an Account**: Each account is defined by an `AccountID` plus the logic (the “code”) behind it. State
  is isolated per `AccountID`.

### About Collection Types

When defining an account’s storage, Evolve enforces the use of **collection** types—`Item`, `Map`, `Vector`, and
others—to ensure safe and consistent on-chain data handling. You can refer to
the [Evolve Collections Documentation](https://github.com/evolve-docs) (placeholder link) for a deep dive into all
available collection types. In the example below, we use `Item<T>`, but you can choose a different collection type if
your use case demands it.

### Auto-Generated References (AccountRef)

By annotating a Rust module with `#[account_impl(YourAccountStruct)]`, Evolve generates a **client interface** (commonly
referred to as `AccountRef`) that other accounts or off-chain code can use to interact with the account. Specifically:

- **`init` Method**: The reference exposes a function (named after your `#[init]` method) to deploy and initialize your
  account.
    - The signature typically returns `SdkResult<(AccountRef, InitReturnType)>`, where:
        - `AccountRef` is a handle to the newly created account (for subsequent calls).
        - `InitReturnType` matches the Rust return type of your init method.
- **`exec` Methods**: Correspond to each `#[exec]`-annotated method in your code.
- **`query` Methods**: Correspond to each `#[query]`-annotated function, enabling read-only calls.

This auto-generation saves a significant amount of boilerplate and ensures consistent, type-safe interaction across
different Evolve accounts.

---

## Example: A Pool Account

Below, we define a **Pool** account that also deploys a token (**LP token**). Users can deposit two different fungible
assets into the pool in exchange for LP tokens, and they can later burn LP tokens to withdraw their share. Notice how
`#[init]`, `#[exec]`, and `#[query]` methods are used.

```rust
use evolve_macros::account_impl;

// Account impl macro defines who is the one implementing the AccountCode trait.
// The AccountCode trait is implemented based on exec/init/query methods 
// which are marked using macro attributes.
#[account_impl(Account)]
pub mod pool {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::{Environment, ErrorCode, FungibleAsset, SdkResult};
    use evolve_fungible_asset::{FungibleAssetInterfaceRef, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};
    use evolve_token::account::TokenRef;

    const ERR_INVALID_FUNDS_AMOUNT: ErrorCode = ErrorCode::new(0, "invalid funds amount");
    const ERR_INVALID_ASSET_ID: ErrorCode = ErrorCode::new(1, "invalid asset id");

    #[derive(BorshDeserialize, BorshSerialize)]
    pub struct PoolState {
        pub asset_one: FungibleAsset,
        pub asset_two: FungibleAsset,
    }

    // Define the account and what it has inside of state.
    // Only 'collection' types are allowed (Item, Map, Vector, etc.).
    pub struct Account {
        // A Ref is an auto-generated client to talk with another account.
        // The methods of this Ref are derived from the Query/Exec/Init methods on the account.
        lp_token: Item<TokenRef>,

        asset_one: Item<FungibleAssetInterfaceRef>,
        asset_two: Item<FungibleAssetInterfaceRef>,
    }

    impl Account {
        // The prefixes (0,1,2) must be unique for each field to avoid collisions.
        pub const fn new() -> Self {
            Account {
                lp_token: Item::new(0),
                asset_one: Item::new(1),
                asset_two: Item::new(2),
            }
        }

        // The #[init(payable)] attribute marks this as the sole initialization method,
        // and also allows the method to receive a Vec<FungibleAsset> (funds).
        #[init(payable)]
        pub fn initialize(
            self: &Account,
            mut initial_funds: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if initial_funds.len() != 2 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }
            let asset_two = initial_funds.pop().unwrap();
            let asset_one = initial_funds.pop().unwrap();

            // Initialize an LP token account and get back its reference.
            let (lp_token, _) = TokenRef::initialize(
                FungibleAssetMetadata {
                    name: "lp".to_string(),
                    symbol: "LP".to_string(),
                    decimals: 0,
                    icon_url: "".to_string(),
                    description: "".to_string(),
                },
                vec![],
                Some(env.sender()),
                env,
            )?;

            // Store references to the LP token and underlying assets.
            self.lp_token.set(&lp_token, env)?;
            self.asset_one
                .set(&FungibleAssetInterfaceRef::new(asset_one.asset_id), env)?;
            self.asset_two
                .set(&FungibleAssetInterfaceRef::new(asset_two.asset_id), env)?;

            Ok(())
        }

        // Marked with #[exec(payable)], meaning this method is called with a mutable Environment
        // and can receive fungible assets.
        #[exec(payable)]
        pub fn deposit(
            self: &Account,
            mut inputs: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if inputs.len() != 2 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }

            let asset_two = inputs.pop().unwrap();
            let asset_one = inputs.pop().unwrap();

            // Validate that the correct assets are being deposited.
            let asset_one_ref = self.asset_one.get(env)?;
            if asset_one_ref.0 != asset_one.asset_id {
                return Err(ERR_INVALID_ASSET_ID);
            }
            let asset_two_ref = self.asset_two.get(env)?;
            if asset_two_ref.0 != asset_two.asset_id {
                return Err(ERR_INVALID_ASSET_ID);
            }

            // Compute how many LP tokens to mint for the user (example logic).
            let lp_to_user: u128 = 100;

            let lp_token = self.lp_token.get(&env)?;
            lp_token.mint(env.sender(), lp_to_user, env)?;

            Ok(())
        }

        #[exec(payable)]
        pub fn burn(
            self: &Account,
            mut lp_in: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if lp_in.len() != 1 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }

            let lp = lp_in.pop().unwrap();
            // Check that the token being transferred is our own LP token.
            if lp.asset_id != env.whoami() {
                return Err(ERR_INVALID_ASSET_ID);
            }

            // (Placeholder) Calculate how many underlying assets to return to the user.
            let asset_one_out = 100u128;
            let asset_two_out = 100u128;

            // Burn the LP tokens and transfer out the underlying assets.
            self.lp_token.get(env)?.burn(env.sender(), lp.amount, env)?;
            self.asset_one
                .get(env)?
                .transfer(env.sender(), asset_one_out, env)?;
            self.asset_two
                .get(env)?
                .transfer(env.sender(), asset_two_out, env)?;

            Ok(())
        }

        // Marked with #[query], meaning it's a read-only method that cannot change on-chain state.
        #[query]
        pub fn pool_state(self: &Account, env: &dyn Environment) -> SdkResult<PoolState> {
            let asset_one = self.asset_one.get(env)?;
            let asset_two = self.asset_two.get(env)?;

            Ok(PoolState {
                asset_one: FungibleAsset {
                    asset_id: asset_one.0,
                    amount: asset_one.get_balance(env.whoami(), env)?.unwrap_or_default(),
                },
                asset_two: FungibleAsset {
                    asset_id: asset_two.0,
                    amount: asset_two.get_balance(env.whoami(), env)?.unwrap_or_default(),
                },
            })
        }
    }
}
```

---

## How the Macro Works

### 1. Macro Annotation

```rust
#[account_impl(Account)]
pub mod pool { ... }
```

- This annotation informs Evolve to generate an **`AccountRef`** (client interface) for the `Account` struct inside the
  `pool` module.

### 2. Generated Methods in `AccountRef`

1. **`initialize`** (matching your `#[init]` function)
    - Usage:
      ```rust
      let (pool_ref, init_result) = PoolRef::initialize(...args...)?;
      ```  
    - Returns `(AccountRef, InitReturnType)`, where `AccountRef` is your client handle, and `InitReturnType` is whatever
      the Rust init method returns (commonly `SdkResult<()>`).

2. **`deposit`, `burn`, etc.** (matching each `#[exec]` function)
    - Usage:
      ```rust
      pool_ref.deposit(...args...)?;
      ```
    - Under the hood, this calls into the on-chain `deposit` method, passing required parameters.

3. **`pool_state`** (matching the `#[query]` function)
    - Usage:
      ```rust
      let state = pool_ref.pool_state()?;
      ```
    - Invokes the read-only method for retrieving current on-chain data.

---

## Key Takeaways

1. **Macros Reduce Boilerplate**
    - You only write `#[init]`, `#[exec]`, and `#[query]`; Evolve expands these into trait implementations behind the
      scenes.

2. **Isolation by Design**
    - State is accessed exclusively via typed collections (`Item`, `Map`, `Vector`, etc.), minimizing accidental
      overlaps.

3. **Auto-Generated AccountRef**
    - When you decorate your module with `#[account_impl(Account)]`, an **account reference** is created with the **same
      ** method signatures for `init`, `exec`, and `query`.
    - This reference is how other accounts (or user-facing code) interact with your account.

4. **No Sudo**
    - Every action must be requested by an account (or the special runtime account). There is no privileged
      “super-user.”

5. **Flexibility and Extensibility**
    - Core Evolve interfaces are intentionally minimal. For more specialized behaviors (e.g., block info, validators),
      you can create additional accounts that each implement specific logic.

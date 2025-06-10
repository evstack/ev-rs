# ADR-01: Genesis & Initialization Flow

## Status

Accepted

## Context

Evolve is an account-based blockchain runtime where all functionality is implemented as accounts. Currently, account code is statically linked at compile time, but the architecture is designed to support dynamic loading of account code in the future (e.g., as WASM modules or other formats). This forward-looking design requires a flexible genesis mechanism that can:

1. Initialize accounts with custom logic and state
2. Handle dependencies between accounts during genesis
3. Support complex initialization flows where one account's initialization depends on another's response
4. Maintain compatibility with different genesis formats and encoding schemes
5. Abstract account creation to work seamlessly with both current static linking and future dynamic loading

## What We Want to Achieve

- **Flexible Account Initialization**: Each account type can define its own initialization logic through custom `#[init]` methods
- **Dependency Management**: Support initialization flows where accounts depend on each other (e.g., a scheduler needs to know about block info service)
- **Runtime-Agnostic Genesis**: The runtime doesn't assume any specific genesis encoding - applications define their own genesis flow
- **Developer-Friendly API**: Simple, type-safe initialization through generated wrapper methods
- **Composable Genesis**: Ability to chain account initializations and use responses from previous initializations

## How We Achieve It

### 1. Account Initialization via `#[init]` Attribute

Accounts define their initialization logic using the `#[init]` attribute on a method. The `evolve_macros::account_impl` macro processes this to generate the necessary boilerplate.

**Example from Escrow account:**

```rust
#[account_impl(Escrow)]
pub mod escrow {
    impl Escrow {
        #[init]
        pub fn initialize(
            &self,
            unique_account: AccountId,
            block_info_account: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.unique.set(&UniqueRef::from(unique_account), env)?;
            self.block_info.set(&BlockInfoRef::from(block_info_account), env)?;
            Ok(())
        }
    }
}
```

### 2. Generated Wrapper Methods

The macro generates a `{Account}Ref` wrapper struct with a constructor method that:

- Creates the account via the runtime's `CreateAccount` function
- Returns both the new account ID and the initialization response
- Handles encoding/decoding automatically

**Generated for Escrow:**

```rust
pub struct EscrowRef(pub AccountId);

impl EscrowRef {
    pub fn initialize(
        unique_account: AccountId,
        block_info_account: AccountId,
        env: &mut dyn Environment
    ) -> SdkResult<(Self, ())> {
        let (acc_id, resp) = evolve_core::low_level::create_account(
            "Escrow".to_string(),
            &InitializeMsg { unique_account, block_info_account },
            vec![],
            env,
        )?;
        Ok((acc_id.into(), resp))
    }
}
```

### 3. Runtime Account Creation

Under the hood, account creation is handled by the runtime through the `CreateAccount` message:

```rust
pub struct CreateAccountRequest {
    pub code_id: String,        // Account type identifier (currently maps to statically linked code)
    pub init_message: Message,  // Serialized initialization parameters
}
```

The STF (State Transition Function) processes this by:

1. Assigning a new unique account ID
2. Associating the account with its code type via the `code_id` string identifier
3. Looking up the account code implementation (currently from statically linked registry)
4. Calling the account's `init` method with the provided parameters

**Current Implementation**: Account codes are registered at compile time through the `install_account_codes` function and stored in an `AccountsCodeStorage` that maps string identifiers to statically linked `AccountCode` implementations.

**Future Vision**: The same `code_id` mechanism will support dynamic loading, where the runtime can load account implementations from WASM modules, shared libraries, or other formats without changing the genesis flow or account creation API.

### 4. Application Genesis Implementation

Applications implement their genesis flow as a Rust function that orchestrates account creation:

**Example from testapp:**

```rust
pub fn do_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
    stf: &CustomStf,
    codes: &'a A,
    storage: &'a S,
) -> SdkResult<ExecutionState<'a, S>> {
    let genesis_height = 0;
    let genesis_time_unix_ms = 0;

    let (_, state) = stf.sudo(storage, codes, genesis_height, |env| {
        // 1. Create name service first (no dependencies)
        let ns_acc = NameServiceRef::initialize(vec![], env)?.0;

        // 2. Create token with initial balances
        let atom = TokenRef::initialize(
            FungibleAssetMetadata {
                name: "uatom".to_string(),
                symbol: "ATOM".to_string(),
                decimals: 6,
                icon_url: "https://lol.wtf".to_string(),
                description: "The atom coin".to_string(),
            },
            vec![(ALICE, 1000), (BOB, 2000)],  // Initial balances
            Some(MINTER),                       // Minter account
            env,
        )?.0;

        // 3. Create block info service
        let block_info = BlockInfoRef::initialize(
            genesis_height,
            genesis_time_unix_ms,
            env
        )?.0;

        // 4. Create scheduler (depends on block_info)
        let scheduler_acc = SchedulerRef::initialize(
            vec![block_info.0],  // begin blockers
            vec![],              // end blockers
            env
        )?.0;

        // 5. Create gas service
        let gas_service_acc = GasServiceRef::initialize(
            StorageGasConfig {
                storage_get_charge: 10,
                storage_set_charge: 10,
                storage_remove_charge: 10,
            },
            env,
        )?.0;

        // 6. Create PoA consensus (depends on scheduler)
        let poa = PoaRef::initialize(
            RUNTIME_ACCOUNT_ID,
            scheduler_acc.0,
            vec![],  // initial validators
            env
        )?.0;

        // 7. Register well-known names
        ns_acc.updates_names(
            vec![
                ("runtime".to_string(), RUNTIME_ACCOUNT_ID),
                ("storage".to_string(), STORAGE_ACCOUNT_ID),
                ("scheduler".to_string(), scheduler_acc.0),
                ("atom".to_string(), atom.0),
                ("gas".to_string(), gas_service_acc.0),
                ("poa".to_string(), poa.0),
                ("block_info".to_string(), block_info.0),
            ],
            env,
        )?;

        // 8. Configure scheduler with PoA as begin blocker
        scheduler_acc.update_begin_blockers(vec![poa.0], env)?;

        Ok(())
    })?;

    Ok(state)
}
```

## Key Differences from Cosmos SDK

| Aspect               | Cosmos SDK                                     | Evolve                                                                        |
| -------------------- | ---------------------------------------------- | ----------------------------------------------------------------------------- |
| **Module Init**      | Static `InitGenesis` functions called in order | Dynamic account creation with custom `#[init]` methods                        |
| **Dependencies**     | Manual ordering of module initialization       | Explicit dependency passing via initialization parameters                     |
| **Genesis Format**   | Predefined JSON structure                      | Application-defined, encoding-agnostic                                        |
| **State Management** | Module-specific genesis state                  | Account-based state initialization                                            |
| **Code Loading**     | Modules statically linked at compile time      | Currently static, designed for future dynamic loading (WASM, etc.)            |
| **Extensibility**    | Requires framework changes for new modules     | New account types can be added (currently at compile time, future at runtime) |

## Implementation Details

### Macro Processing

The `#[account_impl]` macro:

1. Scans for `#[init]` methods in the account implementation
2. Generates message structs for initialization parameters
3. Implements `AccountCode` trait with proper message dispatch
4. Creates wrapper structs with type-safe initialization methods

### Runtime Integration

The STF handles account creation through:

1. **Account ID Assignment**: Sequential unique IDs for new accounts
2. **Code Association**: Mapping account IDs to their code type identifiers
3. **Code Resolution**: Looking up account implementations (currently static, future dynamic)
4. **Initialization Execution**: Calling the account's init method in a controlled environment
5. **State Management**: Ensuring initialization happens in a clean state context

### Error Handling

- **Initialization Failures**: If an account's init method fails, the entire genesis fails
- **Dependency Errors**: Missing dependencies are caught at runtime during initialization
- **State Consistency**: Failed initializations don't leave partial state

## Benefits

1. **Type Safety**: Compile-time verification of initialization parameters
2. **Flexibility**: Each account defines its own initialization logic
3. **Composability**: Complex genesis flows can be built by chaining account creations
4. **Maintainability**: Clear separation between account logic and genesis orchestration
5. **Testability**: Genesis flows can be unit tested like regular Rust functions
6. **Future-Proof Architecture**: Design supports transition from static to dynamic account loading
7. **Code Reusability**: Account implementations can be shared across different applications

## Example: Adding a New Account to Genesis

To add a new account type to genesis:

1. **Define the account with init method:**

```rust
#[account_impl(MyAccount)]
pub mod my_account {
    impl MyAccount {
        #[init]
        pub fn initialize(
            &self,
            config: MyConfig,
            env: &mut dyn Environment,
        ) -> SdkResult<MyResponse> {
            // Initialization logic
            Ok(MyResponse { success: true })
        }
    }
}
```

2. **Add to account codes (current static approach):**

```rust
pub fn install_account_codes(codes: &mut impl WritableAccountsCodeStorage) {
    codes.add_code(MyAccount::new()).unwrap();
    // ... other accounts
}
```

**Note**: In the future, this step may be replaced by dynamic loading mechanisms where account code is loaded from external sources (WASM modules, etc.) rather than being statically compiled into the application.

3. **Include in genesis:**

```rust
pub fn do_genesis(/* ... */) -> SdkResult<ExecutionState> {
    stf.sudo(storage, codes, genesis_height, |env| {
        // ... other account creation

        let my_account = MyAccountRef::initialize(
            MyConfig { /* ... */ },
            env
        )?.0;

        // Use my_account.0 (AccountId) in other initializations
        Ok(())
    })
}
```

# ADR-02: Account-Based Authentication System

## Status

Accepted

## Context

Evolve is an account-based blockchain runtime where all functionality is implemented as accounts. Unlike traditional
blockchain frameworks that impose a single authentication mechanism across all transactions, Evolve's architecture
allows each account to define its own authentication logic. This flexibility is crucial for supporting diverse
authentication schemes (signatures, multi-sig, smart contract logic, etc.) while maintaining a clean and
developer-friendly API.

The authentication system must handle:

1. **Multiple Authentication Schemes**: Different accounts may use different signature algorithms, nonce systems, or
   authorization logic
2. **Multiple Transaction Types**: A single account might need to authenticate different types of transactions with
   varying structures
3. **Performance vs. Flexibility Trade-offs**: Balance runtime efficiency with developer experience and system
   flexibility
4. **Type Safety**: Maintain compile-time safety while allowing runtime flexibility in transaction handling
5. **Developer Experience**: Keep the API simple and avoid leaking complex type parameters across the entire system

## What We Want to Achieve

- **Flexible Authentication**: Each account can implement custom authentication logic tailored to its specific needs
- **Multi-Transaction Support**: Accounts can authenticate multiple transaction types from the same authentication
  implementation
- **Developer-Friendly API**: Simple, type-safe authentication without complex type parameter propagation
- **Runtime Efficiency**: Minimal overhead while maintaining flexibility
- **Extensibility**: Easy to add new authentication schemes without framework changes
- **Composable Security**: Authentication logic can build upon other account services (nonces, multi-sig, etc.)

## How We Achieve It

### 1. Account-Based Authentication via `AuthenticationInterface`

Accounts implement authentication by providing the `AuthenticationInterface` trait. The `evolve_macros::account_impl`
macro processes this to generate the necessary boilerplate for secure message validation.

**Core Authentication Trait:**

```rust
#[account_impl(AuthenticationInterface)]
pub mod auth_interface {
    pub trait AuthenticationInterface {
        #[exec]
        fn authenticate(
            &self,
            tx: evolve_core::Message,  // Generic message container
            env: &mut dyn Environment,
        ) -> SdkResult<()>;
    }
}
```

### 2. Generic Message Design

The authentication system uses `evolve_core::Message` as a generic container instead of parameterizing over transaction
types. This design choice prioritizes developer experience:

**Benefits of Generic Messages:**

- **No Type Parameter Leakage**: Transaction types don't propagate through the entire account system
- **Multi-Transaction Support**: Single accounts can handle multiple transaction formats
- **Clean APIs**: Account interfaces remain simple and focused
- **Advanced Use Case Containment**: Authentication complexity is isolated to accounts that need it

**Example Implementation:**

```rust
#[derive(BorshSerialize, BorshDeserialize, Clone)]
struct EthTx {
    nonce: u64,
    sig: [u8; 64],
    // ... other fields
}

struct EthAccount {
    nonce: Item<u64>,
}

impl AuthenticationInterface for EthAccount {
    fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
        // Cast generic message to expected transaction type
        let tx: EthTx = tx.get()?;

        // Validate nonce
        let current_nonce = self.nonce.get(env)?;
        if tx.nonce != current_nonce {
            return Err(ERR_UNAUTHORIZED);
        }

        // Validate signature
        if !self.verify_signature(&tx) {
            return Err(ERR_UNAUTHORIZED);
        }

        // Update nonce state
        self.nonce.set(&(current_nonce + 1), env)?;

        Ok(())
    }
}
```

### 3. Transaction Validation Integration

The authentication system integrates seamlessly with the runtime's transaction validation through the
`AuthenticationTxValidator`:

```rust
pub struct AuthenticationTxValidator<Tx>(PhantomData<Tx>);

impl<T: Transaction + Clone + Encodable + Decodable> TxValidator<T>
    for AuthenticationTxValidator<T>
{
    fn validate_tx(&self, tx: &T, env: &mut dyn Environment) -> SdkResult<()> {
        // Trigger authentication on the sender account
        auth_interface::AuthenticationInterfaceRef::new(tx.sender())
            .authenticate(Message::new(tx)?, env)
            .map_err(|e| {
                if e == ERR_UNKNOWN_FUNCTION {
                    return ERR_NOT_EOA;  // Not an Externally Owned Account
                }
                e
            })
    }
}
```

### 4. Multi-Transaction Type Support

A single account can authenticate multiple transaction types by pattern matching or using transaction type indicators:

```rust
impl AuthenticationInterface for MultiTxAccount {
    fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
        // Try different transaction formats
        if let Ok(eth_tx) = tx.get::<EthTx>() {
            return self.authenticate_eth_tx(eth_tx, env);
        }

        if let Ok(secp_tx) = tx.get::<Secp256k1Tx>() {
            return self.authenticate_secp_tx(secp_tx, env);
        }

        Err(ERR_UNSUPPORTED_TX_TYPE)
    }
}
```

### 5. State Management and Nonce Handling

Authentication implementations manage their own state, including nonces, counters, or any other authentication-related
data:

```rust
use evolve_ns::resolve_as_ref;
use evolve_block_info::account::BlockInfoRef;

struct AdvancedAuthAccount {
    nonce: Item<u64>,
    failed_attempts: Item<u32>,
    locked_until: Item<u64>,
    authorized_keys: UnorderedMap<PublicKey, Permission>,
}

impl AuthenticationInterface for AdvancedAuthAccount {
    fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
        let tx: SecureTx = tx.get()?;

        // Check if account is locked
        let block_info = resolve_as_ref::<BlockInfoRef>("block_info", env)?.unwrap();
        let current_time = block_info.get_time_unix_ms(env)?;
        if current_time < self.locked_until.get(env)? {
            return Err(ERR_ACCOUNT_LOCKED);
        }

        // Validate transaction
        let result = self.verify_transaction(&tx, env);

        if result.is_err() {
            // Increment failed attempts
            let failures = self.failed_attempts.get(env)? + 1;
            self.failed_attempts.set(&failures, env)?;

            // Lock account after too many failures
            if failures >= MAX_FAILED_ATTEMPTS {
                self.locked_until.set(&(current_time + LOCK_DURATION), env)?;
            }
        } else {
            // Reset failed attempts on success
            self.failed_attempts.set(&0, env)?;
            // Update nonce
            self.nonce.set(&(self.nonce.get(env)? + 1), env)?;
        }

        result
    }
}
```

## Key Differences from Traditional Blockchain Authentication

| Aspect                   | Traditional (e.g., Cosmos SDK)            | Evolve                                                 |
|--------------------------|-------------------------------------------|--------------------------------------------------------|
| **Authentication Logic** | Fixed signature verification in framework | Account-defined authentication logic                   |
| **Transaction Types**    | Single transaction format per chain       | Multiple transaction formats per account               |
| **Signature Schemes**    | Limited to framework-supported algorithms | Any signature scheme implementable in account logic    |
| **Nonce Management**     | Framework-managed account sequences       | Account-controlled state management                    |
| **Multi-Sig Support**    | Requires special framework modules        | Implemented as regular account authentication logic    |
| **Extensibility**        | Requires framework changes                | New authentication schemes via account implementations |
| **Type Safety**          | Complex type parameters throughout system | Generic messages with runtime casting                  |

## Implementation Details

### Message Casting Performance

The system uses runtime type casting via `Message::get<T>()` instead of compile-time type parameters:

**Performance Characteristics:**

- **Casting Overhead**: Minimal runtime cost for deserialization
- **Memory Efficiency**: Single message representation regardless of transaction type
- **Development Speed**: Faster compilation and simpler type checking

**Trade-off Justification:**
The slight performance cost of message casting is acceptable because:

1. Authentication is not the performance bottleneck in most blockchain operations
2. The flexibility gained enables much more sophisticated authentication schemes
3. Developer productivity improvements outweigh marginal runtime costs
4. Authentication logic is typically not called in tight loops

### Error Handling

The authentication system provides specific error codes:

```rust
pub const ERR_NOT_EOA: ErrorCode = ErrorCode::new(4111, "not an externally owned account");
// Additional authentication errors defined by individual accounts
```

When `ERR_UNKNOWN_FUNCTION` is returned (account doesn't implement authentication), it's translated to `ERR_NOT_EOA` to
indicate the account is not externally owned.

### Account Code Registration

Authentication-capable accounts are registered with the runtime like any other account type:

```rust
pub fn install_account_codes(codes: &mut impl WritableAccountsCodeStorage) {
    codes.add_code(EthAccount::new()).unwrap();
    codes.add_code(MultiSigAccount::new()).unwrap();
    codes.add_code(SmartContractAccount::new()).unwrap();
    // ... other accounts
}
```

## Benefits

1. **Maximum Flexibility**: Each account can implement any authentication scheme
2. **Multiple Transaction Support**: Single accounts can handle diverse transaction formats
3. **Clean Architecture**: Authentication complexity is contained within accounts that need it
4. **Developer Experience**: Simple APIs without complex type parameter propagation
5. **Extensibility**: New authentication schemes don't require framework changes
6. **Composability**: Authentication can leverage other account services and state
7. **Future-Proof**: Design accommodates unknown future authentication requirements
8. **Testing**: Authentication logic can be unit tested independently
9. **Security**: Account-level isolation prevents authentication bugs from affecting other accounts

## Example: Implementing Multi-Signature Authentication

To implement a multi-signature account with custom authentication:

1. **Define the account with authentication:**

```rust
#[account_impl(MultiSigAccount, AuthenticationInterface)]
pub mod multi_sig_account {
    #[derive(BorshSerialize, BorshDeserialize)]
    struct MultiSigTx {
        nonce: u64,
        signature: Vec<u8>,  // Composite signature containing multiple signatures
    }

    struct MultiSigAccount {
        nonce: Item<u64>,
        required_signatures: Item<u32>,
        authorized_signers: Vector<PublicKey>,
    }

    impl AuthenticationInterface for MultiSigAccount {
        fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
            let tx: MultiSigTx = tx.get()?;

            // Validate nonce
            if tx.nonce != self.nonce.get(env)? {
                return Err(ERR_UNAUTHORIZED);
            }

            // Decompose the composite signature internally
            let signatures = self.decompose_multisig_signature(&tx.signature)?;

            // Validate minimum signature count
            let required = self.required_signatures.get(env)?;
            if signatures.len() < required as usize {
                return Err(ERR_INSUFFICIENT_SIGNATURES);
            }

            // Verify each signature is from an authorized signer
            let mut valid_sigs = 0;
            for (sig_bytes, pubkey) in signatures {
                if self.is_authorized_signer(&pubkey, env)? {
                    if self.verify_signature(&tx, &sig_bytes, &pubkey)? {
                        valid_sigs += 1;
                    }
                }
            }

            if valid_sigs < required {
                return Err(ERR_INSUFFICIENT_VALID_SIGNATURES);
            }

            // Update nonce
            self.nonce.set(&(tx.nonce + 1), env)?;

            Ok(())
        }
    }

    impl MultiSigAccount {
        #[init]
        pub fn initialize(
            &self,
            signers: Vec<PublicKey>,
            required_signatures: u32,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            for signer in signers {
                self.authorized_signers.push(&signer, env)?;
            }
            self.required_signatures.set(&required_signatures, env)?;
            self.nonce.set(&0, env)?;
            Ok(())
        }

        /// Decomposes a composite signature into individual signatures and their public keys
        fn decompose_multisig_signature(
            &self,
            composite_sig: &[u8],
        ) -> SdkResult<Vec<([u8; 64], PublicKey)>> {
            // This is a simplified example - real implementation would handle
            // proper encoding/decoding of the composite signature format
            let mut signatures = Vec::new();
            let mut offset = 0;

            // Read number of signatures (first 4 bytes)
            if composite_sig.len() < 4 {
                return Err(ERR_INVALID_SIGNATURE_FORMAT);
            }

            let sig_count = u32::from_le_bytes([
                composite_sig[0], composite_sig[1],
                composite_sig[2], composite_sig[3]
            ]);
            offset += 4;

            for _ in 0..sig_count {
                // Each entry: 64 bytes signature + 32 bytes public key
                if offset + 96 > composite_sig.len() {
                    return Err(ERR_INVALID_SIGNATURE_FORMAT);
                }

                let mut sig_bytes = [0u8; 64];
                sig_bytes.copy_from_slice(&composite_sig[offset..offset + 64]);
                offset += 64;

                let mut pubkey_bytes = [0u8; 32];
                pubkey_bytes.copy_from_slice(&composite_sig[offset..offset + 32]);
                offset += 32;

                let pubkey = PublicKey::from_bytes(pubkey_bytes)?;
                signatures.push((sig_bytes, pubkey));
            }

            Ok(signatures)
        }
    }
}
```

2. **Use in application:**

```rust
// During genesis or runtime
let multi_sig = MultiSigAccountRef::initialize(
    vec![alice_pubkey, bob_pubkey, charlie_pubkey],
    2, // Require 2 of 3 signatures
    env,
)?.0;

// The account will now authenticate multi-signature transactions automatically
// when used as a transaction sender
```

## Advanced Use Cases

### Smart Contract Accounts

Accounts can implement sophisticated authentication logic equivalent to smart contract wallets:

```rust
impl AuthenticationInterface for SmartWalletAccount {
    fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
        let tx: SmartWalletTx = tx.get()?;

        // Execute custom authentication logic
        self.execute_auth_script(tx.auth_script, &tx, env)?;

        // Update any relevant state
        let block_info = resolve_as_ref::<BlockInfoRef>("block_info", env)?.unwrap();
        let current_time = block_info.get_time_unix_ms(env)?;
        self.last_tx_time.set(&current_time, env)?;

        Ok(())
    }
}
```

### Time-Locked Authentication

```rust
impl AuthenticationInterface for TimeLockAccount {
    fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
        let tx: TimeLockTx = tx.get()?;

        // Check time constraints
        let block_info = resolve_as_ref::<BlockInfoRef>("block_info", env)?.unwrap();
        let current_time = block_info.get_time_unix_ms(env)?;
        if current_time < self.unlock_time.get(env)? {
            return Err(ERR_TIME_LOCKED);
        }

        // Proceed with signature verification
        self.verify_signature(&tx, env)
    }
}
```

This authentication system provides the foundation for implementing any conceivable authentication scheme while
maintaining clean APIs and excellent developer experience.

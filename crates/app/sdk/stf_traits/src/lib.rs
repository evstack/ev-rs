use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, ErrorCode, FungibleAsset, InvokeRequest,
    InvokeResponse, Message, ReadonlyKV, SdkResult,
};
use rayon::prelude::*;

/// Optional bootstrap metadata for sender account auto-registration.
///
/// Transaction types can return this when STF should ensure the sender account
/// is initialized before validation/execution.
#[derive(Clone, Debug)]
pub struct SenderBootstrap {
    /// Account code identifier to register if sender account is missing.
    pub account_code_id: &'static str,
    /// Account-type-specific init message for bootstrap registration.
    pub init_message: Message,
}

pub trait Transaction {
    fn sender(&self) -> AccountId;
    fn recipient(&self) -> AccountId;
    fn request(&self) -> &InvokeRequest;
    fn gas_limit(&self) -> u64;
    fn funds(&self) -> &[FungibleAsset];
    fn compute_identifier(&self) -> [u8; 32];

    /// Optional sender bootstrap primitive for STF account auto-registration.
    fn sender_bootstrap(&self) -> Option<SenderBootstrap> {
        None
    }
}

pub trait TxDecoder<T> {
    fn decode(&self, bytes: &mut &[u8]) -> SdkResult<T>;
}

pub trait Block<Tx> {
    fn context(&self) -> BlockContext;
    // TODO: make mut
    fn txs(&self) -> &[Tx];

    /// Returns the maximum gas allowed for this block.
    ///
    /// The STF will stop processing transactions once cumulative gas
    /// exceeds this limit. Defaults to `u64::MAX` (no limit) for
    /// backwards compatibility.
    fn gas_limit(&self) -> u64 {
        u64::MAX
    }
}

pub trait TxValidator<Tx> {
    fn validate_tx(&self, tx: &Tx, env: &mut dyn Environment) -> SdkResult<()>;
}

pub trait PostTxExecution<Tx> {
    fn after_tx_executed(
        tx: &Tx,
        gas_consumed: u64,
        tx_result: SdkResult<InvokeResponse>,
        env: &mut dyn Environment,
    ) -> SdkResult<()>;
}

pub trait BeginBlocker<B> {
    fn begin_block(&self, block: &B, env: &mut dyn Environment);
}

pub trait EndBlocker {
    fn end_block(&self, env: &mut dyn Environment);
}

/// Stores account code.
pub trait AccountsCodeStorage {
    /// TODO: this probably needs to consume gas, and should accept gas.
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R;

    /// Returns a list of all registered account code identifiers.
    fn list_identifiers(&self) -> Vec<String>;
}

/// Extension to also add more codes.
pub trait WritableAccountsCodeStorage: AccountsCodeStorage {
    fn add_code(&mut self, code: impl AccountCode + 'static) -> Result<(), ErrorCode>;
}

#[derive(Debug)]
pub enum StateChange {
    Set { key: Vec<u8>, value: Vec<u8> },
    Remove { key: Vec<u8> },
}

pub trait WritableKV: ReadonlyKV {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode>;
}

/// Trait for extracting and verifying signatures from transactions.
///
/// This trait enables parallel signature verification by separating
/// the CPU-bound cryptographic work from state-touching validation.
///
/// # Performance
///
/// Signature verification is typically the most CPU-intensive part of
/// transaction validation. By implementing this trait, you can verify
/// signatures in parallel across multiple CPU cores before running
/// sequential state validation.
pub trait SignatureVerifier<Tx> {
    /// Verifies the cryptographic signature of a transaction.
    ///
    /// This method should ONLY perform cryptographic verification.
    /// It must NOT access any state or modify anything.
    ///
    /// # Arguments
    /// * `tx` - The transaction to verify
    ///
    /// # Returns
    /// * `Ok(())` if the signature is valid
    /// * `Err(ErrorCode)` if the signature is invalid or missing
    fn verify_signature(&self, tx: &Tx) -> SdkResult<()>;
}

/// Result of parallel signature verification for a single transaction.
#[derive(Debug, Clone)]
pub struct SignatureVerificationResult {
    /// Index of the transaction in the batch
    pub index: usize,
    /// Result of signature verification
    pub result: Result<(), ErrorCode>,
}

/// Verifies signatures for a batch of transactions in parallel.
///
/// This function uses rayon to parallelize signature verification across
/// all available CPU cores. It returns results for all transactions,
/// preserving the order.
///
/// # Arguments
/// * `txs` - Slice of transactions to verify
/// * `verifier` - The signature verifier implementation
///
/// # Returns
/// A vector of verification results, one per transaction, in order.
///
/// # Example
/// ```ignore
/// let results = verify_signatures_parallel(&transactions, &my_verifier);
///
/// // Filter out transactions with invalid signatures
/// let valid_txs: Vec<_> = transactions
///     .into_iter()
///     .zip(results.iter())
///     .filter(|(_, r)| r.result.is_ok())
///     .map(|(tx, _)| tx)
///     .collect();
/// ```
pub fn verify_signatures_parallel<Tx, V>(
    txs: &[Tx],
    verifier: &V,
) -> Vec<SignatureVerificationResult>
where
    Tx: Sync,
    V: SignatureVerifier<Tx> + Sync,
{
    txs.par_iter()
        .enumerate()
        .map(|(index, tx)| SignatureVerificationResult {
            index,
            result: verifier.verify_signature(tx),
        })
        .collect()
}

/// A no-op signature verifier that always succeeds.
///
/// Use this when your transaction type doesn't have traditional signatures
/// or when signature verification is handled elsewhere.
#[derive(Default, Clone, Copy)]
pub struct NoOpSignatureVerifier;

impl<Tx> SignatureVerifier<Tx> for NoOpSignatureVerifier {
    fn verify_signature(&self, _tx: &Tx) -> SdkResult<()> {
        Ok(())
    }
}

/// Filters a batch of transactions based on signature verification results.
///
/// Returns only the transactions that passed signature verification,
/// along with indices mapping to the original positions.
///
/// # Arguments
/// * `txs` - Vector of transactions
/// * `results` - Signature verification results from `verify_signatures_parallel`
///
/// # Returns
/// A tuple of (valid_transactions, original_indices)
pub fn filter_valid_signatures<Tx>(
    txs: Vec<Tx>,
    results: &[SignatureVerificationResult],
) -> (Vec<Tx>, Vec<usize>) {
    let mut valid_txs = Vec::with_capacity(txs.len());
    let mut indices = Vec::with_capacity(txs.len());

    for (tx, result) in txs.into_iter().zip(results.iter()) {
        if result.result.is_ok() {
            indices.push(result.index);
            valid_txs.push(tx);
        }
    }

    (valid_txs, indices)
}

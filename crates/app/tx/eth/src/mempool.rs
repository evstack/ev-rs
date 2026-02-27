//! Mempool transaction wrapper for Ethereum transactions.
//!
//! Wraps an Ethereum `TxEnvelope` and implements the `MempoolTx` trait
//! for use with the generic mempool.
//!
//! # Function Routing
//!
//! Routing is defined by `evolve_tx_eth` and derived from Ethereum calldata.
//! See `TxEnvelope::to_invoke_requests()` for the canonical mapping.
//!
//! Example: A CLOB account can expose `place_order`, `cancel_order`, etc., each with
//! their own Ethereum selector computed as `keccak256(signature)[0..4]`.

use alloy_primitives::{Address, B256};
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{AccountId, Environment, FungibleAsset, InvokeRequest, Message, SdkResult};
use evolve_mempool::{GasPriceOrdering, MempoolTx, SenderKey};
use evolve_stf_traits::{AuthenticationPayload, Transaction};

use crate::envelope::TxEnvelope;
use crate::eoa_registry::{
    lookup_account_id_in_env, lookup_contract_account_id_in_env, resolve_or_create_eoa_account,
};
use crate::error::ERR_RECIPIENT_REQUIRED;
use crate::traits::TypedTransaction;

/// A verified transaction ready for mempool storage.
///
/// Wraps an Ethereum `TxEnvelope` and caches derived values for efficient
/// access during block production.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The original Ethereum transaction envelope.
    envelope: TxEnvelope,
    /// The invoke request to execute (derived by evolve_tx).
    invoke_request: InvokeRequest,
    /// Gas price for ordering (effective gas price).
    effective_gas_price: u128,
}

impl TxContext {
    /// Create a new mempool transaction from an Ethereum envelope.
    ///
    /// Returns `None` if the transaction has no recipient (contract creation).
    pub fn new(envelope: TxEnvelope, base_fee: u128) -> Option<Self> {
        // TODO(vm): when EVM contract creation is supported, allow `to == None`
        // and route create-transactions through deployment execution instead of
        // rejecting them at mempool decode time.
        envelope.to()?;

        let invoke_request = envelope.to_invoke_requests().into_iter().next()?;

        // Calculate effective gas price for ordering
        let effective_gas_price = calculate_effective_gas_price(&envelope, base_fee);

        Some(Self {
            envelope,
            invoke_request,
            effective_gas_price,
        })
    }

    /// Get the transaction hash.
    pub fn hash(&self) -> B256 {
        self.envelope.tx_hash()
    }

    /// Get the sender address.
    pub fn sender_address(&self) -> Address {
        self.envelope.sender()
    }

    /// Get the nonce.
    pub fn nonce(&self) -> u64 {
        self.envelope.nonce()
    }

    /// Get the effective gas price for ordering.
    pub fn effective_gas_price(&self) -> u128 {
        self.effective_gas_price
    }

    /// Get the underlying envelope.
    pub fn envelope(&self) -> &TxEnvelope {
        &self.envelope
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> Option<u64> {
        self.envelope.chain_id()
    }
}

impl MempoolTx for TxContext {
    type OrderingKey = GasPriceOrdering;

    fn tx_id(&self) -> [u8; 32] {
        self.envelope.tx_hash().0
    }

    fn ordering_key(&self) -> Self::OrderingKey {
        GasPriceOrdering::new(self.effective_gas_price, self.nonce())
    }

    fn sender_key(&self) -> Option<SenderKey> {
        SenderKey::new(&self.envelope.sender().0 .0)
    }

    fn gas_limit(&self) -> u64 {
        self.envelope.gas_limit()
    }
}

impl Transaction for TxContext {
    fn sender(&self) -> AccountId {
        AccountId::invalid()
    }

    fn resolve_sender_account(&self, env: &mut dyn Environment) -> SdkResult<AccountId> {
        resolve_or_create_eoa_account(self.sender_address(), env)
    }

    fn recipient(&self) -> AccountId {
        AccountId::invalid()
    }

    fn resolve_recipient_account(&self, env: &mut dyn Environment) -> SdkResult<AccountId> {
        // TODO(vm): contract creation currently has no recipient and is rejected.
        // Once VM deployment is supported, this branch should route to creation
        // logic instead of returning recipient-required.
        let to = self.envelope.to().ok_or(ERR_RECIPIENT_REQUIRED)?;
        if let Some(account_id) = lookup_account_id_in_env(to, env)? {
            return Ok(account_id);
        }
        if let Some(account_id) = lookup_contract_account_id_in_env(to, env)? {
            return Ok(account_id);
        }
        resolve_or_create_eoa_account(to, env)
    }

    fn request(&self) -> &InvokeRequest {
        &self.invoke_request
    }

    fn gas_limit(&self) -> u64 {
        self.envelope.gas_limit()
    }

    fn funds(&self) -> &[FungibleAsset] {
        // TODO: Convert value transfer to FungibleAsset when native token is defined
        &[]
    }

    fn compute_identifier(&self) -> [u8; 32] {
        self.envelope.tx_hash().0
    }

    fn sender_eth_address(&self) -> Option<[u8; 20]> {
        Some(self.sender_address().into())
    }

    fn recipient_eth_address(&self) -> Option<[u8; 20]> {
        self.envelope.to().map(Into::into)
    }
}

impl AuthenticationPayload for TxContext {
    fn authentication_payload(&self) -> SdkResult<Message> {
        let sender: [u8; 20] = self.sender_address().into();
        Message::new(&sender)
    }
}

impl Encodable for TxContext {
    fn encode(&self) -> SdkResult<Vec<u8>> {
        Ok(self.envelope.encode())
    }
}

impl Decodable for TxContext {
    fn decode(bytes: &[u8]) -> SdkResult<Self> {
        let envelope = TxEnvelope::decode(bytes)?;
        // Use base_fee of 0 for decoding - the effective gas price will be
        // recalculated if needed when the transaction is added to a mempool
        TxContext::new(envelope, 0).ok_or(ERR_RECIPIENT_REQUIRED)
    }
}

/// Calculate effective gas price for transaction ordering.
///
/// - Legacy: gas_price
/// - EIP-1559: min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)
fn calculate_effective_gas_price(envelope: &TxEnvelope, base_fee: u128) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => {
            // Legacy transactions have a fixed gas price
            tx.tx().gas_price
        }
        TxEnvelope::Eip1559(tx) => {
            let max_fee = tx.max_fee_per_gas();
            let priority_fee = tx.max_priority_fee_per_gas();
            // Effective = min(max_fee, base_fee + priority_fee)
            max_fee.min(base_fee.saturating_add(priority_fee))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eoa_registry::{lookup_account_id_in_env, register_runtime_contract_account};
    use crate::traits::{derive_eth_eoa_account_id, derive_runtime_contract_account_id};
    use alloy_consensus::{SignableTransaction, TxLegacy};
    use alloy_primitives::{Bytes, PrimitiveSignature, TxKind, U256};
    use evolve_core::runtime_api::{
        RegisterAccountAtIdRequest, RegisterAccountAtIdResponse, ACCOUNT_IDENTIFIER_PREFIX,
        RUNTIME_ACCOUNT_ID,
    };
    use evolve_core::storage_api::{
        StorageGetRequest, StorageGetResponse, StorageSetRequest, StorageSetResponse,
        STORAGE_ACCOUNT_ID,
    };
    use evolve_core::{
        BlockContext, EnvironmentQuery, FungibleAsset, InvokableMessage, InvokeResponse,
        ERR_UNKNOWN_FUNCTION,
    };
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
    use rand::rngs::OsRng;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MockEnv {
        funds: Vec<FungibleAsset>,
        storage: BTreeMap<Vec<u8>, Vec<u8>>,
    }

    impl MockEnv {
        fn account_scoped_key(account_id: AccountId, key: &[u8]) -> Vec<u8> {
            let mut full = account_id.as_bytes().to_vec();
            full.extend_from_slice(key);
            full
        }
    }

    impl EnvironmentQuery for MockEnv {
        fn whoami(&self) -> AccountId {
            RUNTIME_ACCOUNT_ID
        }

        fn sender(&self) -> AccountId {
            AccountId::invalid()
        }

        fn funds(&self) -> &[FungibleAsset] {
            &self.funds
        }

        fn block(&self) -> BlockContext {
            BlockContext::default()
        }

        fn do_query(
            &mut self,
            to: AccountId,
            data: &InvokeRequest,
        ) -> evolve_core::SdkResult<InvokeResponse> {
            if to != STORAGE_ACCOUNT_ID {
                return Err(ERR_UNKNOWN_FUNCTION);
            }
            match data.function() {
                StorageGetRequest::FUNCTION_IDENTIFIER => {
                    let req: StorageGetRequest = data.get()?;
                    let key = Self::account_scoped_key(req.account_id, &req.key);
                    let value = self
                        .storage
                        .get(&key)
                        .cloned()
                        .map(evolve_core::Message::from_bytes);
                    InvokeResponse::new(&StorageGetResponse { value })
                }
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }
    }

    impl Environment for MockEnv {
        fn do_exec(
            &mut self,
            to: AccountId,
            data: &InvokeRequest,
            _funds: Vec<FungibleAsset>,
        ) -> evolve_core::SdkResult<InvokeResponse> {
            if to == STORAGE_ACCOUNT_ID && data.function() == StorageSetRequest::FUNCTION_IDENTIFIER
            {
                let req: StorageSetRequest = data.get()?;
                let key = Self::account_scoped_key(RUNTIME_ACCOUNT_ID, &req.key);
                self.storage.insert(key, req.value.into_bytes()?);
                return InvokeResponse::new(&StorageSetResponse {});
            }

            if to == RUNTIME_ACCOUNT_ID
                && data.function() == RegisterAccountAtIdRequest::FUNCTION_IDENTIFIER
            {
                let req: RegisterAccountAtIdRequest = data.get()?;
                let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
                key.extend_from_slice(&req.account_id.as_bytes());
                self.storage
                    .insert(key, evolve_core::Message::new(&req.code_id)?.into_bytes()?);
                return InvokeResponse::new(&RegisterAccountAtIdResponse {});
            }

            Err(ERR_UNKNOWN_FUNCTION)
        }

        fn emit_event(&mut self, _name: &str, _data: &[u8]) -> evolve_core::SdkResult<()> {
            Ok(())
        }

        fn unique_id(&mut self) -> evolve_core::SdkResult<[u8; 32]> {
            Ok([0u8; 32])
        }
    }

    fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
        let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).unwrap();
        let r = U256::from_be_slice(&sig.r().to_bytes());
        let s = U256::from_be_slice(&sig.s().to_bytes());
        let v = recovery_id.is_y_odd();
        PrimitiveSignature::new(r, s, v)
    }

    fn build_tx_context(to: Address) -> TxContext {
        let signing_key = SigningKey::random(&mut OsRng);
        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(to),
            value: U256::ZERO,
            input: Bytes::new(),
        };
        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.into_signed(signature);
        let mut encoded = Vec::new();
        signed.rlp_encode(&mut encoded);

        let envelope = TxEnvelope::decode(&encoded).expect("decode signed tx");
        TxContext::new(envelope, 0).expect("construct tx context")
    }

    #[test]
    fn resolve_recipient_account_creates_mapping_for_unseen_eoa() {
        let recipient = Address::repeat_byte(0xAB);
        let tx = build_tx_context(recipient);
        let mut env = MockEnv::default();

        let resolved = tx
            .resolve_recipient_account(&mut env)
            .expect("resolve recipient");
        assert_eq!(resolved, derive_eth_eoa_account_id(recipient));

        let mapped = lookup_account_id_in_env(recipient, &mut env)
            .expect("lookup recipient")
            .expect("recipient mapping exists");
        assert_eq!(mapped, resolved);
    }

    #[test]
    fn resolve_recipient_account_prefers_existing_contract_mapping() {
        let mut env = MockEnv::default();
        let contract_id = derive_runtime_contract_account_id(b"mempool-test-contract");
        let contract_address =
            register_runtime_contract_account(contract_id, &mut env).expect("register contract");

        let tx = build_tx_context(contract_address);
        let resolved = tx
            .resolve_recipient_account(&mut env)
            .expect("resolve recipient");
        assert_eq!(resolved, contract_id);
    }
}

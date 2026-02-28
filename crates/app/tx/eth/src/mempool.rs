//! Mempool transaction context for Ethereum and custom sender payloads.
//!
//! # Function Routing
//!
//! Routing for Ethereum EOAs is derived from calldata via
//! `TxEnvelope::to_invoke_requests()`.

use alloy_primitives::{keccak256, Address, B256};
use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{AccountId, Environment, FungibleAsset, InvokeRequest, Message, SdkResult};
use evolve_mempool::{GasPriceOrdering, MempoolTx, SenderKey};
use evolve_stf_traits::{AuthenticationPayload, SenderBootstrap, Transaction};

use crate::envelope::TxEnvelope;
use crate::eoa_registry::{
    ensure_eoa_mapping, lookup_account_id_in_env, lookup_contract_account_id_in_env,
    resolve_or_create_eoa_account, ETH_EOA_CODE_ID,
};
use crate::error::{ERR_RECIPIENT_REQUIRED, ERR_TX_DECODE};
use crate::payload::{EthIntentPayload, TxPayload};
use crate::sender_type;
use crate::traits::{derive_eth_eoa_account_id, TypedTransaction};

const TX_CONTEXT_WIRE_MAGIC: [u8; 4] = *b"ctx1";

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
enum TxPayloadWire {
    Eoa(Vec<u8>),
    Custom(Vec<u8>),
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
struct TxContextWireV1 {
    sender_type: u16,
    payload: TxPayloadWire,
    tx_hash: [u8; 32],
    gas_limit: u64,
    nonce: u64,
    chain_id: Option<u64>,
    effective_gas_price: u128,
    invoke_request: InvokeRequest,
    funds: Vec<FungibleAsset>,
    sender_account: AccountId,
    recipient_account: Option<AccountId>,
    sender_key: Vec<u8>,
    authentication_payload: Message,
    sender_eth_address: Option<[u8; 20]>,
    recipient_eth_address: Option<[u8; 20]>,
}

#[derive(Clone, Debug)]
enum SenderResolution {
    EoaAddress(Address),
    Account(AccountId),
}

#[derive(Clone, Debug)]
enum RecipientResolution {
    EoaAddress(Address),
    Account(AccountId),
    None,
}

/// Metadata required to construct a context from a non-EOA payload.
#[derive(Clone, Debug)]
pub struct TxContextMeta {
    pub tx_hash: B256,
    pub gas_limit: u64,
    pub nonce: u64,
    pub chain_id: Option<u64>,
    pub effective_gas_price: u128,
    pub invoke_request: InvokeRequest,
    pub funds: Vec<FungibleAsset>,
    pub sender_account: AccountId,
    pub recipient_account: Option<AccountId>,
    pub sender_key: Vec<u8>,
    pub authentication_payload: Message,
    pub sender_eth_address: Option<[u8; 20]>,
    pub recipient_eth_address: Option<[u8; 20]>,
}

/// A verified transaction ready for mempool storage.
#[derive(Clone, Debug)]
pub struct TxContext {
    payload: TxPayload,
    sender_type: u16,
    invoke_request: InvokeRequest,
    effective_gas_price: u128,
    tx_hash: B256,
    gas_limit: u64,
    nonce: u64,
    chain_id: Option<u64>,
    sender_resolution: SenderResolution,
    recipient_resolution: RecipientResolution,
    sender_key: Vec<u8>,
    authentication_payload: Message,
    sender_eth_address: Option<[u8; 20]>,
    recipient_eth_address: Option<[u8; 20]>,
    funds: Vec<FungibleAsset>,
}

impl TxContext {
    /// Create a new context from an Ethereum EOA envelope.
    ///
    /// Returns `None` for contract creation transactions (`to == None`).
    pub fn new(envelope: TxEnvelope, base_fee: u128) -> Option<Self> {
        let to = envelope.to()?;
        let sender = envelope.sender();
        let invoke_request = envelope.to_invoke_requests().into_iter().next()?;
        let authentication_payload = Message::new(&sender.into_array()).ok()?;

        Some(Self {
            sender_type: sender_type::EOA_SECP256K1,
            payload: TxPayload::Eoa(Box::new(envelope.clone())),
            invoke_request,
            effective_gas_price: calculate_effective_gas_price(&envelope, base_fee),
            tx_hash: envelope.tx_hash(),
            gas_limit: envelope.gas_limit(),
            nonce: envelope.nonce(),
            chain_id: envelope.chain_id(),
            sender_resolution: SenderResolution::EoaAddress(sender),
            recipient_resolution: RecipientResolution::EoaAddress(to),
            sender_key: sender.as_slice().to_vec(),
            authentication_payload,
            sender_eth_address: Some(sender.into()),
            recipient_eth_address: Some(to.into()),
            funds: Vec::new(),
        })
    }

    /// Create a context from an arbitrary payload and explicit metadata.
    pub fn from_payload(payload: TxPayload, sender_type: u16, meta: TxContextMeta) -> Option<Self> {
        let sender_resolution = if sender_type == sender_type::EOA_SECP256K1 {
            let addr = Address::from(meta.sender_eth_address?);
            SenderResolution::EoaAddress(addr)
        } else {
            SenderResolution::Account(meta.sender_account)
        };

        let recipient_resolution = if let Some(account) = meta.recipient_account {
            RecipientResolution::Account(account)
        } else if let Some(addr) = meta.recipient_eth_address {
            RecipientResolution::EoaAddress(Address::from(addr))
        } else {
            RecipientResolution::None
        };

        let sender_key = if meta.sender_key.is_empty() {
            match sender_resolution {
                SenderResolution::EoaAddress(addr) => addr.as_slice().to_vec(),
                SenderResolution::Account(account) => account.as_bytes().to_vec(),
            }
        } else {
            meta.sender_key
        };

        let _ = SenderKey::new(&sender_key)?;

        Some(Self {
            payload,
            sender_type,
            invoke_request: meta.invoke_request,
            effective_gas_price: meta.effective_gas_price,
            tx_hash: meta.tx_hash,
            gas_limit: meta.gas_limit,
            nonce: meta.nonce,
            chain_id: meta.chain_id,
            sender_resolution,
            recipient_resolution,
            sender_key,
            authentication_payload: meta.authentication_payload,
            sender_eth_address: meta.sender_eth_address,
            recipient_eth_address: meta.recipient_eth_address,
            funds: meta.funds,
        })
    }

    /// Create a custom-sender context that reuses Ethereum transaction fields.
    ///
    /// The `intent` envelope provides Ethereum execution semantics (`to`, calldata,
    /// nonce, gas, and chain id), while authentication is delegated to `sender_type`
    /// via `auth_proof` and `authentication_payload`.
    pub fn from_eth_intent(
        sender_type: u16,
        intent: EthIntentPayload,
        sender_account: AccountId,
        sender_key: Vec<u8>,
        authentication_payload: Message,
        base_fee: u128,
    ) -> Option<Self> {
        let envelope = intent.decode_envelope().ok()?;
        let to = envelope.to()?;
        let invoke_request = envelope.to_invoke_requests().into_iter().next()?;
        let payload_bytes = borsh::to_vec(&intent).ok()?;
        let tx_hash = B256::from(keccak256(&payload_bytes));

        Self::from_payload(
            TxPayload::Custom(payload_bytes),
            sender_type,
            TxContextMeta {
                tx_hash,
                gas_limit: envelope.gas_limit(),
                nonce: envelope.nonce(),
                chain_id: envelope.chain_id(),
                effective_gas_price: calculate_effective_gas_price(&envelope, base_fee),
                invoke_request,
                funds: Vec::new(),
                sender_account,
                recipient_account: None,
                sender_key,
                authentication_payload,
                sender_eth_address: None,
                recipient_eth_address: Some(to.into()),
            },
        )
    }

    /// Returns true when bytes are encoded with the custom `TxContext` wire format.
    pub fn is_wire_encoded(bytes: &[u8]) -> bool {
        bytes.len() >= TX_CONTEXT_WIRE_MAGIC.len()
            && bytes[..TX_CONTEXT_WIRE_MAGIC.len()] == TX_CONTEXT_WIRE_MAGIC
    }

    /// Get the transaction hash.
    pub fn hash(&self) -> B256 {
        self.tx_hash
    }

    /// Get sender type.
    pub fn sender_type(&self) -> u16 {
        self.sender_type
    }

    /// Get sender address for EOA payloads.
    pub fn sender_address(&self) -> Address {
        match self.sender_resolution {
            SenderResolution::EoaAddress(address) => address,
            SenderResolution::Account(_) => {
                panic!("sender_address() is only available for EOA sender types")
            }
        }
    }

    /// Get sender address when available.
    pub fn sender_address_opt(&self) -> Option<Address> {
        match self.sender_resolution {
            SenderResolution::EoaAddress(address) => Some(address),
            SenderResolution::Account(_) => None,
        }
    }

    /// Get nonce.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Get effective gas price.
    pub fn effective_gas_price(&self) -> u128 {
        self.effective_gas_price
    }

    /// Get payload.
    pub fn payload(&self) -> &TxPayload {
        &self.payload
    }

    /// Get envelope for EOA payloads.
    pub fn envelope(&self) -> &TxEnvelope {
        match &self.payload {
            TxPayload::Eoa(envelope) => envelope.as_ref(),
            TxPayload::Custom(_) => panic!("envelope() is only available for EOA payloads"),
        }
    }

    /// Get envelope when available.
    pub fn envelope_opt(&self) -> Option<&TxEnvelope> {
        match &self.payload {
            TxPayload::Eoa(envelope) => Some(envelope.as_ref()),
            TxPayload::Custom(_) => None,
        }
    }

    /// Get chain ID.
    pub fn chain_id(&self) -> Option<u64> {
        self.chain_id
    }
}

impl MempoolTx for TxContext {
    type OrderingKey = GasPriceOrdering;

    fn tx_id(&self) -> [u8; 32] {
        self.tx_hash.0
    }

    fn ordering_key(&self) -> Self::OrderingKey {
        GasPriceOrdering::new(self.effective_gas_price, self.nonce)
    }

    fn sender_key(&self) -> Option<SenderKey> {
        SenderKey::new(&self.sender_key)
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
}

impl Transaction for TxContext {
    fn sender(&self) -> AccountId {
        match self.sender_resolution {
            SenderResolution::Account(account) => account,
            SenderResolution::EoaAddress(address) => derive_eth_eoa_account_id(address),
        }
    }

    fn resolve_sender_account(&self, env: &mut dyn Environment) -> SdkResult<AccountId> {
        match self.sender_resolution {
            SenderResolution::Account(account) => Ok(account),
            SenderResolution::EoaAddress(address) => {
                if let Some(account_id) = lookup_account_id_in_env(address, env)? {
                    return Ok(account_id);
                }
                Ok(derive_eth_eoa_account_id(address))
            }
        }
    }

    fn recipient(&self) -> AccountId {
        match self.recipient_resolution {
            RecipientResolution::Account(account) => account,
            RecipientResolution::EoaAddress(address) => derive_eth_eoa_account_id(address),
            RecipientResolution::None => AccountId::invalid(),
        }
    }

    fn resolve_recipient_account(&self, env: &mut dyn Environment) -> SdkResult<AccountId> {
        match self.recipient_resolution {
            RecipientResolution::Account(account) => Ok(account),
            RecipientResolution::EoaAddress(to) => {
                if let Some(account_id) = lookup_account_id_in_env(to, env)? {
                    return Ok(account_id);
                }
                if let Some(account_id) = lookup_contract_account_id_in_env(to, env)? {
                    return Ok(account_id);
                }
                resolve_or_create_eoa_account(to, env)
            }
            RecipientResolution::None => Err(ERR_RECIPIENT_REQUIRED),
        }
    }

    fn request(&self) -> &InvokeRequest {
        &self.invoke_request
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn funds(&self) -> &[FungibleAsset] {
        &self.funds
    }

    fn compute_identifier(&self) -> [u8; 32] {
        self.tx_hash.0
    }

    fn sender_bootstrap(&self) -> Option<SenderBootstrap> {
        if self.sender_type != sender_type::EOA_SECP256K1 {
            return None;
        }
        let address = self.sender_address_opt()?;
        let init_message = Message::new(&address.into_array()).ok()?;
        Some(SenderBootstrap {
            account_code_id: ETH_EOA_CODE_ID,
            init_message,
        })
    }

    fn after_sender_bootstrap(
        &self,
        resolved_sender: AccountId,
        env: &mut dyn Environment,
    ) -> SdkResult<()> {
        if self.sender_type != sender_type::EOA_SECP256K1 {
            return Ok(());
        }
        let Some(address) = self.sender_address_opt() else {
            return Err(ERR_TX_DECODE);
        };
        ensure_eoa_mapping(address, resolved_sender, env)
    }

    fn sender_eth_address(&self) -> Option<[u8; 20]> {
        self.sender_eth_address
    }

    fn recipient_eth_address(&self) -> Option<[u8; 20]> {
        self.recipient_eth_address
    }
}

impl AuthenticationPayload for TxContext {
    fn authentication_payload(&self) -> SdkResult<Message> {
        Ok(self.authentication_payload.clone())
    }
}

impl Encodable for TxContext {
    fn encode(&self) -> SdkResult<Vec<u8>> {
        if let TxPayload::Eoa(envelope) = &self.payload {
            return Ok(envelope.encode());
        }

        let sender_account = self.sender();
        let recipient_account = match self.recipient_resolution {
            RecipientResolution::Account(account) => Some(account),
            RecipientResolution::EoaAddress(_) | RecipientResolution::None => None,
        };
        let payload = match &self.payload {
            TxPayload::Eoa(envelope) => TxPayloadWire::Eoa(envelope.encode()),
            TxPayload::Custom(bytes) => TxPayloadWire::Custom(bytes.clone()),
        };
        let wire = TxContextWireV1 {
            sender_type: self.sender_type,
            payload,
            tx_hash: self.tx_hash.0,
            gas_limit: self.gas_limit,
            nonce: self.nonce,
            chain_id: self.chain_id,
            effective_gas_price: self.effective_gas_price,
            invoke_request: self.invoke_request.clone(),
            funds: self.funds.clone(),
            sender_account,
            recipient_account,
            sender_key: self.sender_key.clone(),
            authentication_payload: self.authentication_payload.clone(),
            sender_eth_address: self.sender_eth_address,
            recipient_eth_address: self.recipient_eth_address,
        };

        let mut encoded = TX_CONTEXT_WIRE_MAGIC.to_vec();
        encoded.extend(borsh::to_vec(&wire).map_err(|_| ERR_TX_DECODE)?);
        Ok(encoded)
    }
}

impl Decodable for TxContext {
    fn decode(bytes: &[u8]) -> SdkResult<Self> {
        if Self::is_wire_encoded(bytes) {
            let wire: TxContextWireV1 = borsh::from_slice(&bytes[TX_CONTEXT_WIRE_MAGIC.len()..])
                .map_err(|_| ERR_TX_DECODE)?;

            if SenderKey::new(&wire.sender_key).is_none() {
                return Err(ERR_TX_DECODE);
            }

            let payload = match wire.payload {
                TxPayloadWire::Eoa(raw) => TxPayload::Eoa(Box::new(TxEnvelope::decode(&raw)?)),
                TxPayloadWire::Custom(raw) => TxPayload::Custom(raw),
            };

            let sender_resolution = if wire.sender_type == sender_type::EOA_SECP256K1 {
                let address = wire.sender_eth_address.ok_or(ERR_TX_DECODE)?;
                SenderResolution::EoaAddress(Address::from(address))
            } else {
                SenderResolution::Account(wire.sender_account)
            };

            let recipient_resolution = if let Some(account) = wire.recipient_account {
                RecipientResolution::Account(account)
            } else if let Some(address) = wire.recipient_eth_address {
                RecipientResolution::EoaAddress(Address::from(address))
            } else {
                RecipientResolution::None
            };

            return Ok(Self {
                payload,
                sender_type: wire.sender_type,
                invoke_request: wire.invoke_request,
                effective_gas_price: wire.effective_gas_price,
                tx_hash: B256::from(wire.tx_hash),
                gas_limit: wire.gas_limit,
                nonce: wire.nonce,
                chain_id: wire.chain_id,
                sender_resolution,
                recipient_resolution,
                sender_key: wire.sender_key,
                authentication_payload: wire.authentication_payload,
                sender_eth_address: wire.sender_eth_address,
                recipient_eth_address: wire.recipient_eth_address,
                funds: wire.funds,
            });
        }

        let envelope = TxEnvelope::decode(bytes)?;
        TxContext::new(envelope, 0).ok_or(ERR_RECIPIENT_REQUIRED)
    }
}

/// Calculate effective gas price for transaction ordering.
///
/// - Legacy: gas_price
/// - EIP-1559: min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)
fn calculate_effective_gas_price(envelope: &TxEnvelope, base_fee: u128) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => {
            let max_fee = tx.max_fee_per_gas();
            let priority_fee = tx.max_priority_fee_per_gas();
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
        BlockContext, EnvironmentQuery, InvokableMessage, InvokeResponse, ERR_UNKNOWN_FUNCTION,
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

    #[test]
    fn roundtrip_custom_payload_context() {
        let request =
            InvokeRequest::new_from_message("custom.execute", 7, Message::new(&42u64).unwrap());
        let context = TxContext::from_payload(
            TxPayload::Custom(vec![1, 2, 3, 4]),
            sender_type::CUSTOM,
            TxContextMeta {
                tx_hash: B256::from([9u8; 32]),
                gas_limit: 120_000,
                nonce: 11,
                chain_id: None,
                effective_gas_price: 33,
                invoke_request: request,
                funds: vec![],
                sender_account: AccountId::from_u64(77),
                recipient_account: Some(AccountId::from_u64(88)),
                sender_key: vec![0xCC; 32],
                authentication_payload: Message::new(&AccountId::from_u64(77)).unwrap(),
                sender_eth_address: None,
                recipient_eth_address: None,
            },
        )
        .expect("construct custom context");

        let encoded = context.encode().expect("encode custom context");
        let decoded = TxContext::decode(&encoded).expect("decode custom context");
        assert_eq!(decoded.sender_type(), sender_type::CUSTOM);
        assert!(matches!(decoded.payload(), TxPayload::Custom(data) if data == &vec![1, 2, 3, 4]));
        assert_eq!(decoded.nonce(), 11);
        assert_eq!(MempoolTx::gas_limit(&decoded), 120_000);
    }
}

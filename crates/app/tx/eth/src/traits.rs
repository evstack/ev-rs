//! Core traits and canonical identity derivation for typed transactions.

use alloy_primitives::{keccak256, Address, B256};
use evolve_core::{AccountId, InvokeRequest};

const DOMAIN_EOA_ETH_V1: &[u8] = b"eoa:eth:v1";
const DOMAIN_CONTRACT_RUNTIME_V1: &[u8] = b"contract:runtime:v1";
const DOMAIN_SYSTEM_V1: &[u8] = b"system:v1";
const DOMAIN_CONTRACT_ADDR_RUNTIME_V1: &[u8] = b"contract:addr:runtime:v1";

/// Derive a canonical ETH EOA account ID from a 20-byte address.
pub fn derive_eth_eoa_account_id(addr: Address) -> AccountId {
    derive_account_id(DOMAIN_EOA_ETH_V1, addr.as_slice())
}

/// Derive a canonical runtime contract account ID from creation entropy.
pub fn derive_runtime_contract_account_id(creation_entropy: &[u8]) -> AccountId {
    derive_account_id(DOMAIN_CONTRACT_RUNTIME_V1, creation_entropy)
}

/// Derive a canonical system/module account ID from a module name.
pub fn derive_system_account_id(module_name: &str) -> AccountId {
    derive_account_id(DOMAIN_SYSTEM_V1, module_name.as_bytes())
}

/// Deterministic runtime contract address used for explicit registry mappings.
pub fn derive_runtime_contract_address(account_id: AccountId) -> Address {
    let mut input = Vec::with_capacity(DOMAIN_CONTRACT_ADDR_RUNTIME_V1.len() + 32);
    input.extend_from_slice(DOMAIN_CONTRACT_ADDR_RUNTIME_V1);
    input.extend_from_slice(&account_id.as_bytes());
    let digest = keccak256(&input);
    if let Some(address_bytes) = digest.as_slice().get(12..32) {
        Address::from_slice(address_bytes)
    } else {
        Address::ZERO
    }
}

fn derive_account_id(domain_tag: &[u8], payload: &[u8]) -> AccountId {
    let mut input = Vec::with_capacity(domain_tag.len() + payload.len());
    input.extend_from_slice(domain_tag);
    input.extend_from_slice(payload);
    AccountId::from_bytes(keccak256(&input).0)
}

/// Core trait that all transaction types must implement.
pub trait TypedTransaction: Send + Sync {
    fn tx_type(&self) -> u8;
    fn sender(&self) -> Address;
    fn tx_hash(&self) -> B256;
    fn gas_limit(&self) -> u64;
    fn chain_id(&self) -> Option<u64>;
    fn nonce(&self) -> u64;
    fn to(&self) -> Option<Address>;
    fn value(&self) -> alloy_primitives::U256;
    fn input(&self) -> &[u8];
    fn to_invoke_requests(&self) -> Vec<InvokeRequest>;

    fn sender_account_id(&self) -> AccountId {
        derive_eth_eoa_account_id(self.sender())
    }

    fn recipient_account_id(&self) -> Option<AccountId> {
        self.to().map(derive_eth_eoa_account_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eth_eoa_derivation_is_deterministic() {
        let addr = Address::from([0x11; 20]);
        assert_eq!(
            derive_eth_eoa_account_id(addr),
            derive_eth_eoa_account_id(addr)
        );
    }

    #[test]
    fn test_domains_are_separated() {
        let payload = [0x22; 20];
        let eoa = derive_account_id(DOMAIN_EOA_ETH_V1, &payload);
        let contract = derive_account_id(DOMAIN_CONTRACT_RUNTIME_V1, &payload);
        assert_ne!(eoa, contract);
    }

    #[test]
    fn test_system_derivation() {
        let a = derive_system_account_id("runtime");
        let b = derive_system_account_id("storage");
        assert_ne!(a, b);
    }

    #[test]
    fn test_runtime_contract_address_is_deterministic() {
        let id = AccountId::new(0x112233u128);
        let a = derive_runtime_contract_address(id);
        let b = derive_runtime_contract_address(id);
        assert_eq!(a, b);
        assert_ne!(derive_eth_eoa_account_id(a), id);
    }
}

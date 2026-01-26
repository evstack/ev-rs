//! Core traits for typed transactions.

use alloy_primitives::{Address, B256};
use evolve_core::{AccountId, InvokeRequest};

/// Core trait that all transaction types must implement.
///
/// This trait defines the common interface for all transaction types,
/// whether they are standard Ethereum transactions or custom Evolve types.
pub trait TypedTransaction: Send + Sync {
    /// Returns the EIP-2718 transaction type byte.
    ///
    /// - `0x00-0x7f`: Reserved for Ethereum standard types
    /// - `0x80-0xff`: Available for custom/L2 types
    fn tx_type(&self) -> u8;

    /// Returns the sender address.
    ///
    /// For signed transactions, this is recovered from the signature.
    fn sender(&self) -> Address;

    /// Returns the transaction hash.
    fn tx_hash(&self) -> B256;

    /// Returns the gas limit for this transaction.
    fn gas_limit(&self) -> u64;

    /// Returns the chain ID for replay protection (EIP-155).
    ///
    /// Returns `None` for legacy transactions without EIP-155.
    fn chain_id(&self) -> Option<u64>;

    /// Returns the nonce for this transaction.
    fn nonce(&self) -> u64;

    /// Returns the recipient address, if any.
    ///
    /// Returns `None` for contract creation transactions.
    fn to(&self) -> Option<Address>;

    /// Returns the value (in wei) being transferred.
    fn value(&self) -> alloy_primitives::U256;

    /// Returns the input data for this transaction.
    fn input(&self) -> &[u8];

    /// Converts the transaction to internal InvokeRequest(s).
    ///
    /// Standard Ethereum transactions produce a single InvokeRequest.
    /// Batch transactions may produce multiple.
    fn to_invoke_requests(&self) -> Vec<InvokeRequest>;

    /// Converts the sender Address to an AccountId.
    ///
    /// Uses the lower 128 bits of the address for compatibility.
    fn sender_account_id(&self) -> AccountId {
        address_to_account_id(self.sender())
    }

    /// Converts the recipient Address to an AccountId, if present.
    fn recipient_account_id(&self) -> Option<AccountId> {
        self.to().map(address_to_account_id)
    }
}

/// Converts an Ethereum Address (20 bytes) to an AccountId (u128).
///
/// Takes the last 16 bytes of the address. This mapping is reversible:
/// `address_to_account_id(account_id_to_address(id)) == id`
///
/// This allows contract accounts to be addressed via Ethereum transactions.
/// The first 4 bytes of the address are discarded, so addresses that only
/// differ in those bytes will map to the same AccountId.
pub fn address_to_account_id(addr: Address) -> AccountId {
    let bytes = addr.as_slice();
    let mut id_bytes = [0u8; 16];
    id_bytes.copy_from_slice(&bytes[4..]);
    AccountId::new(u128::from_be_bytes(id_bytes))
}

/// Converts an AccountId to an Ethereum Address.
///
/// Pads with zeros in the first 4 bytes. This is the inverse of `address_to_account_id`:
/// `address_to_account_id(account_id_to_address(id)) == id`
///
/// For EOA addresses derived from public keys (which have random first 4 bytes),
/// this won't recover the original address. But for contract addresses that were
/// created from AccountIds, this is a perfect round-trip.
pub fn account_id_to_address(id: AccountId) -> Address {
    let id_bytes = id.as_bytes();
    let mut addr_bytes = [0u8; 20];
    // Copy 16 bytes of AccountId to last 16 bytes of address
    addr_bytes[4..20].copy_from_slice(&id_bytes[..16]);
    Address::from_slice(&addr_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_id_to_address_encodes_id_bytes() {
        let id = AccountId::new(0x112233445566778899aabbccddeeff00);
        let addr = account_id_to_address(id);
        assert_eq!(&addr.as_slice()[4..], &id.as_bytes()[..16]);
    }

    #[test]
    fn test_address_to_account_id_deterministic() {
        let addr1 = Address::repeat_byte(0xAB);
        let addr2 = Address::repeat_byte(0xAB);

        assert_eq!(address_to_account_id(addr1), address_to_account_id(addr2));
    }

    #[test]
    fn test_account_id_address_round_trip() {
        // For any AccountId, converting to address and back should give the same ID
        let id = AccountId::new(0x112233445566778899aabbccddeeff00);
        let addr = account_id_to_address(id);
        let id_back = address_to_account_id(addr);
        assert_eq!(id, id_back);

        // Test with various values
        for i in [0u128, 1, u128::MAX, 0xdeadbeef, 12345678901234567890] {
            let id = AccountId::new(i);
            let addr = account_id_to_address(id);
            let id_back = address_to_account_id(addr);
            assert_eq!(id, id_back, "round-trip failed for id={}", i);
        }
    }

    #[test]
    fn test_address_to_account_id_first_4_bytes_ignored() {
        // Addresses that only differ in the first 4 bytes map to the same AccountId.
        // This is the trade-off for reversibility with account_id_to_address.
        let mut bytes1 = [0u8; 20];
        let mut bytes2 = [0u8; 20];
        bytes1[0] = 0x01;
        bytes2[0] = 0x02;
        let addr1 = Address::from_slice(&bytes1);
        let addr2 = Address::from_slice(&bytes2);

        // These collide - this is expected and documented behavior
        assert_eq!(address_to_account_id(addr1), address_to_account_id(addr2));

        // But addresses differing in the last 16 bytes do NOT collide
        let mut bytes3 = [0u8; 20];
        bytes3[19] = 0x01;
        let addr3 = Address::from_slice(&bytes3);
        assert_ne!(address_to_account_id(addr1), address_to_account_id(addr3));
    }
}

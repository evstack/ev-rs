use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::{AccountId, FungibleAsset, InvokeRequest};
use evolve_server_core::Transaction;
use sha2::Digest;

#[derive(BorshSerialize, BorshDeserialize)]
pub struct Tx {
    pub sender: AccountId,
    pub recipient: AccountId,
    pub request: InvokeRequest,
    pub gas_limit: u64,
    pub funds: Vec<FungibleAsset>,
}

impl Transaction for Tx {
    fn sender(&self) -> AccountId {
        self.sender
    }

    fn recipient(&self) -> AccountId {
        self.recipient
    }

    fn request(&self) -> &InvokeRequest {
        &self.request
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn funds(&self) -> &[FungibleAsset] {
        self.funds.as_slice()
    }

    fn compute_identifier(&self) -> [u8; 32] {
        let h = sha2::Sha256::digest(borsh::to_vec(self).unwrap());
        h.into()
    }
}

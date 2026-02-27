use crate::Simulator;
use evolve_core::{runtime_api::ACCOUNT_IDENTIFIER_PREFIX, AccountId, ErrorCode, Message};
use evolve_stf_traits::StateChange;
use k256::ecdsa::SigningKey;

pub fn generate_signing_key(sim: &mut Simulator, max_attempts: usize) -> Option<SigningKey> {
    for _ in 0..max_attempts {
        let bytes: [u8; 32] = sim.rng().gen();
        if let Ok(key) = SigningKey::from_bytes(&bytes.into()) {
            return Some(key);
        }
    }
    None
}

pub fn register_account_code_identifier(
    sim: &mut Simulator,
    account_id: AccountId,
    code_id: &str,
) -> Result<(), ErrorCode> {
    let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
    key.extend_from_slice(&account_id.as_bytes());
    let value = Message::new(&code_id.to_string())
        .expect("encode code id")
        .into_bytes()
        .expect("code id bytes");

    sim.apply_state_changes(vec![StateChange::Set { key, value }])
}

pub fn init_eth_eoa_storage(
    sim: &mut Simulator,
    account_id: AccountId,
    eth_address: [u8; 20],
) -> Result<(), ErrorCode> {
    let mut nonce_key = account_id.as_bytes().to_vec();
    nonce_key.push(0u8);
    let nonce_value = Message::new(&0u64)
        .expect("encode nonce")
        .into_bytes()
        .expect("nonce bytes");

    let mut addr_key = account_id.as_bytes().to_vec();
    addr_key.push(1u8);
    let addr_value = Message::new(&eth_address)
        .expect("encode eth address")
        .into_bytes()
        .expect("addr bytes");

    sim.apply_state_changes(vec![
        StateChange::Set {
            key: nonce_key,
            value: nonce_value,
        },
        StateChange::Set {
            key: addr_key,
            value: addr_value,
        },
    ])
}

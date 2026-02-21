//! Validator identity and epoch-based validator set management.
//!
//! Each Evolve validator carries two public keys:
//! - An Ed25519 key for P2P identity and network authentication.
//! - A BLS12-381 key for consensus threshold signatures.
//!
//! [`ValidatorSet`] stores validators in a [`BTreeMap`] keyed by their Ed25519
//! P2P key, guaranteeing deterministic iteration order for consensus.

use std::collections::BTreeMap;

use commonware_cryptography::{bls12381, ed25519};

/// A validator's dual identity in the Evolve network.
#[derive(Clone, Debug)]
pub struct ValidatorIdentity {
    /// Ed25519 public key for P2P authentication and network-layer identity.
    pub p2p_key: ed25519::PublicKey,

    /// BLS12-381 public key for consensus threshold signatures.
    pub consensus_key: bls12381::PublicKey,
}

/// The set of validators active during a specific epoch.
///
/// Stored in a [`BTreeMap`] keyed by the Ed25519 P2P key so that iteration
/// order is deterministic — a requirement for consensus correctness.
#[derive(Clone, Debug)]
pub struct ValidatorSet {
    validators: BTreeMap<ed25519::PublicKey, ValidatorIdentity>,
    epoch: u64,
}

impl ValidatorSet {
    /// Create an empty validator set for the given epoch.
    pub fn new(epoch: u64) -> Self {
        Self {
            validators: BTreeMap::new(),
            epoch,
        }
    }

    /// Add a validator to the set.
    ///
    /// If a validator with the same P2P key already exists it is replaced.
    pub fn add_validator(&mut self, identity: ValidatorIdentity) {
        self.validators.insert(identity.p2p_key.clone(), identity);
    }

    /// Return a reference to the underlying validator map.
    pub fn validators(&self) -> &BTreeMap<ed25519::PublicKey, ValidatorIdentity> {
        &self.validators
    }

    /// Number of validators in the set.
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Returns `true` when the set contains no validators.
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// The epoch this set corresponds to.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Ed25519 P2P public keys for all validators, in deterministic order.
    pub fn p2p_keys(&self) -> Vec<ed25519::PublicKey> {
        self.validators.keys().cloned().collect()
    }

    /// BLS12-381 consensus public keys for all validators, in the same
    /// deterministic order as [`p2p_keys`](Self::p2p_keys).
    pub fn consensus_keys(&self) -> Vec<bls12381::PublicKey> {
        self.validators
            .values()
            .map(|v| v.consensus_key.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{bls12381, ed25519, Signer as _};

    fn make_identity(seed: u64) -> ValidatorIdentity {
        let p2p_key = ed25519::PrivateKey::from_seed(seed).public_key();
        let consensus_key = bls12381::PrivateKey::from_seed(seed).public_key();
        ValidatorIdentity {
            p2p_key,
            consensus_key,
        }
    }

    #[test]
    fn new_set_is_empty() {
        let vs = ValidatorSet::new(0);
        assert!(vs.is_empty());
        assert_eq!(vs.len(), 0);
        assert_eq!(vs.epoch(), 0);
    }

    #[test]
    fn add_and_retrieve_validators() {
        let mut vs = ValidatorSet::new(1);
        let id = make_identity(1);
        let p2p_key = id.p2p_key.clone();
        vs.add_validator(id);

        assert_eq!(vs.len(), 1);
        assert!(!vs.is_empty());
        assert!(vs.validators().contains_key(&p2p_key));
    }

    #[test]
    fn duplicate_p2p_key_replaces_entry() {
        let mut vs = ValidatorSet::new(2);
        let id1 = make_identity(10);
        let p2p_key = id1.p2p_key.clone();

        let id2 = ValidatorIdentity {
            p2p_key: p2p_key.clone(),
            consensus_key: bls12381::PrivateKey::from_seed(99).public_key(),
        };

        vs.add_validator(id1);
        vs.add_validator(id2);
        assert_eq!(vs.len(), 1);
    }

    #[test]
    fn p2p_keys_deterministic_order() {
        let mut vs = ValidatorSet::new(3);
        for seed in 0..5u64 {
            vs.add_validator(make_identity(seed));
        }

        let keys = vs.p2p_keys();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "p2p_keys must be in sorted (deterministic) order"
        );
    }

    #[test]
    fn p2p_and_consensus_keys_same_length() {
        let mut vs = ValidatorSet::new(4);
        for seed in 0..3u64 {
            vs.add_validator(make_identity(seed));
        }
        assert_eq!(vs.p2p_keys().len(), vs.consensus_keys().len());
    }
}

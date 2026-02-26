//! Epoch-based peer set provider for the Evolve P2P layer.
//!
//! [`EpochPeerProvider`] implements [`commonware_p2p::Provider`] so that the
//! Commonware authenticated P2P stack can query the current set of validator
//! peers and receive change notifications on epoch transitions.

use std::{collections::BTreeMap, fmt, sync::Arc};

use commonware_cryptography::ed25519;
use commonware_p2p::Provider;
use commonware_utils::ordered::Set;
use tokio::sync::{mpsc, RwLock};

/// Convenience alias for the subscriber notification payload.
///
/// `(epoch_id, new_peer_set, all_tracked_peers)`
type Notification = (u64, Set<ed25519::PublicKey>, Set<ed25519::PublicKey>);
const SUBSCRIBER_CHANNEL_CAPACITY: usize = 64;

#[derive(Debug)]
struct ProviderState {
    /// Peer sets indexed by epoch. BTreeMap for deterministic iteration.
    peer_sets: BTreeMap<u64, Set<ed25519::PublicKey>>,
    /// Active subscriber channels. Dead senders are pruned on each notify.
    subscribers: Vec<mpsc::Sender<Notification>>,
    /// Union of all currently tracked peer sets.
    all_peers: Set<ed25519::PublicKey>,
}

impl ProviderState {
    fn new() -> Self {
        Self {
            peer_sets: BTreeMap::new(),
            subscribers: Vec::new(),
            all_peers: Set::default(),
        }
    }

    fn recompute_all_peers(&mut self) {
        let all_peers: Vec<ed25519::PublicKey> = self
            .peer_sets
            .values()
            .flat_map(|s| s.iter().cloned())
            .collect();
        self.all_peers = Set::from_iter_dedup(all_peers);
    }

    /// Notify live subscribers and prune dead channels.
    fn notify(&mut self, epoch: u64, peers: Set<ed25519::PublicKey>) {
        self.subscribers.retain(|tx| {
            match tx.try_send((epoch, peers.clone(), self.all_peers.clone())) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        epoch,
                        "provider: dropping peer-set notification for slow subscriber"
                    );
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }
}

/// Provides peer sets to the P2P layer based on epochs.
///
/// When the validator set changes (new epoch), call [`update_epoch`] to
/// register the new peer set and notify all current subscribers.
///
/// [`EpochPeerProvider`] is cheaply cloneable; all clones share the same
/// underlying state via an [`Arc`].
///
/// [`update_epoch`]: EpochPeerProvider::update_epoch
#[derive(Clone)]
pub struct EpochPeerProvider {
    inner: Arc<RwLock<ProviderState>>,
}

impl fmt::Debug for EpochPeerProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EpochPeerProvider").finish_non_exhaustive()
    }
}

impl EpochPeerProvider {
    /// Create a new provider with no registered peer sets.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ProviderState::new())),
        }
    }

    /// Register a peer set for the given epoch and notify all subscribers.
    ///
    /// If a peer set for `epoch` already exists it is replaced. Subscribers
    /// receive the new set and the updated union of all tracked peers. Dead
    /// subscriber channels are pruned automatically.
    ///
    /// # Note
    ///
    /// Old epochs are not pruned — `peer_sets` grows with each call. For
    /// production use, call [`retain_epochs`](Self::retain_epochs) periodically
    /// to bound memory usage.
    pub async fn update_epoch(&self, epoch: u64, peers: Set<ed25519::PublicKey>) {
        let mut state = self.inner.write().await;
        state.peer_sets.insert(epoch, peers.clone());
        state.recompute_all_peers();

        state.notify(epoch, peers);
    }

    /// Remove all epoch entries older than `min_epoch`.
    ///
    /// Recomputes `all_peers` from the remaining sets and notifies subscribers
    /// with the highest retained epoch's peer set (if one exists).
    pub async fn retain_epochs(&self, min_epoch: u64) {
        let mut state = self.inner.write().await;
        state.peer_sets.retain(|&e, _| e >= min_epoch);
        state.recompute_all_peers();

        if let Some((epoch, peers)) = state
            .peer_sets
            .iter()
            .next_back()
            .map(|(e, p)| (*e, p.clone()))
        {
            state.notify(epoch, peers);
        }
    }
}

impl Default for EpochPeerProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for EpochPeerProvider {
    type PublicKey = ed25519::PublicKey;

    fn peer_set(
        &mut self,
        id: u64,
    ) -> impl std::future::Future<Output = Option<Set<Self::PublicKey>>> + Send {
        let inner = self.inner.clone();
        async move { inner.read().await.peer_sets.get(&id).cloned() }
    }

    fn subscribe(
        &mut self,
    ) -> impl std::future::Future<
        Output = mpsc::UnboundedReceiver<(u64, Set<Self::PublicKey>, Set<Self::PublicKey>)>,
    > + Send {
        let inner = self.inner.clone();
        async move {
            let (bounded_tx, mut bounded_rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
            let (unbounded_tx, unbounded_rx) = mpsc::unbounded_channel();

            inner.write().await.subscribers.push(bounded_tx);

            tokio::spawn(async move {
                while let Some(notification) = bounded_rx.recv().await {
                    if unbounded_tx.send(notification).is_err() {
                        break;
                    }
                }
            });

            unbounded_rx
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{ed25519, Signer as _};

    fn make_key(seed: u64) -> ed25519::PublicKey {
        ed25519::PrivateKey::from_seed(seed).public_key()
    }

    fn peer_set(seeds: &[u64]) -> Set<ed25519::PublicKey> {
        Set::from_iter_dedup(seeds.iter().map(|&s| make_key(s)))
    }

    fn has_key(set: &Set<ed25519::PublicKey>, key: &ed25519::PublicKey) -> bool {
        set.iter().any(|k| k == key)
    }

    #[tokio::test]
    async fn peer_set_returns_none_before_registration() {
        let mut provider = EpochPeerProvider::new();
        assert!(provider.peer_set(0).await.is_none());
    }

    #[tokio::test]
    async fn peer_set_returns_registered_set() {
        let mut provider = EpochPeerProvider::new();
        let peers = peer_set(&[1, 2, 3]);
        provider.update_epoch(0, peers.clone()).await;

        let result = provider.peer_set(0).await.expect("epoch 0 must exist");
        assert_eq!(result, peers);
    }

    #[tokio::test]
    async fn peer_set_not_found_for_unknown_epoch() {
        let mut provider = EpochPeerProvider::new();
        provider.update_epoch(1, peer_set(&[1])).await;

        assert!(provider.peer_set(0).await.is_none());
        assert!(provider.peer_set(2).await.is_none());
    }

    #[tokio::test]
    async fn subscriber_notified_on_epoch_update() {
        let provider = EpochPeerProvider::new();
        let mut subscriber = provider.clone();
        let mut rx = subscriber.subscribe().await;

        let peers = peer_set(&[10, 20]);
        provider.update_epoch(5, peers.clone()).await;

        let (epoch, new_set, _all) = rx.recv().await.expect("notification expected");
        assert_eq!(epoch, 5);
        assert_eq!(new_set, peers);
    }

    #[tokio::test]
    async fn all_peers_is_union_of_tracked_sets() {
        let provider = EpochPeerProvider::new();
        let mut subscriber = provider.clone();
        let mut rx = subscriber.subscribe().await;

        provider.update_epoch(0, peer_set(&[1, 2])).await;
        let _ = rx.recv().await;

        provider.update_epoch(1, peer_set(&[3, 4])).await;
        let (_, _, all) = rx.recv().await.expect("second notification expected");

        assert_eq!(all.len(), 4, "all_peers must be union of epochs 0 and 1");
    }

    #[tokio::test]
    async fn clones_share_state() {
        let provider = EpochPeerProvider::new();
        let mut clone = provider.clone();

        provider.update_epoch(0, peer_set(&[7, 8])).await;
        let result = clone.peer_set(0).await;
        assert!(result.is_some(), "clone must see updates from original");
    }

    #[tokio::test]
    async fn update_epoch_replaces_existing_epoch_set() {
        let mut provider = EpochPeerProvider::new();
        provider.update_epoch(3, peer_set(&[1, 2])).await;
        provider.update_epoch(3, peer_set(&[2, 9])).await;

        let epoch_set = provider.peer_set(3).await.expect("epoch must exist");
        assert_eq!(epoch_set.len(), 2);
        assert!(has_key(&epoch_set, &make_key(2)));
        assert!(has_key(&epoch_set, &make_key(9)));
        assert!(!has_key(&epoch_set, &make_key(1)));
    }

    #[tokio::test]
    async fn retain_epochs_prunes_old_sets_and_updates_union() {
        let mut provider = EpochPeerProvider::new();
        let mut subscriber = provider.clone();
        let mut rx = subscriber.subscribe().await;

        provider.update_epoch(0, peer_set(&[1, 2])).await;
        let _ = rx.recv().await;
        provider.update_epoch(1, peer_set(&[3])).await;
        let _ = rx.recv().await;
        provider.update_epoch(2, peer_set(&[4])).await;
        let _ = rx.recv().await;

        provider.retain_epochs(1).await;
        let (epoch, retained_set, retained_all) =
            rx.recv().await.expect("retain notification expected");
        assert_eq!(epoch, 2);
        assert_eq!(retained_set, peer_set(&[4]));
        assert_eq!(retained_all.len(), 2);
        assert!(has_key(&retained_all, &make_key(3)));
        assert!(has_key(&retained_all, &make_key(4)));
        assert!(provider.peer_set(0).await.is_none());
        assert!(provider.peer_set(1).await.is_some());
        assert!(provider.peer_set(2).await.is_some());

        provider.update_epoch(3, peer_set(&[5])).await;
        let (_, _, all) = rx.recv().await.expect("notification expected");
        assert_eq!(all.len(), 3);
        assert!(has_key(&all, &make_key(3)));
        assert!(has_key(&all, &make_key(4)));
        assert!(has_key(&all, &make_key(5)));
        assert!(!has_key(&all, &make_key(1)));
        assert!(!has_key(&all, &make_key(2)));
    }

    #[tokio::test]
    async fn retain_epochs_pruning_highest_epoch_notifies_new_highest() {
        let mut provider = EpochPeerProvider::new();
        let mut subscriber = provider.clone();
        let mut rx = subscriber.subscribe().await;

        provider.update_epoch(1, peer_set(&[1])).await;
        let _ = rx.recv().await;
        provider.update_epoch(5, peer_set(&[5])).await;
        let _ = rx.recv().await;

        provider.retain_epochs(0).await;
        let _ = rx.recv().await;

        provider.retain_epochs(6).await;
        assert!(provider.peer_set(1).await.is_none());
        assert!(provider.peer_set(5).await.is_none());

        provider.update_epoch(6, peer_set(&[6])).await;
        let (epoch, new_set, all) = rx.recv().await.expect("notification expected");
        assert_eq!(epoch, 6);
        assert_eq!(new_set, peer_set(&[6]));
        assert_eq!(all.len(), 1);
        assert!(has_key(&all, &make_key(6)));
    }

    #[tokio::test]
    async fn dropped_subscriber_is_pruned_on_notify() {
        let provider = EpochPeerProvider::new();
        let mut sub_a = provider.clone();
        let mut sub_b = provider.clone();
        let _rx_a = sub_a.subscribe().await;
        let rx_b = sub_b.subscribe().await;

        assert_eq!(provider.inner.read().await.subscribers.len(), 2);
        drop(rx_b);

        provider.update_epoch(9, peer_set(&[42])).await;
        tokio::task::yield_now().await;
        provider.update_epoch(10, peer_set(&[43])).await;
        assert_eq!(
            provider.inner.read().await.subscribers.len(),
            1,
            "dead subscriber should be removed after notify"
        );
    }
}

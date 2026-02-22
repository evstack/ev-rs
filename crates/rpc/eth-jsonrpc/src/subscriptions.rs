//! WebSocket subscription management for eth_subscribe.
//!
//! This module provides the infrastructure for managing real-time subscriptions
//! to blockchain events like new blocks, logs, and pending transactions.

use alloy_primitives::{Address, B256, U64};
use evolve_rpc_types::{RpcBlock, RpcLog};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Subscription types supported by eth_subscribe.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubscriptionKind {
    /// Subscribe to new block headers.
    NewHeads,
    /// Subscribe to logs matching a filter.
    Logs,
    /// Subscribe to new pending transaction hashes.
    NewPendingTransactions,
    /// Subscribe to sync status changes.
    Syncing,
}

/// Parameters for log subscriptions.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogSubscriptionParams {
    /// Filter by contract address(es).
    #[serde(default)]
    pub address: Option<LogFilterAddress>,
    /// Filter by topic(s).
    #[serde(default)]
    pub topics: Option<Vec<Option<LogFilterTopic>>>,
}

/// Address filter for log subscriptions.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LogFilterAddress {
    Single(Address),
    Multiple(Vec<Address>),
}

/// Topic filter for log subscriptions.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LogFilterTopic {
    Single(B256),
    Multiple(Vec<B256>),
}

/// Events that can be published to subscribers.
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    /// New block header.
    NewHead(Box<RpcBlock>),
    /// New log event.
    Log(Box<RpcLog>),
    /// New pending transaction hash.
    PendingTransaction(B256),
    /// Sync status changed.
    SyncStatus(SyncStatusEvent),
}

/// Sync status event payload.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SyncStatusEvent {
    Syncing {
        #[serde(rename = "startingBlock")]
        starting_block: U64,
        #[serde(rename = "currentBlock")]
        current_block: U64,
        #[serde(rename = "highestBlock")]
        highest_block: U64,
    },
    NotSyncing(bool),
}

/// A subscription with its filter criteria.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: U64,
    pub kind: SubscriptionKind,
    pub params: Option<LogSubscriptionParams>,
}

/// Result payload sent to subscribers.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionResult {
    pub subscription: U64,
    pub result: serde_json::Value,
}

/// Manages active subscriptions and event distribution.
pub struct SubscriptionManager {
    /// Counter for generating unique subscription IDs.
    next_id: AtomicU64,
    /// Active subscriptions by ID.
    subscriptions: RwLock<BTreeMap<u64, Subscription>>,
    /// Broadcast channel for new block headers.
    new_heads_tx: broadcast::Sender<Box<RpcBlock>>,
    /// Broadcast channel for logs.
    logs_tx: broadcast::Sender<Box<RpcLog>>,
    /// Broadcast channel for pending transactions.
    pending_tx_tx: broadcast::Sender<B256>,
    /// Broadcast channel for sync status.
    sync_tx: broadcast::Sender<SyncStatusEvent>,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionManager {
    /// Create a new subscription manager.
    pub fn new() -> Self {
        // Use reasonable channel capacities - subscribers that fall behind will miss messages
        let (new_heads_tx, _) = broadcast::channel(64);
        let (logs_tx, _) = broadcast::channel(256);
        let (pending_tx_tx, _) = broadcast::channel(1024);
        let (sync_tx, _) = broadcast::channel(16);

        Self {
            next_id: AtomicU64::new(1),
            subscriptions: RwLock::new(BTreeMap::new()),
            new_heads_tx,
            logs_tx,
            pending_tx_tx,
            sync_tx,
        }
    }

    /// Create a new subscription and return its ID.
    pub fn subscribe(&self, kind: SubscriptionKind, params: Option<LogSubscriptionParams>) -> U64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let subscription = Subscription {
            id: U64::from(id),
            kind,
            params,
        };

        self.subscriptions.write().insert(id, subscription);
        U64::from(id)
    }

    /// Remove a subscription by ID. Returns true if it existed.
    pub fn unsubscribe(&self, id: U64) -> bool {
        self.subscriptions.write().remove(&id.to::<u64>()).is_some()
    }

    /// Get a subscription by ID.
    pub fn get(&self, id: U64) -> Option<Subscription> {
        self.subscriptions.read().get(&id.to::<u64>()).cloned()
    }

    /// Get the number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.read().len()
    }

    /// Subscribe to new block headers.
    pub fn subscribe_new_heads(&self) -> broadcast::Receiver<Box<RpcBlock>> {
        self.new_heads_tx.subscribe()
    }

    /// Subscribe to logs.
    pub fn subscribe_logs(&self) -> broadcast::Receiver<Box<RpcLog>> {
        self.logs_tx.subscribe()
    }

    /// Subscribe to pending transactions.
    pub fn subscribe_pending_transactions(&self) -> broadcast::Receiver<B256> {
        self.pending_tx_tx.subscribe()
    }

    /// Subscribe to sync status.
    pub fn subscribe_sync(&self) -> broadcast::Receiver<SyncStatusEvent> {
        self.sync_tx.subscribe()
    }

    /// Publish a new block header to subscribers.
    pub fn publish_new_head(&self, block: RpcBlock) {
        // Ignore send errors - they just mean no active subscribers
        let _ = self.new_heads_tx.send(Box::new(block));
    }

    /// Publish a log to subscribers.
    pub fn publish_log(&self, log: RpcLog) {
        let _ = self.logs_tx.send(Box::new(log));
    }

    /// Publish a pending transaction hash to subscribers.
    pub fn publish_pending_transaction(&self, hash: B256) {
        let _ = self.pending_tx_tx.send(hash);
    }

    /// Publish sync status to subscribers.
    pub fn publish_sync_status(&self, status: SyncStatusEvent) {
        let _ = self.sync_tx.send(status);
    }

    /// Check if a log matches a subscription's filter.
    pub fn log_matches_filter(log: &RpcLog, params: &Option<LogSubscriptionParams>) -> bool {
        let Some(params) = params else {
            return true; // No filter means match all
        };

        // Check address filter
        if let Some(ref addr_filter) = params.address {
            let matches = match addr_filter {
                LogFilterAddress::Single(addr) => log.address == *addr,
                LogFilterAddress::Multiple(addrs) => addrs.contains(&log.address),
            };
            if !matches {
                return false;
            }
        }

        // Check topic filters
        if let Some(ref topic_filters) = params.topics {
            for (i, filter) in topic_filters.iter().enumerate() {
                if let Some(filter) = filter {
                    let log_topic = log.topics.get(i);
                    let matches = match filter {
                        LogFilterTopic::Single(t) => log_topic == Some(t),
                        LogFilterTopic::Multiple(ts) => log_topic.is_some_and(|lt| ts.contains(lt)),
                    };
                    if !matches {
                        return false;
                    }
                }
            }
        }

        true
    }
}

/// Thread-safe reference to a subscription manager.
pub type SharedSubscriptionManager = Arc<SubscriptionManager>;

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;

    #[test]
    fn test_subscribe_unsubscribe() {
        let manager = SubscriptionManager::new();

        let id1 = manager.subscribe(SubscriptionKind::NewHeads, None);
        let id2 = manager.subscribe(SubscriptionKind::Logs, None);

        assert_eq!(manager.subscription_count(), 2);
        assert!(manager.get(id1).is_some());
        assert!(manager.get(id2).is_some());

        assert!(manager.unsubscribe(id1));
        assert_eq!(manager.subscription_count(), 1);
        assert!(manager.get(id1).is_none());
        assert!(manager.get(id2).is_some());

        // Double unsubscribe returns false
        assert!(!manager.unsubscribe(id1));
    }

    #[test]
    fn test_log_filter_address() {
        let log = RpcLog {
            address: Address::repeat_byte(0x01),
            topics: vec![],
            data: Bytes::new(),
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        };

        // No filter matches all
        assert!(SubscriptionManager::log_matches_filter(&log, &None));

        // Single address match
        let params = LogSubscriptionParams {
            address: Some(LogFilterAddress::Single(Address::repeat_byte(0x01))),
            topics: None,
        };
        assert!(SubscriptionManager::log_matches_filter(&log, &Some(params)));

        // Single address no match
        let params = LogSubscriptionParams {
            address: Some(LogFilterAddress::Single(Address::repeat_byte(0x02))),
            topics: None,
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &log,
            &Some(params)
        ));

        // Multiple addresses match
        let params = LogSubscriptionParams {
            address: Some(LogFilterAddress::Multiple(vec![
                Address::repeat_byte(0x01),
                Address::repeat_byte(0x02),
            ])),
            topics: None,
        };
        assert!(SubscriptionManager::log_matches_filter(&log, &Some(params)));
    }

    #[test]
    fn test_log_filter_topics() {
        let log = RpcLog {
            address: Address::ZERO,
            topics: vec![B256::repeat_byte(0x01), B256::repeat_byte(0x02)],
            data: Bytes::new(),
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        };

        // Single topic match
        let params = LogSubscriptionParams {
            address: None,
            topics: Some(vec![Some(LogFilterTopic::Single(B256::repeat_byte(0x01)))]),
        };
        assert!(SubscriptionManager::log_matches_filter(&log, &Some(params)));

        // Single topic no match
        let params = LogSubscriptionParams {
            address: None,
            topics: Some(vec![Some(LogFilterTopic::Single(B256::repeat_byte(0xff)))]),
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &log,
            &Some(params)
        ));

        // Wildcard first topic, match second
        let params = LogSubscriptionParams {
            address: None,
            topics: Some(vec![
                None,
                Some(LogFilterTopic::Single(B256::repeat_byte(0x02))),
            ]),
        };
        assert!(SubscriptionManager::log_matches_filter(&log, &Some(params)));

        // Multiple topics (OR semantics) matches
        let params = LogSubscriptionParams {
            address: None,
            topics: Some(vec![Some(LogFilterTopic::Multiple(vec![
                B256::repeat_byte(0xff),
                B256::repeat_byte(0x01),
            ]))]),
        };
        assert!(SubscriptionManager::log_matches_filter(&log, &Some(params)));

        // Topic index out of bounds should fail when a filter is provided
        let params = LogSubscriptionParams {
            address: None,
            topics: Some(vec![
                None,
                None,
                Some(LogFilterTopic::Single(B256::repeat_byte(0x03))),
            ]),
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &log,
            &Some(params)
        ));
    }

    #[tokio::test]
    async fn test_subscribe_logs_receives_published_log() {
        let manager = SubscriptionManager::new();
        let mut rx = manager.subscribe_logs();
        let log = RpcLog {
            address: Address::repeat_byte(0x01),
            topics: vec![B256::repeat_byte(0x02)],
            data: Bytes::from_static(&[0xAA, 0xBB]),
            block_number: Some(U64::from(10)),
            transaction_hash: Some(B256::repeat_byte(0x03)),
            transaction_index: Some(U64::from(1)),
            block_hash: Some(B256::repeat_byte(0x04)),
            log_index: Some(U64::from(0)),
            removed: false,
        };

        manager.publish_log(log.clone());
        let received = rx.recv().await.expect("log broadcast must be received");
        assert_eq!(received.address, log.address);
        assert_eq!(received.topics, log.topics);
        assert_eq!(received.data, log.data);
        assert_eq!(received.block_number, log.block_number);
        assert_eq!(received.transaction_hash, log.transaction_hash);
        assert_eq!(received.transaction_index, log.transaction_index);
        assert_eq!(received.block_hash, log.block_hash);
        assert_eq!(received.log_index, log.log_index);
        assert_eq!(received.removed, log.removed);
    }

    #[test]
    fn test_log_filter_combines_address_and_topics_semantics() {
        let manager = SubscriptionManager::new();
        let params = LogSubscriptionParams {
            address: Some(LogFilterAddress::Multiple(vec![
                Address::repeat_byte(0xAA),
                Address::repeat_byte(0xBB),
            ])),
            topics: Some(vec![
                Some(LogFilterTopic::Single(B256::repeat_byte(0x01))),
                None,
                Some(LogFilterTopic::Multiple(vec![
                    B256::repeat_byte(0x03),
                    B256::repeat_byte(0x04),
                ])),
            ]),
        };
        let sub_id = manager.subscribe(SubscriptionKind::Logs, Some(params));
        let stored = manager.get(sub_id).expect("subscription exists");

        let matching_log = RpcLog {
            address: Address::repeat_byte(0xAA),
            topics: vec![
                B256::repeat_byte(0x01),
                B256::repeat_byte(0xFF),
                B256::repeat_byte(0x04),
            ],
            data: Bytes::new(),
            block_number: Some(U64::from(1)),
            transaction_hash: Some(B256::repeat_byte(0x11)),
            transaction_index: Some(U64::from(0)),
            block_hash: Some(B256::repeat_byte(0x12)),
            log_index: Some(U64::from(0)),
            removed: false,
        };
        assert!(SubscriptionManager::log_matches_filter(
            &matching_log,
            &stored.params
        ));

        let wrong_address_log = RpcLog {
            address: Address::repeat_byte(0xCC),
            ..matching_log.clone()
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &wrong_address_log,
            &stored.params
        ));

        let wrong_topic_log = RpcLog {
            topics: vec![
                B256::repeat_byte(0x01),
                B256::repeat_byte(0xFF),
                B256::repeat_byte(0x05),
            ],
            ..matching_log
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &wrong_topic_log,
            &stored.params
        ));
    }

    #[test]
    fn test_log_filter_empty_alternatives_match_nothing() {
        let log = RpcLog {
            address: Address::repeat_byte(0x01),
            topics: vec![B256::repeat_byte(0x02)],
            data: Bytes::new(),
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        };

        let empty_address_filter = LogSubscriptionParams {
            address: Some(LogFilterAddress::Multiple(vec![])),
            topics: None,
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &log,
            &Some(empty_address_filter)
        ));

        let empty_topic_alternatives = LogSubscriptionParams {
            address: None,
            topics: Some(vec![Some(LogFilterTopic::Multiple(vec![]))]),
        };
        assert!(!SubscriptionManager::log_matches_filter(
            &log,
            &Some(empty_topic_alternatives)
        ));
    }
}

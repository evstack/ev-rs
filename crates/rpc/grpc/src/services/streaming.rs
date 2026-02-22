//! StreamingService implementation providing server-streaming RPC methods.

use std::pin::Pin;

use alloy_primitives::{Address, B256};
use async_trait::async_trait;
use evolve_eth_jsonrpc::SharedSubscriptionManager;
use evolve_rpc_types::SyncStatus as RpcSyncStatus;
use futures::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::conversion::{
    b256_to_proto, proto_to_address, proto_to_b256, rpc_block_to_header, rpc_log_to_proto,
    rpc_sync_status_to_proto,
};
use crate::proto::evolve::v1::{self as proto, streaming_service_server::StreamingService};

/// StreamingService implementation that wraps a SubscriptionManager.
pub struct StreamingServiceImpl {
    subscriptions: SharedSubscriptionManager,
}

impl StreamingServiceImpl {
    /// Create a new StreamingService with the given subscription manager.
    pub fn new(subscriptions: SharedSubscriptionManager) -> Self {
        Self { subscriptions }
    }
}

type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

#[async_trait]
impl StreamingService for StreamingServiceImpl {
    type SubscribeNewHeadsStream = ResponseStream<proto::BlockHeader>;
    type SubscribeLogsStream = ResponseStream<proto::Log>;
    type SubscribePendingTransactionsStream = ResponseStream<proto::H256>;
    type SubscribeSyncingStream = ResponseStream<proto::SyncStatus>;

    async fn subscribe_new_heads(
        &self,
        _request: Request<proto::SubscribeNewHeadsRequest>,
    ) -> Result<Response<Self::SubscribeNewHeadsStream>, Status> {
        let rx = self.subscriptions.subscribe_new_heads();

        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(block) => Some(Ok(rpc_block_to_header(&block))),
            Err(err) => {
                tracing::warn!("gRPC newHeads subscriber error: {}", err);
                None // Skip errors (lag or closed)
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn subscribe_logs(
        &self,
        request: Request<proto::SubscribeLogsRequest>,
    ) -> Result<Response<Self::SubscribeLogsStream>, Status> {
        let req = request.into_inner();

        // Parse address filter
        let addresses: Vec<Address> = req.addresses.iter().filter_map(proto_to_address).collect();

        // Parse topic filters
        let topics: Vec<Vec<B256>> = req
            .topics
            .iter()
            .map(|tf| tf.topics.iter().filter_map(proto_to_b256).collect())
            .collect();

        let rx = self.subscriptions.subscribe_logs();

        let stream = BroadcastStream::new(rx).filter_map(move |result| {
            match result {
                Ok(log) => {
                    // Apply address filter
                    if !addresses.is_empty() && !addresses.contains(&log.address) {
                        return None;
                    }

                    // Apply topic filters
                    for (i, filter_topics) in topics.iter().enumerate() {
                        if filter_topics.is_empty() {
                            continue; // Wildcard at this position
                        }
                        match log.topics.get(i) {
                            Some(log_topic) if filter_topics.contains(log_topic) => {}
                            _ => return None, // No match
                        }
                    }

                    Some(Ok(rpc_log_to_proto(&log)))
                }
                Err(err) => {
                    tracing::warn!("gRPC logs subscriber error: {}", err);
                    None
                }
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn subscribe_pending_transactions(
        &self,
        _request: Request<proto::SubscribePendingTransactionsRequest>,
    ) -> Result<Response<Self::SubscribePendingTransactionsStream>, Status> {
        let rx = self.subscriptions.subscribe_pending_transactions();

        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(hash) => Some(Ok(b256_to_proto(hash))),
            Err(err) => {
                tracing::warn!("gRPC pendingTx subscriber error: {}", err);
                None
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn subscribe_syncing(
        &self,
        _request: Request<proto::SubscribeSyncingRequest>,
    ) -> Result<Response<Self::SubscribeSyncingStream>, Status> {
        let rx = self.subscriptions.subscribe_sync();

        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(status) => {
                // Convert subscription event to RpcSyncStatus
                let rpc_status = match status {
                    evolve_eth_jsonrpc::subscriptions::SyncStatusEvent::Syncing {
                        starting_block,
                        current_block,
                        highest_block,
                    } => RpcSyncStatus::Syncing(evolve_rpc_types::SyncProgress {
                        starting_block,
                        current_block,
                        highest_block,
                    }),
                    evolve_eth_jsonrpc::subscriptions::SyncStatusEvent::NotSyncing(_) => {
                        RpcSyncStatus::NotSyncing(false)
                    }
                };
                Some(Ok(rpc_sync_status_to_proto(&rpc_status)))
            }
            Err(err) => {
                tracing::warn!("gRPC syncing subscriber error: {}", err);
                None
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U64};
    use evolve_eth_jsonrpc::SubscriptionManager;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn test_subscribe_logs_applies_address_and_topic_filters() {
        use crate::proto::evolve::v1::streaming_service_server::StreamingService;

        let subscriptions = std::sync::Arc::new(SubscriptionManager::new());
        let service = StreamingServiceImpl::new(subscriptions.clone());

        let req = proto::SubscribeLogsRequest {
            addresses: vec![proto::Address {
                data: vec![0x11; 20],
            }],
            topics: vec![proto::TopicFilter {
                topics: vec![proto::H256 {
                    data: vec![0x22; 32],
                }],
            }],
        };

        let response = service
            .subscribe_logs(Request::new(req))
            .await
            .expect("subscribe_logs should succeed");
        let mut stream = response.into_inner();

        // Filtered out by address.
        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x33),
            topics: vec![B256::repeat_byte(0x22)],
            data: Bytes::new(),
            block_number: Some(U64::from(1)),
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        });

        // Filtered out by topic.
        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x11),
            topics: vec![B256::repeat_byte(0x44)],
            data: Bytes::new(),
            block_number: Some(U64::from(2)),
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        });

        // Matching event should be delivered.
        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x11),
            topics: vec![B256::repeat_byte(0x22)],
            data: Bytes::from_static(&[0xAB]),
            block_number: Some(U64::from(3)),
            transaction_hash: Some(B256::repeat_byte(0x55)),
            transaction_index: Some(U64::from(1)),
            block_hash: Some(B256::repeat_byte(0x66)),
            log_index: Some(U64::from(0)),
            removed: false,
        });

        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("stream should yield")
            .expect("stream should not terminate")
            .expect("stream item should be ok");

        assert_eq!(item.address.unwrap().data, vec![0x11; 20]);
        assert_eq!(item.topics.len(), 1);
        assert_eq!(item.topics[0].data, vec![0x22; 32]);
        assert_eq!(item.data, vec![0xAB]);
    }

    #[tokio::test]
    async fn test_subscribe_logs_ignores_invalid_filter_entries() {
        use crate::proto::evolve::v1::streaming_service_server::StreamingService;

        let subscriptions = std::sync::Arc::new(SubscriptionManager::new());
        let service = StreamingServiceImpl::new(subscriptions.clone());

        let req = proto::SubscribeLogsRequest {
            addresses: vec![proto::Address {
                data: vec![0x11; 3],
            }],
            topics: vec![proto::TopicFilter {
                topics: vec![proto::H256 {
                    data: vec![0x22; 5],
                }],
            }],
        };

        let response = service
            .subscribe_logs(Request::new(req))
            .await
            .expect("subscribe_logs should succeed");
        let mut stream = response.into_inner();

        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x44),
            topics: vec![B256::repeat_byte(0x55)],
            data: Bytes::from_static(&[0xCD]),
            block_number: Some(U64::from(8)),
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        });

        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("stream should yield")
            .expect("stream should not terminate")
            .expect("stream item should be ok");

        assert_eq!(item.address.expect("address").data, vec![0x44; 20]);
        assert_eq!(item.topics[0].data, vec![0x55; 32]);
        assert_eq!(item.data, vec![0xCD]);
    }

    #[tokio::test]
    async fn test_subscribe_logs_topic_position_wildcard_and_missing_topic_behavior() {
        use crate::proto::evolve::v1::streaming_service_server::StreamingService;

        let subscriptions = std::sync::Arc::new(SubscriptionManager::new());
        let service = StreamingServiceImpl::new(subscriptions.clone());

        let req = proto::SubscribeLogsRequest {
            addresses: Vec::new(),
            topics: vec![
                proto::TopicFilter { topics: Vec::new() },
                proto::TopicFilter {
                    topics: vec![proto::H256 {
                        data: vec![0xBB; 32],
                    }],
                },
            ],
        };

        let response = service
            .subscribe_logs(Request::new(req))
            .await
            .expect("subscribe_logs should succeed");
        let mut stream = response.into_inner();

        // Rejected: topic[1] required by filter but log has only one topic.
        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x11),
            topics: vec![B256::repeat_byte(0xAA)],
            data: Bytes::new(),
            block_number: Some(U64::from(1)),
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        });

        // Accepted: topic[0] wildcard, topic[1] matches.
        subscriptions.publish_log(evolve_rpc_types::RpcLog {
            address: Address::repeat_byte(0x11),
            topics: vec![B256::repeat_byte(0xAA), B256::repeat_byte(0xBB)],
            data: Bytes::from_static(&[0xEF]),
            block_number: Some(U64::from(2)),
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        });

        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("stream should yield")
            .expect("stream should not terminate")
            .expect("stream item should be ok");

        assert_eq!(item.topics.len(), 2);
        assert_eq!(item.topics[0].data, vec![0xAA; 32]);
        assert_eq!(item.topics[1].data, vec![0xBB; 32]);
        assert_eq!(item.data, vec![0xEF]);
    }

    #[tokio::test]
    async fn test_subscribe_pending_transactions_streams_hashes() {
        use crate::proto::evolve::v1::streaming_service_server::StreamingService;

        let subscriptions = std::sync::Arc::new(SubscriptionManager::new());
        let service = StreamingServiceImpl::new(subscriptions.clone());

        let response = service
            .subscribe_pending_transactions(Request::new(
                proto::SubscribePendingTransactionsRequest {},
            ))
            .await
            .expect("subscribe_pending_transactions should succeed");
        let mut stream = response.into_inner();

        subscriptions.publish_pending_transaction(B256::repeat_byte(0x77));

        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("stream should yield")
            .expect("stream should not terminate")
            .expect("stream item should be ok");

        assert_eq!(item.data, vec![0x77; 32]);
    }
}

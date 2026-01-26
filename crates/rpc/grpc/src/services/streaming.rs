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

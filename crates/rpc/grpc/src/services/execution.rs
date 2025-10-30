//! ExecutionService implementation providing unary RPC methods.

use std::sync::Arc;

use async_trait::async_trait;
use evolve_eth_jsonrpc::{RpcError, StateProvider};
use evolve_rpc_types::BlockNumberOrTag;
use tonic::{Request, Response, Status};

use crate::conversion::{
    account_schema_to_proto, b256_to_proto, proto_to_address, proto_to_b256, proto_to_block_id,
    proto_to_call_request, proto_to_log_filter, proto_to_u256, rpc_block_to_proto,
    rpc_log_to_proto, rpc_receipt_to_proto, rpc_sync_status_to_proto, rpc_transaction_to_proto,
    u256_to_proto,
};
use crate::error::GrpcError;
use crate::proto::evolve::v1::{self as proto, execution_service_server::ExecutionService};

/// ExecutionService implementation that wraps a StateProvider.
pub struct ExecutionServiceImpl<S: StateProvider> {
    chain_id: u64,
    state: Arc<S>,
}

impl<S: StateProvider> ExecutionServiceImpl<S> {
    /// Create a new ExecutionService with the given chain ID and state provider.
    pub fn new(chain_id: u64, state: Arc<S>) -> Self {
        Self { chain_id, state }
    }

    /// Resolve a BlockId to an actual block number.
    async fn resolve_block(&self, block: Option<&proto::BlockId>) -> Result<Option<u64>, RpcError> {
        match block.and_then(proto_to_block_id) {
            None => Ok(None),
            Some(BlockNumberOrTag::Number(n)) => Ok(Some(n.to::<u64>())),
            Some(BlockNumberOrTag::Tag(tag)) => {
                use evolve_rpc_types::BlockTag;
                match tag {
                    BlockTag::Latest | BlockTag::Safe | BlockTag::Finalized | BlockTag::Pending => {
                        Ok(Some(self.state.block_number().await?))
                    }
                    BlockTag::Earliest => Ok(Some(0)),
                }
            }
        }
    }
}

#[async_trait]
impl<S: StateProvider> ExecutionService for ExecutionServiceImpl<S> {
    async fn get_chain_id(
        &self,
        _request: Request<proto::GetChainIdRequest>,
    ) -> Result<Response<proto::GetChainIdResponse>, Status> {
        Ok(Response::new(proto::GetChainIdResponse {
            chain_id: self.chain_id,
        }))
    }

    async fn get_block_number(
        &self,
        _request: Request<proto::GetBlockNumberRequest>,
    ) -> Result<Response<proto::GetBlockNumberResponse>, Status> {
        let number = self.state.block_number().await.map_err(GrpcError::from)?;
        Ok(Response::new(proto::GetBlockNumberResponse {
            block_number: number,
        }))
    }

    async fn get_syncing(
        &self,
        _request: Request<proto::GetSyncingRequest>,
    ) -> Result<Response<proto::GetSyncingResponse>, Status> {
        // For now, always return not syncing (same as JSON-RPC)
        let status = evolve_rpc_types::SyncStatus::NotSyncing(false);
        Ok(Response::new(proto::GetSyncingResponse {
            status: Some(rpc_sync_status_to_proto(&status)),
        }))
    }

    async fn get_block_by_number(
        &self,
        request: Request<proto::GetBlockByNumberRequest>,
    ) -> Result<Response<proto::GetBlockByNumberResponse>, Status> {
        let req = request.into_inner();
        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let block = match block_num {
            Some(n) => self
                .state
                .get_block_by_number(n, req.full_transactions)
                .await
                .map_err(GrpcError::from)?,
            None => None,
        };

        Ok(Response::new(proto::GetBlockByNumberResponse {
            block: block.map(|b| rpc_block_to_proto(&b, req.full_transactions)),
        }))
    }

    async fn get_block_by_hash(
        &self,
        request: Request<proto::GetBlockByHashRequest>,
    ) -> Result<Response<proto::GetBlockByHashResponse>, Status> {
        let req = request.into_inner();
        let hash = req
            .hash
            .as_ref()
            .and_then(proto_to_b256)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid block hash".to_string()))?;

        let block = self
            .state
            .get_block_by_hash(hash, req.full_transactions)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetBlockByHashResponse {
            block: block.map(|b| rpc_block_to_proto(&b, req.full_transactions)),
        }))
    }

    async fn get_block_transaction_count_by_number(
        &self,
        request: Request<proto::GetBlockTransactionCountByNumberRequest>,
    ) -> Result<Response<proto::GetBlockTransactionCountResponse>, Status> {
        let req = request.into_inner();
        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let count = match block_num {
            Some(n) => {
                let block = self
                    .state
                    .get_block_by_number(n, false)
                    .await
                    .map_err(GrpcError::from)?;
                block.map(|b| match b.transactions {
                    Some(evolve_rpc_types::block::BlockTransactions::Hashes(h)) => h.len() as u64,
                    Some(evolve_rpc_types::block::BlockTransactions::Full(f)) => f.len() as u64,
                    None => 0,
                })
            }
            None => None,
        };

        Ok(Response::new(proto::GetBlockTransactionCountResponse {
            count,
        }))
    }

    async fn get_block_transaction_count_by_hash(
        &self,
        request: Request<proto::GetBlockTransactionCountByHashRequest>,
    ) -> Result<Response<proto::GetBlockTransactionCountResponse>, Status> {
        let req = request.into_inner();
        let hash = req
            .hash
            .as_ref()
            .and_then(proto_to_b256)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid block hash".to_string()))?;

        let block = self
            .state
            .get_block_by_hash(hash, false)
            .await
            .map_err(GrpcError::from)?;

        let count = block.map(|b| match b.transactions {
            Some(evolve_rpc_types::block::BlockTransactions::Hashes(h)) => h.len() as u64,
            Some(evolve_rpc_types::block::BlockTransactions::Full(f)) => f.len() as u64,
            None => 0,
        });

        Ok(Response::new(proto::GetBlockTransactionCountResponse {
            count,
        }))
    }

    async fn get_transaction_by_hash(
        &self,
        request: Request<proto::GetTransactionByHashRequest>,
    ) -> Result<Response<proto::GetTransactionByHashResponse>, Status> {
        let req = request.into_inner();
        let hash =
            req.hash.as_ref().and_then(proto_to_b256).ok_or_else(|| {
                GrpcError::InvalidArgument("Invalid transaction hash".to_string())
            })?;

        let tx = self
            .state
            .get_transaction_by_hash(hash)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetTransactionByHashResponse {
            transaction: tx.map(|t| rpc_transaction_to_proto(&t)),
        }))
    }

    async fn get_transaction_receipt(
        &self,
        request: Request<proto::GetTransactionReceiptRequest>,
    ) -> Result<Response<proto::GetTransactionReceiptResponse>, Status> {
        let req = request.into_inner();
        let hash =
            req.hash.as_ref().and_then(proto_to_b256).ok_or_else(|| {
                GrpcError::InvalidArgument("Invalid transaction hash".to_string())
            })?;

        let receipt = self
            .state
            .get_transaction_receipt(hash)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetTransactionReceiptResponse {
            receipt: receipt.map(|r| rpc_receipt_to_proto(&r)),
        }))
    }

    async fn get_transaction_count(
        &self,
        request: Request<proto::GetTransactionCountRequest>,
    ) -> Result<Response<proto::GetTransactionCountResponse>, Status> {
        let req = request.into_inner();
        let address = req
            .address
            .as_ref()
            .and_then(proto_to_address)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid address".to_string()))?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let count = self
            .state
            .get_transaction_count(address, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetTransactionCountResponse { count }))
    }

    async fn get_balance(
        &self,
        request: Request<proto::GetBalanceRequest>,
    ) -> Result<Response<proto::GetBalanceResponse>, Status> {
        let req = request.into_inner();
        let address = req
            .address
            .as_ref()
            .and_then(proto_to_address)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid address".to_string()))?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let balance = self
            .state
            .get_balance(address, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetBalanceResponse {
            balance: Some(u256_to_proto(balance)),
        }))
    }

    async fn get_code(
        &self,
        request: Request<proto::GetCodeRequest>,
    ) -> Result<Response<proto::GetCodeResponse>, Status> {
        let req = request.into_inner();
        let address = req
            .address
            .as_ref()
            .and_then(proto_to_address)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid address".to_string()))?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let code = self
            .state
            .get_code(address, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetCodeResponse {
            code: code.to_vec(),
        }))
    }

    async fn get_storage_at(
        &self,
        request: Request<proto::GetStorageAtRequest>,
    ) -> Result<Response<proto::GetStorageAtResponse>, Status> {
        let req = request.into_inner();
        let address = req
            .address
            .as_ref()
            .and_then(proto_to_address)
            .ok_or_else(|| GrpcError::InvalidArgument("Invalid address".to_string()))?;

        let position =
            req.position.as_ref().map(proto_to_u256).ok_or_else(|| {
                GrpcError::InvalidArgument("Invalid storage position".to_string())
            })?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let value = self
            .state
            .get_storage_at(address, position, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetStorageAtResponse {
            value: Some(b256_to_proto(value)),
        }))
    }

    async fn call(
        &self,
        request: Request<proto::ExecuteCallRequest>,
    ) -> Result<Response<proto::CallResponse>, Status> {
        let req = request.into_inner();
        let call_request = req
            .request
            .as_ref()
            .map(proto_to_call_request)
            .ok_or_else(|| GrpcError::InvalidArgument("Missing call request".to_string()))?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let result = self
            .state
            .call(&call_request, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::CallResponse {
            result: result.to_vec(),
        }))
    }

    async fn estimate_gas(
        &self,
        request: Request<proto::EstimateGasRequest>,
    ) -> Result<Response<proto::EstimateGasResponse>, Status> {
        let req = request.into_inner();
        let call_request = req
            .request
            .as_ref()
            .map(proto_to_call_request)
            .ok_or_else(|| GrpcError::InvalidArgument("Missing call request".to_string()))?;

        let block_num = self
            .resolve_block(req.block.as_ref())
            .await
            .map_err(GrpcError::from)?;

        let gas = self
            .state
            .estimate_gas(&call_request, block_num)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::EstimateGasResponse { gas }))
    }

    async fn get_logs(
        &self,
        request: Request<proto::GetLogsRequest>,
    ) -> Result<Response<proto::GetLogsResponse>, Status> {
        let req = request.into_inner();
        let filter = req
            .filter
            .as_ref()
            .map(proto_to_log_filter)
            .unwrap_or_default();

        let logs = self
            .state
            .get_logs(&filter)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetLogsResponse {
            logs: logs.iter().map(rpc_log_to_proto).collect(),
        }))
    }

    async fn send_raw_transaction(
        &self,
        request: Request<proto::SendRawTransactionRequest>,
    ) -> Result<Response<proto::SendRawTransactionResponse>, Status> {
        let req = request.into_inner();
        if req.data.is_empty() {
            return Err(GrpcError::InvalidArgument("Empty transaction data".to_string()).into());
        }

        let hash = self
            .state
            .send_raw_transaction(&req.data)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::SendRawTransactionResponse {
            hash: Some(b256_to_proto(hash)),
        }))
    }

    async fn list_modules(
        &self,
        _request: Request<proto::ListModulesRequest>,
    ) -> Result<Response<proto::ListModulesResponse>, Status> {
        let identifiers = self
            .state
            .list_module_identifiers()
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::ListModulesResponse { identifiers }))
    }

    async fn get_module_schema(
        &self,
        request: Request<proto::GetModuleSchemaRequest>,
    ) -> Result<Response<proto::GetModuleSchemaResponse>, Status> {
        let req = request.into_inner();
        let schema = self
            .state
            .get_module_schema(&req.identifier)
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetModuleSchemaResponse {
            schema: schema.map(|s| account_schema_to_proto(&s)),
        }))
    }

    async fn get_all_schemas(
        &self,
        _request: Request<proto::GetAllSchemasRequest>,
    ) -> Result<Response<proto::GetAllSchemasResponse>, Status> {
        let schemas = self
            .state
            .get_all_schemas()
            .await
            .map_err(GrpcError::from)?;

        Ok(Response::new(proto::GetAllSchemasResponse {
            schemas: schemas.iter().map(account_schema_to_proto).collect(),
        }))
    }
}

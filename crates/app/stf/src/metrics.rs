use crate::execution_state::ExecutionMetrics;

#[derive(Debug, Clone, Default)]
pub struct TxExecutionMetrics {
    pub index: u64,
    pub duration_ns: u64,
    pub gas_used: u64,
    pub storage_reads: u64,
    pub storage_writes: u64,
    pub events_emitted: u64,
    pub checkpoints: u64,
    pub restores: u64,
    pub overlay_len_max: usize,
    pub undo_log_len_max: usize,
}

impl TxExecutionMetrics {
    pub fn from_delta(
        index: u64,
        duration_ns: u64,
        gas_used: u64,
        delta: ExecutionMetrics,
    ) -> Self {
        Self {
            index,
            duration_ns,
            gas_used,
            storage_reads: delta.storage_reads,
            storage_writes: delta.storage_writes,
            events_emitted: delta.events_emitted,
            checkpoints: delta.checkpoints,
            restores: delta.restores,
            overlay_len_max: delta.overlay_len_max,
            undo_log_len_max: delta.undo_log_len_max,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockExecutionMetrics {
    pub begin_block_ns: u64,
    pub end_block_ns: u64,
    pub total_ns: u64,
    pub storage_reads: u64,
    pub storage_writes: u64,
    pub events_emitted: u64,
    pub checkpoints: u64,
    pub restores: u64,
    pub overlay_len_max: usize,
    pub undo_log_len_max: usize,
    pub txs: Vec<TxExecutionMetrics>,
}

impl BlockExecutionMetrics {
    pub fn from_delta(
        begin_block_ns: u64,
        end_block_ns: u64,
        total_ns: u64,
        delta: ExecutionMetrics,
        txs: Vec<TxExecutionMetrics>,
    ) -> Self {
        Self {
            begin_block_ns,
            end_block_ns,
            total_ns,
            storage_reads: delta.storage_reads,
            storage_writes: delta.storage_writes,
            events_emitted: delta.events_emitted,
            checkpoints: delta.checkpoints,
            restores: delta.restores,
            overlay_len_max: delta.overlay_len_max,
            undo_log_len_max: delta.undo_log_len_max,
            txs,
        }
    }
}

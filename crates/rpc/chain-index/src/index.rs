//! Chain index trait and persistent implementation.
//!
//! The `ChainIndex` trait defines operations for storing and retrieving chain data.
//! `PersistentChainIndex` implements this trait using a SQLite database with a
//! connection pool (r2d2) for concurrent reads and a dedicated writer connection.

use std::sync::Mutex;

use alloy_primitives::B256;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

use crate::cache::ChainCache;
use crate::error::{ChainIndexError, ChainIndexResult};
use crate::types::{StoredBlock, StoredLog, StoredReceipt, StoredTransaction, TxLocation};

/// Trait for chain data indexing operations.
///
/// All methods are synchronous to avoid Send bound issues with the storage layer.
pub trait ChainIndex: Send + Sync {
    /// Get the latest indexed block number.
    fn latest_block_number(&self) -> ChainIndexResult<Option<u64>>;

    /// Get a block by number.
    fn get_block(&self, number: u64) -> ChainIndexResult<Option<StoredBlock>>;

    /// Get a block by hash.
    fn get_block_by_hash(&self, hash: B256) -> ChainIndexResult<Option<StoredBlock>>;

    /// Get block number by hash.
    fn get_block_number(&self, hash: B256) -> ChainIndexResult<Option<u64>>;

    /// Get transaction hashes in a block.
    fn get_block_transactions(&self, number: u64) -> ChainIndexResult<Vec<B256>>;

    /// Get a transaction by hash.
    fn get_transaction(&self, hash: B256) -> ChainIndexResult<Option<StoredTransaction>>;

    /// Get transaction location (block number and index).
    fn get_transaction_location(&self, hash: B256) -> ChainIndexResult<Option<TxLocation>>;

    /// Get a transaction receipt by hash.
    fn get_receipt(&self, hash: B256) -> ChainIndexResult<Option<StoredReceipt>>;

    /// Get logs for a block.
    fn get_logs_by_block(&self, number: u64) -> ChainIndexResult<Vec<StoredLog>>;

    /// Store a block with its transactions and receipts.
    /// This is the only potentially slow operation as it writes to storage.
    fn store_block(
        &self,
        block: StoredBlock,
        transactions: Vec<StoredTransaction>,
        receipts: Vec<StoredReceipt>,
    ) -> ChainIndexResult<()>;
}

/// Persistent chain index backed by SQLite.
///
/// Uses a connection pool for concurrent reads and a dedicated writer connection
/// for serialized writes. SQLite WAL mode allows readers to proceed without
/// blocking the writer and vice versa.
pub struct PersistentChainIndex {
    /// Connection pool for read operations (concurrent).
    read_pool: Pool<SqliteConnectionManager>,
    /// Dedicated connection for write operations (serialized).
    writer: Mutex<Connection>,
    cache: ChainCache,
}

/// Configure a connection with standard PRAGMAs for WAL mode.
fn configure_connection(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;",
    )
}

impl PersistentChainIndex {
    /// Create a new persistent chain index backed by an on-disk SQLite database.
    pub fn new(db_path: impl AsRef<std::path::Path>) -> ChainIndexResult<Self> {
        // Writer connection -- dedicated for store_block
        let writer = Connection::open(&db_path)?;
        configure_connection(&writer)?;

        // Read pool -- concurrent read-only connections
        let manager = SqliteConnectionManager::file(&db_path)
            .with_flags(
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .with_init(|conn| configure_connection(conn));
        let read_pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(|e| ChainIndexError::Sqlite(e.to_string()))?;

        let index = Self {
            read_pool,
            writer: Mutex::new(writer),
            cache: ChainCache::with_defaults(),
        };
        index.init_schema()?;
        Ok(index)
    }

    /// Create an in-memory chain index for testing.
    ///
    /// In-memory SQLite DBs are per-connection, so tests use a single shared
    /// connection for both reads and writes via a file-based shared cache URI.
    pub fn in_memory() -> ChainIndexResult<Self> {
        // Use a named in-memory DB with shared cache so all connections see the same data.
        let uri = format!("file:test_{}?mode=memory&cache=shared", unique_id());
        let writer = Connection::open(&uri)?;
        configure_connection(&writer)?;

        let manager =
            SqliteConnectionManager::file(&uri).with_init(|conn| configure_connection(conn));
        let read_pool = Pool::builder()
            .max_size(2)
            .build(manager)
            .map_err(|e| ChainIndexError::Sqlite(e.to_string()))?;

        let index = Self {
            read_pool,
            writer: Mutex::new(writer),
            cache: ChainCache::with_defaults(),
        };
        index.init_schema()?;
        Ok(index)
    }

    /// Get a read connection from the pool.
    fn read_conn(&self) -> ChainIndexResult<r2d2::PooledConnection<SqliteConnectionManager>> {
        self.read_pool
            .get()
            .map_err(|e| ChainIndexError::Sqlite(e.to_string()))
    }

    /// Get direct access to the cache.
    pub fn cache(&self) -> &ChainCache {
        &self.cache
    }

    /// Initialize from storage, loading the latest block number.
    pub fn initialize(&self) -> ChainIndexResult<()> {
        if let Some(latest) = self.load_latest_block_number()? {
            self.cache.set_latest_block_number(latest);
            tracing::info!("Chain index initialized at block {}", latest);
        } else {
            tracing::info!("Chain index initialized (empty)");
        }
        Ok(())
    }

    fn init_schema(&self) -> ChainIndexResult<()> {
        let conn = self.writer.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS blocks (
                 number INTEGER PRIMARY KEY,
                 hash BLOB NOT NULL UNIQUE,
                 parent_hash BLOB NOT NULL,
                 state_root BLOB NOT NULL,
                 transactions_root BLOB NOT NULL,
                 receipts_root BLOB NOT NULL,
                 timestamp INTEGER NOT NULL,
                 gas_used INTEGER NOT NULL,
                 gas_limit INTEGER NOT NULL,
                 transaction_count INTEGER NOT NULL,
                 miner BLOB NOT NULL,
                 extra_data BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_blocks_hash ON blocks(hash);

             CREATE TABLE IF NOT EXISTS transactions (
                 hash BLOB PRIMARY KEY,
                 block_number INTEGER NOT NULL,
                 block_hash BLOB NOT NULL,
                 transaction_index INTEGER NOT NULL,
                 from_addr BLOB NOT NULL,
                 to_addr BLOB,
                 value BLOB NOT NULL,
                 gas INTEGER NOT NULL,
                 gas_price BLOB NOT NULL,
                 input BLOB NOT NULL,
                 nonce INTEGER NOT NULL,
                 v INTEGER NOT NULL,
                 r BLOB NOT NULL,
                 s BLOB NOT NULL,
                 tx_type INTEGER NOT NULL,
                 chain_id INTEGER,
                 FOREIGN KEY (block_number) REFERENCES blocks(number)
             );
             CREATE INDEX IF NOT EXISTS idx_tx_block ON transactions(block_number);

             CREATE TABLE IF NOT EXISTS receipts (
                 transaction_hash BLOB PRIMARY KEY,
                 transaction_index INTEGER NOT NULL,
                 block_hash BLOB NOT NULL,
                 block_number INTEGER NOT NULL,
                 from_addr BLOB NOT NULL,
                 to_addr BLOB,
                 cumulative_gas_used INTEGER NOT NULL,
                 gas_used INTEGER NOT NULL,
                 contract_address BLOB,
                 status INTEGER NOT NULL,
                 tx_type INTEGER NOT NULL,
                 FOREIGN KEY (block_number) REFERENCES blocks(number)
             );
             CREATE INDEX IF NOT EXISTS idx_receipts_block ON receipts(block_number);

             CREATE TABLE IF NOT EXISTS logs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 block_number INTEGER NOT NULL,
                 transaction_hash BLOB NOT NULL,
                 address BLOB NOT NULL,
                 topics BLOB NOT NULL,
                 data BLOB NOT NULL,
                 FOREIGN KEY (block_number) REFERENCES blocks(number)
             );
             CREATE INDEX IF NOT EXISTS idx_logs_block ON logs(block_number);
             CREATE INDEX IF NOT EXISTS idx_logs_address ON logs(address);
             CREATE INDEX IF NOT EXISTS idx_logs_tx ON logs(transaction_hash);

             CREATE TABLE IF NOT EXISTS metadata (
                 key TEXT PRIMARY KEY,
                 value BLOB NOT NULL
             );",
        )?;
        Ok(())
    }

    fn load_latest_block_number(&self) -> ChainIndexResult<Option<u64>> {
        let conn = self.read_conn()?;
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'latest_block'",
            [],
            |row| row.get(0),
        );
        match result {
            Ok(n) => Ok(Some(n as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn row_to_stored_block(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredBlock> {
        use alloy_primitives::Bytes;

        let number: i64 = row.get(0)?;
        let hash_bytes: Vec<u8> = row.get(1)?;
        let parent_hash_bytes: Vec<u8> = row.get(2)?;
        let state_root_bytes: Vec<u8> = row.get(3)?;
        let transactions_root_bytes: Vec<u8> = row.get(4)?;
        let receipts_root_bytes: Vec<u8> = row.get(5)?;
        let timestamp: i64 = row.get(6)?;
        let gas_used: i64 = row.get(7)?;
        let gas_limit: i64 = row.get(8)?;
        let transaction_count: i64 = row.get(9)?;
        let miner_bytes: Vec<u8> = row.get(10)?;
        let extra_data_bytes: Vec<u8> = row.get(11)?;

        Ok(StoredBlock {
            number: number as u64,
            hash: b256_from_row(&hash_bytes, 1)?,
            parent_hash: b256_from_row(&parent_hash_bytes, 2)?,
            state_root: b256_from_row(&state_root_bytes, 3)?,
            transactions_root: b256_from_row(&transactions_root_bytes, 4)?,
            receipts_root: b256_from_row(&receipts_root_bytes, 5)?,
            timestamp: timestamp as u64,
            gas_used: gas_used as u64,
            gas_limit: gas_limit as u64,
            transaction_count: transaction_count as u32,
            miner: address_from_row(&miner_bytes, 10)?,
            extra_data: Bytes::from(extra_data_bytes),
        })
    }

    fn row_to_stored_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTransaction> {
        use alloy_primitives::{Bytes, U256};

        let hash_bytes: Vec<u8> = row.get(0)?;
        let block_number: i64 = row.get(1)?;
        let block_hash_bytes: Vec<u8> = row.get(2)?;
        let transaction_index: i64 = row.get(3)?;
        let from_bytes: Vec<u8> = row.get(4)?;
        let to_bytes: Option<Vec<u8>> = row.get(5)?;
        let value_bytes: Vec<u8> = row.get(6)?;
        let gas: i64 = row.get(7)?;
        let gas_price_bytes: Vec<u8> = row.get(8)?;
        let input_bytes: Vec<u8> = row.get(9)?;
        let nonce: i64 = row.get(10)?;
        let v: i64 = row.get(11)?;
        let r_bytes: Vec<u8> = row.get(12)?;
        let s_bytes: Vec<u8> = row.get(13)?;
        let tx_type: i64 = row.get(14)?;
        let chain_id: Option<i64> = row.get(15)?;

        let to = to_bytes
            .as_deref()
            .map(|b| address_from_row(b, 5))
            .transpose()?;

        Ok(StoredTransaction {
            hash: b256_from_row(&hash_bytes, 0)?,
            block_number: block_number as u64,
            block_hash: b256_from_row(&block_hash_bytes, 2)?,
            transaction_index: transaction_index as u32,
            from: address_from_row(&from_bytes, 4)?,
            to,
            value: U256::from_be_slice(&value_bytes),
            gas: gas as u64,
            gas_price: U256::from_be_slice(&gas_price_bytes),
            input: Bytes::from(input_bytes),
            nonce: nonce as u64,
            v: v as u64,
            r: U256::from_be_slice(&r_bytes),
            s: U256::from_be_slice(&s_bytes),
            tx_type: tx_type as u8,
            chain_id: chain_id.map(|c| c as u64),
        })
    }

    fn row_to_stored_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredReceipt> {
        let transaction_hash_bytes: Vec<u8> = row.get(0)?;
        let transaction_index: i64 = row.get(1)?;
        let block_hash_bytes: Vec<u8> = row.get(2)?;
        let block_number: i64 = row.get(3)?;
        let from_bytes: Vec<u8> = row.get(4)?;
        let to_bytes: Option<Vec<u8>> = row.get(5)?;
        let cumulative_gas_used: i64 = row.get(6)?;
        let gas_used: i64 = row.get(7)?;
        let contract_address_bytes: Option<Vec<u8>> = row.get(8)?;
        let status: i64 = row.get(9)?;
        let tx_type: i64 = row.get(10)?;

        let to = to_bytes
            .as_deref()
            .map(|b| address_from_row(b, 5))
            .transpose()?;
        let contract_address = contract_address_bytes
            .as_deref()
            .map(|b| address_from_row(b, 8))
            .transpose()?;

        Ok(StoredReceipt {
            transaction_hash: b256_from_row(&transaction_hash_bytes, 0)?,
            transaction_index: transaction_index as u32,
            block_hash: b256_from_row(&block_hash_bytes, 2)?,
            block_number: block_number as u64,
            from: address_from_row(&from_bytes, 4)?,
            to,
            cumulative_gas_used: cumulative_gas_used as u64,
            gas_used: gas_used as u64,
            contract_address,
            logs: vec![], // logs are stored separately
            status: status as u8,
            tx_type: tx_type as u8,
        })
    }
}

impl ChainIndex for PersistentChainIndex {
    fn latest_block_number(&self) -> ChainIndexResult<Option<u64>> {
        if let Some(n) = self.cache.latest_block_number() {
            return Ok(Some(n));
        }
        self.load_latest_block_number()
    }

    fn get_block(&self, number: u64) -> ChainIndexResult<Option<StoredBlock>> {
        if let Some(block) = self.cache.get_block_by_number(number) {
            return Ok(Some((*block).clone()));
        }

        let conn = self.read_conn()?;
        let result = conn.query_row(
            "SELECT number, hash, parent_hash, state_root, transactions_root, receipts_root,
                    timestamp, gas_used, gas_limit, transaction_count, miner, extra_data
             FROM blocks WHERE number = ?",
            params![number as i64],
            Self::row_to_stored_block,
        );

        match result {
            Ok(block) => {
                self.cache.insert_block(block.clone());
                Ok(Some(block))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_block_by_hash(&self, hash: B256) -> ChainIndexResult<Option<StoredBlock>> {
        if let Some(block) = self.cache.get_block_by_hash(hash) {
            return Ok(Some((*block).clone()));
        }

        let number = match self.get_block_number(hash)? {
            Some(n) => n,
            None => return Ok(None),
        };

        self.get_block(number)
    }

    fn get_block_number(&self, hash: B256) -> ChainIndexResult<Option<u64>> {
        if let Some(n) = self.cache.get_block_number_by_hash(hash) {
            return Ok(Some(n));
        }

        let conn = self.read_conn()?;
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT number FROM blocks WHERE hash = ?",
            params![hash.as_slice()],
            |row| row.get(0),
        );

        match result {
            Ok(n) => Ok(Some(n as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_block_transactions(&self, number: u64) -> ChainIndexResult<Vec<B256>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT hash FROM transactions WHERE block_number = ? ORDER BY transaction_index",
        )?;

        let hashes: rusqlite::Result<Vec<B256>> = stmt
            .query_map(params![number as i64], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                b256_from_row(&bytes, 0)
            })?
            .collect();

        Ok(hashes?)
    }

    fn get_transaction(&self, hash: B256) -> ChainIndexResult<Option<StoredTransaction>> {
        if let Some(tx) = self.cache.get_transaction(hash) {
            return Ok(Some((*tx).clone()));
        }

        let conn = self.read_conn()?;
        let result = conn.query_row(
            "SELECT hash, block_number, block_hash, transaction_index, from_addr, to_addr,
                    value, gas, gas_price, input, nonce, v, r, s, tx_type, chain_id
             FROM transactions WHERE hash = ?",
            params![hash.as_slice()],
            Self::row_to_stored_transaction,
        );

        match result {
            Ok(tx) => {
                self.cache.insert_transaction(tx.clone());
                Ok(Some(tx))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_transaction_location(&self, hash: B256) -> ChainIndexResult<Option<TxLocation>> {
        let conn = self.read_conn()?;
        let result: rusqlite::Result<(i64, i64)> = conn.query_row(
            "SELECT block_number, transaction_index FROM transactions WHERE hash = ?",
            params![hash.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        match result {
            Ok((block_number, transaction_index)) => Ok(Some(TxLocation {
                block_number: block_number as u64,
                transaction_index: transaction_index as u32,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_receipt(&self, hash: B256) -> ChainIndexResult<Option<StoredReceipt>> {
        if let Some(receipt) = self.cache.get_receipt(hash) {
            return Ok(Some((*receipt).clone()));
        }

        let conn = self.read_conn()?;
        let result = conn.query_row(
            "SELECT transaction_hash, transaction_index, block_hash, block_number,
                    from_addr, to_addr, cumulative_gas_used, gas_used, contract_address,
                    status, tx_type
             FROM receipts WHERE transaction_hash = ?",
            params![hash.as_slice()],
            Self::row_to_stored_receipt,
        );

        match result {
            Ok(mut receipt) => {
                // Load logs for this receipt
                let logs = load_logs_for_tx(&conn, hash)?;
                receipt.logs = logs;
                self.cache.insert_receipt(receipt.clone());
                Ok(Some(receipt))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_logs_by_block(&self, number: u64) -> ChainIndexResult<Vec<StoredLog>> {
        let conn = self.read_conn()?;
        let mut stmt = conn
            .prepare("SELECT address, topics, data FROM logs WHERE block_number = ? ORDER BY id")?;

        let logs: rusqlite::Result<Vec<StoredLog>> = stmt
            .query_map(params![number as i64], |row| {
                let address_bytes: Vec<u8> = row.get(0)?;
                let topics_json: Vec<u8> = row.get(1)?;
                let data_bytes: Vec<u8> = row.get(2)?;
                Ok((address_bytes, topics_json, data_bytes))
            })?
            .map(|r| {
                let (address_bytes, topics_json, data_bytes) = r?;
                parse_stored_log(&address_bytes, &topics_json, &data_bytes)
            })
            .collect();

        Ok(logs?)
    }

    fn store_block(
        &self,
        block: StoredBlock,
        transactions: Vec<StoredTransaction>,
        receipts: Vec<StoredReceipt>,
    ) -> ChainIndexResult<()> {
        let block_number = block.number;
        let block_hash = block.hash;

        let mut conn = self.writer.lock().unwrap();
        let tx = conn.transaction()?;

        // Delete existing data for this block number (handles re-indexing the same block)
        tx.execute(
            "DELETE FROM logs WHERE block_number = ?",
            params![block_number as i64],
        )?;
        tx.execute(
            "DELETE FROM receipts WHERE block_number = ?",
            params![block_number as i64],
        )?;
        tx.execute(
            "DELETE FROM transactions WHERE block_number = ?",
            params![block_number as i64],
        )?;

        // Insert block
        tx.execute(
            "INSERT OR REPLACE INTO blocks
             (number, hash, parent_hash, state_root, transactions_root, receipts_root,
              timestamp, gas_used, gas_limit, transaction_count, miner, extra_data)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                block.number as i64,
                block.hash.as_slice(),
                block.parent_hash.as_slice(),
                block.state_root.as_slice(),
                block.transactions_root.as_slice(),
                block.receipts_root.as_slice(),
                block.timestamp as i64,
                block.gas_used as i64,
                block.gas_limit as i64,
                block.transaction_count as i64,
                block.miner.as_slice(),
                block.extra_data.as_ref(),
            ],
        )?;

        // Insert transactions using array index as the authoritative transaction_index
        for (idx, transaction) in transactions.iter().enumerate() {
            insert_transaction(&tx, transaction, idx as i64)?;
        }

        // Insert receipts and logs
        for (idx, receipt) in receipts.iter().enumerate() {
            insert_receipt(&tx, receipt, idx as i64)?;
            for log in &receipt.logs {
                insert_log(&tx, block_number, receipt.transaction_hash, log)?;
            }
        }

        // Update latest block number metadata
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('latest_block', ?)",
            params![block_number as i64],
        )?;

        tx.commit()?;

        // Update cache after successful commit
        self.cache
            .insert_block_with_data(block, transactions, receipts);

        tracing::debug!("Stored block {} with hash {:?}", block_number, block_hash);

        Ok(())
    }
}

fn insert_transaction(
    tx: &rusqlite::Transaction<'_>,
    transaction: &StoredTransaction,
    array_index: i64,
) -> ChainIndexResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO transactions
         (hash, block_number, block_hash, transaction_index, from_addr, to_addr,
          value, gas, gas_price, input, nonce, v, r, s, tx_type, chain_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            transaction.hash.as_slice(),
            transaction.block_number as i64,
            transaction.block_hash.as_slice(),
            array_index,
            transaction.from.as_slice(),
            transaction.to.as_ref().map(|a| a.as_slice()),
            &u256_to_be_bytes(transaction.value) as &[u8],
            transaction.gas as i64,
            &u256_to_be_bytes(transaction.gas_price) as &[u8],
            transaction.input.as_ref(),
            transaction.nonce as i64,
            transaction.v as i64,
            &u256_to_be_bytes(transaction.r) as &[u8],
            &u256_to_be_bytes(transaction.s) as &[u8],
            transaction.tx_type as i64,
            transaction.chain_id.map(|c| c as i64),
        ],
    )?;
    Ok(())
}

fn insert_receipt(
    tx: &rusqlite::Transaction<'_>,
    receipt: &StoredReceipt,
    array_index: i64,
) -> ChainIndexResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO receipts
         (transaction_hash, transaction_index, block_hash, block_number, from_addr, to_addr,
          cumulative_gas_used, gas_used, contract_address, status, tx_type)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            receipt.transaction_hash.as_slice(),
            array_index,
            receipt.block_hash.as_slice(),
            receipt.block_number as i64,
            receipt.from.as_slice(),
            receipt.to.as_ref().map(|a| a.as_slice()),
            receipt.cumulative_gas_used as i64,
            receipt.gas_used as i64,
            receipt.contract_address.as_ref().map(|a| a.as_slice()),
            receipt.status as i64,
            receipt.tx_type as i64,
        ],
    )?;
    Ok(())
}

fn insert_log(
    tx: &rusqlite::Transaction<'_>,
    block_number: u64,
    tx_hash: B256,
    log: &StoredLog,
) -> ChainIndexResult<()> {
    let topics_json = serde_json::to_vec(&log.topics)
        .map_err(|e| ChainIndexError::Serialization(e.to_string()))?;

    tx.execute(
        "INSERT INTO logs (block_number, transaction_hash, address, topics, data)
         VALUES (?, ?, ?, ?, ?)",
        params![
            block_number as i64,
            tx_hash.as_slice(),
            log.address.as_slice(),
            topics_json.as_slice(),
            log.data.as_ref(),
        ],
    )?;
    Ok(())
}

fn load_logs_for_tx(conn: &Connection, tx_hash: B256) -> ChainIndexResult<Vec<StoredLog>> {
    let mut stmt = conn
        .prepare("SELECT address, topics, data FROM logs WHERE transaction_hash = ? ORDER BY id")?;

    let logs: rusqlite::Result<Vec<StoredLog>> = stmt
        .query_map(params![tx_hash.as_slice()], |row| {
            let address_bytes: Vec<u8> = row.get(0)?;
            let topics_json: Vec<u8> = row.get(1)?;
            let data_bytes: Vec<u8> = row.get(2)?;
            Ok((address_bytes, topics_json, data_bytes))
        })?
        .map(|r| {
            let (address_bytes, topics_json, data_bytes) = r?;
            parse_stored_log(&address_bytes, &topics_json, &data_bytes)
        })
        .collect();

    Ok(logs?)
}

fn parse_stored_log(
    address_bytes: &[u8],
    topics_json: &[u8],
    data_bytes: &[u8],
) -> rusqlite::Result<StoredLog> {
    use alloy_primitives::Bytes;

    let topics: Vec<B256> = serde_json::from_slice(topics_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Blob, Box::new(e))
    })?;

    Ok(StoredLog {
        address: address_from_row(address_bytes, 0)?,
        topics,
        data: Bytes::from(data_bytes.to_vec()),
    })
}

/// Generate a unique ID for in-memory shared-cache SQLite databases.
fn unique_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn b256_from_row(bytes: &[u8], col: usize) -> rusqlite::Result<B256> {
    if bytes.len() != 32 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            col,
            rusqlite::types::Type::Blob,
            format!("expected 32 bytes for B256, got {}", bytes.len()).into(),
        ));
    }
    Ok(B256::from_slice(bytes))
}

fn address_from_row(bytes: &[u8], col: usize) -> rusqlite::Result<alloy_primitives::Address> {
    if bytes.len() != 20 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            col,
            rusqlite::types::Type::Blob,
            format!("expected 20 bytes for Address, got {}", bytes.len()).into(),
        ));
    }
    Ok(alloy_primitives::Address::from_slice(bytes))
}

fn u256_to_be_bytes(value: alloy_primitives::U256) -> [u8; 32] {
    value.to_be_bytes()
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, U256};

    /// Helper to create a test StoredBlock.
    pub fn make_test_stored_block(number: u64) -> StoredBlock {
        StoredBlock {
            number,
            hash: B256::repeat_byte(number as u8),
            parent_hash: if number > 0 {
                B256::repeat_byte((number - 1) as u8)
            } else {
                B256::ZERO
            },
            state_root: B256::repeat_byte(0xaa),
            transactions_root: B256::repeat_byte(0xbb),
            receipts_root: B256::repeat_byte(0xcc),
            timestamp: 1000 + number * 12,
            gas_used: 21000,
            gas_limit: 30_000_000,
            transaction_count: 1,
            miner: Address::repeat_byte(0xdd),
            extra_data: Bytes::new(),
        }
    }

    /// Helper to create a test StoredTransaction.
    pub fn make_test_stored_transaction(
        hash: B256,
        block_number: u64,
        block_hash: B256,
        index: u32,
    ) -> StoredTransaction {
        StoredTransaction {
            hash,
            block_number,
            block_hash,
            transaction_index: index,
            from: Address::repeat_byte(0x01),
            to: Some(Address::repeat_byte(0x02)),
            value: U256::from(100u64),
            gas: 21000,
            gas_price: U256::ZERO,
            input: Bytes::new(),
            nonce: 0,
            v: 0,
            r: U256::ZERO,
            s: U256::ZERO,
            tx_type: 0,
            chain_id: Some(1),
        }
    }

    /// Helper to create a test StoredReceipt.
    pub fn make_test_stored_receipt(
        tx_hash: B256,
        block_number: u64,
        block_hash: B256,
        index: u32,
        success: bool,
    ) -> StoredReceipt {
        StoredReceipt {
            transaction_hash: tx_hash,
            transaction_index: index,
            block_hash,
            block_number,
            from: Address::repeat_byte(0x01),
            to: Some(Address::repeat_byte(0x02)),
            cumulative_gas_used: 21000 * (index as u64 + 1),
            gas_used: 21000,
            contract_address: None,
            logs: vec![],
            status: if success { 1 } else { 0 },
            tx_type: 0,
        }
    }

    // ==================== Complex behavior tests ====================

    /// Tests that transaction location uses array index, not the field value.
    #[test]
    fn test_transaction_location_uses_array_index() {
        let index = PersistentChainIndex::in_memory().unwrap();

        let block = make_test_stored_block(100);
        // Create tx with transaction_index=5, but it will be at array position 0
        let tx_hash = B256::repeat_byte(0x40);
        let tx = make_test_stored_transaction(tx_hash, 100, block.hash, 5);
        let receipt = make_test_stored_receipt(tx_hash, 100, block.hash, 5, true);

        index.store_block(block, vec![tx], vec![receipt]).unwrap();

        // TxLocation should use array position (0), not the field value (5)
        let location = index.get_transaction_location(tx_hash).unwrap().unwrap();
        assert_eq!(location.transaction_index, 0);
    }

    /// Tests that transaction ordering is preserved when storing multiple txs.
    #[test]
    fn test_transaction_ordering_preserved() {
        let index = PersistentChainIndex::in_memory().unwrap();

        let block = make_test_stored_block(100);
        let hashes: Vec<_> = (0..5).map(|i| B256::repeat_byte(0x50 + i)).collect();

        let txs: Vec<_> = hashes
            .iter()
            .enumerate()
            .map(|(i, &hash)| make_test_stored_transaction(hash, 100, block.hash, i as u32))
            .collect();
        let receipts: Vec<_> = hashes
            .iter()
            .enumerate()
            .map(|(i, &hash)| make_test_stored_receipt(hash, 100, block.hash, i as u32, true))
            .collect();

        index.store_block(block, txs, receipts).unwrap();

        let retrieved_hashes = index.get_block_transactions(100).unwrap();
        assert_eq!(retrieved_hashes, hashes);
    }

    /// Tests log aggregation from multiple receipts.
    #[test]
    fn test_logs_aggregated_from_receipts() {
        let index = PersistentChainIndex::in_memory().unwrap();

        let block = make_test_stored_block(100);

        let tx1_hash = B256::repeat_byte(0x60);
        let tx2_hash = B256::repeat_byte(0x61);

        let tx1 = make_test_stored_transaction(tx1_hash, 100, block.hash, 0);
        let tx2 = make_test_stored_transaction(tx2_hash, 100, block.hash, 1);

        let mut receipt1 = make_test_stored_receipt(tx1_hash, 100, block.hash, 0, true);
        receipt1.logs = vec![StoredLog {
            address: Address::repeat_byte(0x70),
            topics: vec![B256::repeat_byte(0x71)],
            data: Bytes::from(vec![0x01]),
        }];

        let mut receipt2 = make_test_stored_receipt(tx2_hash, 100, block.hash, 1, true);
        receipt2.logs = vec![
            StoredLog {
                address: Address::repeat_byte(0x72),
                topics: vec![],
                data: Bytes::from(vec![0x02]),
            },
            StoredLog {
                address: Address::repeat_byte(0x73),
                topics: vec![],
                data: Bytes::from(vec![0x03]),
            },
        ];

        index
            .store_block(block, vec![tx1, tx2], vec![receipt1, receipt2])
            .unwrap();

        // All 3 logs should be aggregated
        let logs = index.get_logs_by_block(100).unwrap();
        assert_eq!(logs.len(), 3);
    }

    /// Tests cache population after store_block.
    #[test]
    fn test_cache_populated_on_store() {
        let index = PersistentChainIndex::in_memory().unwrap();

        let block = make_test_stored_block(100);
        let block_hash = block.hash;
        let tx_hash = B256::repeat_byte(0x80);
        let tx = make_test_stored_transaction(tx_hash, 100, block.hash, 0);
        let receipt = make_test_stored_receipt(tx_hash, 100, block.hash, 0, true);

        index.store_block(block, vec![tx], vec![receipt]).unwrap();

        // Cache should be populated without hitting storage again
        let cache = index.cache();
        assert!(cache.get_block_by_number(100).is_some());
        assert!(cache.get_block_by_hash(block_hash).is_some());
        assert!(cache.get_transaction(tx_hash).is_some());
        assert!(cache.get_receipt(tx_hash).is_some());
    }

    /// Tests persistence across index instances (simulating restart).
    #[test]
    fn test_persistence_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("chain-index.sqlite");

        // First instance stores data
        {
            let index1 = PersistentChainIndex::new(&db_path).unwrap();
            for i in 0..5 {
                let block = make_test_stored_block(i);
                index1.store_block(block, vec![], vec![]).unwrap();
            }
        }

        // Second instance (simulating restart) should recover state
        let index2 = PersistentChainIndex::new(&db_path).unwrap();
        index2.initialize().unwrap();

        assert_eq!(index2.latest_block_number().unwrap(), Some(4));
        assert!(index2.get_block(3).unwrap().is_some());
    }
}

// ==================== Model-based tests ====================
#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod model_tests {
    use super::tests::{
        make_test_stored_block, make_test_stored_receipt, make_test_stored_transaction,
    };
    use super::*;
    use alloy_primitives::Address;
    use proptest::prelude::*;
    use std::collections::HashMap;

    /// A simple reference model for ChainIndex behavior.
    #[derive(Debug, Default, Clone)]
    #[allow(dead_code)]
    struct ChainIndexModel {
        blocks_by_number: HashMap<u64, StoredBlock>,
        blocks_by_hash: HashMap<B256, StoredBlock>,
        transactions: HashMap<B256, StoredTransaction>,
        tx_locations: HashMap<B256, TxLocation>,
        receipts: HashMap<B256, StoredReceipt>,
        logs_by_block: HashMap<u64, Vec<StoredLog>>,
        latest_block: Option<u64>,
    }

    #[allow(dead_code)]
    impl ChainIndexModel {
        fn new() -> Self {
            Self::default()
        }

        fn store_block(
            &mut self,
            block: StoredBlock,
            transactions: Vec<StoredTransaction>,
            receipts: Vec<StoredReceipt>,
        ) {
            let block_number = block.number;
            let block_hash = block.hash;

            self.blocks_by_number.insert(block_number, block.clone());
            self.blocks_by_hash.insert(block_hash, block);

            for (idx, tx) in transactions.iter().enumerate() {
                self.transactions.insert(tx.hash, tx.clone());
                self.tx_locations.insert(
                    tx.hash,
                    TxLocation {
                        block_number,
                        transaction_index: idx as u32,
                    },
                );
            }

            let mut all_logs = Vec::new();
            for receipt in receipts {
                all_logs.extend(receipt.logs.clone());
                self.receipts.insert(receipt.transaction_hash, receipt);
            }
            if !all_logs.is_empty() {
                self.logs_by_block.insert(block_number, all_logs);
            }

            self.latest_block = Some(
                self.latest_block
                    .map(|l| l.max(block_number))
                    .unwrap_or(block_number),
            );
        }

        fn get_block(&self, number: u64) -> Option<&StoredBlock> {
            self.blocks_by_number.get(&number)
        }

        fn get_block_by_hash(&self, hash: B256) -> Option<&StoredBlock> {
            self.blocks_by_hash.get(&hash)
        }

        fn get_block_number(&self, hash: B256) -> Option<u64> {
            self.blocks_by_hash.get(&hash).map(|b| b.number)
        }

        fn get_transaction(&self, hash: B256) -> Option<&StoredTransaction> {
            self.transactions.get(&hash)
        }

        fn get_transaction_location(&self, hash: B256) -> Option<&TxLocation> {
            self.tx_locations.get(&hash)
        }

        fn get_receipt(&self, hash: B256) -> Option<&StoredReceipt> {
            self.receipts.get(&hash)
        }

        fn get_logs_by_block(&self, number: u64) -> Vec<StoredLog> {
            self.logs_by_block.get(&number).cloned().unwrap_or_default()
        }

        fn latest_block_number(&self) -> Option<u64> {
            self.latest_block
        }
    }

    /// Operations that can be performed on a ChainIndex.
    #[derive(Debug, Clone)]
    enum Operation {
        StoreBlock { block_number: u64, tx_count: usize },
        GetBlock { number: u64 },
        GetBlockByHash { block_idx: usize },
        GetTransaction { tx_idx: usize },
        GetReceipt { tx_idx: usize },
        GetTransactionLocation { tx_idx: usize },
        GetLogsByBlock { number: u64 },
        GetLatestBlockNumber,
    }

    /// State tracker for generating valid operations.
    #[derive(Debug, Default)]
    struct OperationState {
        stored_block_numbers: Vec<u64>,
        stored_block_hashes: Vec<B256>,
        stored_tx_hashes: Vec<B256>,
    }

    fn arb_operation(max_block: u64, _max_txs: usize) -> impl Strategy<Value = Operation> {
        prop_oneof![
            (0..max_block, 0..=3usize).prop_map(|(block_number, tx_count)| Operation::StoreBlock {
                block_number,
                tx_count
            }),
            (0..max_block).prop_map(|number| Operation::GetBlock { number }),
            (0..10usize).prop_map(|block_idx| Operation::GetBlockByHash { block_idx }),
            (0..10usize).prop_map(|tx_idx| Operation::GetTransaction { tx_idx }),
            (0..10usize).prop_map(|tx_idx| Operation::GetReceipt { tx_idx }),
            (0..10usize).prop_map(|tx_idx| Operation::GetTransactionLocation { tx_idx }),
            (0..max_block).prop_map(|number| Operation::GetLogsByBlock { number }),
            Just(Operation::GetLatestBlockNumber),
        ]
    }

    fn arb_operations(count: usize) -> impl Strategy<Value = Vec<Operation>> {
        proptest::collection::vec(arb_operation(20, 5), 1..=count)
    }

    fn generate_block_data(
        block_number: u64,
        tx_count: usize,
        tx_counter: &mut u64,
    ) -> (StoredBlock, Vec<StoredTransaction>, Vec<StoredReceipt>) {
        let block = make_test_stored_block(block_number);
        let mut transactions = Vec::new();
        let mut receipts = Vec::new();

        for i in 0..tx_count {
            let tx_id = *tx_counter;
            *tx_counter += 1;

            let tx_hash = B256::from_slice(&{
                let mut bytes = [0u8; 32];
                bytes[0..8].copy_from_slice(&tx_id.to_be_bytes());
                bytes
            });

            let tx = make_test_stored_transaction(tx_hash, block_number, block.hash, i as u32);
            let mut receipt =
                make_test_stored_receipt(tx_hash, block_number, block.hash, i as u32, true);

            if i % 2 == 0 {
                receipt.logs.push(crate::types::StoredLog {
                    address: Address::repeat_byte((tx_id % 256) as u8),
                    topics: vec![B256::repeat_byte((tx_id % 256) as u8)],
                    data: alloy_primitives::Bytes::from(vec![(tx_id % 256) as u8]),
                });
            }

            transactions.push(tx);
            receipts.push(receipt);
        }

        (block, transactions, receipts)
    }

    proptest! {
        /// Model-based test: verify that PersistentChainIndex behaves identically to the reference model.
        #[test]
        fn prop_chain_index_matches_model(operations in arb_operations(30)) {
            let index = PersistentChainIndex::in_memory().unwrap();
            let mut model = ChainIndexModel::new();
            let mut state = OperationState::default();
            let mut tx_counter = 0u64;

            for op in operations {
                match op {
                    Operation::StoreBlock { block_number, tx_count } => {
                        if state.stored_block_numbers.contains(&block_number) {
                            continue;
                        }

                        let (block, txs, receipts) =
                            generate_block_data(block_number, tx_count, &mut tx_counter);

                        state.stored_block_numbers.push(block_number);
                        state.stored_block_hashes.push(block.hash);
                        for tx in &txs {
                            state.stored_tx_hashes.push(tx.hash);
                        }

                        model.store_block(block.clone(), txs.clone(), receipts.clone());
                        index.store_block(block, txs, receipts).unwrap();
                    }
                    Operation::GetBlock { number } => {
                        let model_result = model.get_block(number);
                        let index_result = index.get_block(number).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.number, i.number);
                                prop_assert_eq!(m.hash, i.hash);
                            }
                            _ => {
                                prop_assert!(false, "Mismatch: model={:?}, index={:?}", model_result.is_some(), index_result.is_some());
                            }
                        }
                    }
                    Operation::GetBlockByHash { block_idx } => {
                        if state.stored_block_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_block_hashes[block_idx % state.stored_block_hashes.len()];

                        let model_result = model.get_block_by_hash(hash);
                        let index_result = index.get_block_by_hash(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.number, i.number);
                                prop_assert_eq!(m.hash, i.hash);
                            }
                            _ => {
                                prop_assert!(false, "GetBlockByHash mismatch");
                            }
                        }
                    }
                    Operation::GetTransaction { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_transaction(hash);
                        let index_result = index.get_transaction(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.hash, i.hash);
                                prop_assert_eq!(m.block_number, i.block_number);
                            }
                            _ => {
                                prop_assert!(false, "GetTransaction mismatch");
                            }
                        }
                    }
                    Operation::GetReceipt { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_receipt(hash);
                        let index_result = index.get_receipt(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.transaction_hash, i.transaction_hash);
                                prop_assert_eq!(m.status, i.status);
                            }
                            _ => {
                                prop_assert!(false, "GetReceipt mismatch");
                            }
                        }
                    }
                    Operation::GetTransactionLocation { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_transaction_location(hash);
                        let index_result = index.get_transaction_location(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.block_number, i.block_number);
                                prop_assert_eq!(m.transaction_index, i.transaction_index);
                            }
                            _ => {
                                prop_assert!(false, "GetTransactionLocation mismatch");
                            }
                        }
                    }
                    Operation::GetLogsByBlock { number } => {
                        let model_logs = model.get_logs_by_block(number);
                        let index_logs = index.get_logs_by_block(number).unwrap();

                        prop_assert_eq!(model_logs.len(), index_logs.len(), "Log count mismatch for block {}", number);
                        for (m, i) in model_logs.iter().zip(index_logs.iter()) {
                            prop_assert_eq!(m.address, i.address);
                            prop_assert_eq!(&m.topics, &i.topics);
                        }
                    }
                    Operation::GetLatestBlockNumber => {
                        let model_result = model.latest_block_number();
                        let index_result = index.latest_block_number().unwrap();
                        prop_assert_eq!(model_result, index_result, "Latest block number mismatch");
                    }
                }
            }
        }

        /// Test that storing the same block number twice doesn't corrupt state.
        #[test]
        fn prop_duplicate_block_storage(block_number in 0u64..100, tx_count1 in 0usize..3, tx_count2 in 0usize..3) {
            let index = PersistentChainIndex::in_memory().unwrap();
            let mut tx_counter = 0u64;

            let (block1, txs1, receipts1) = generate_block_data(block_number, tx_count1, &mut tx_counter);
            index.store_block(block1, txs1, receipts1).unwrap();

            let (block2, txs2, receipts2) = generate_block_data(block_number, tx_count2, &mut tx_counter);
            let block2_hash = block2.hash;
            index.store_block(block2, txs2.clone(), receipts2).unwrap();

            let retrieved = index.get_block(block_number).unwrap().unwrap();
            prop_assert_eq!(retrieved.hash, block2_hash);

            let tx_hashes = index.get_block_transactions(block_number).unwrap();
            prop_assert_eq!(tx_hashes.len(), txs2.len());
        }

        /// Test retrieval of non-existent data never crashes.
        #[test]
        fn prop_missing_data_returns_none(
            block_number in 1000u64..2000,
            tx_hash_bytes in proptest::collection::vec(any::<u8>(), 32)
        ) {
            let index = PersistentChainIndex::in_memory().unwrap();

            let tx_hash = B256::from_slice(&tx_hash_bytes);

            prop_assert!(index.get_block(block_number).unwrap().is_none());
            prop_assert!(index.get_block_by_hash(tx_hash).unwrap().is_none());
            prop_assert!(index.get_transaction(tx_hash).unwrap().is_none());
            prop_assert!(index.get_receipt(tx_hash).unwrap().is_none());
            prop_assert!(index.get_transaction_location(tx_hash).unwrap().is_none());
            prop_assert!(index.get_logs_by_block(block_number).unwrap().is_empty());
        }

        /// Test that block number lookups are consistent with block storage.
        #[test]
        fn prop_block_number_lookup_consistent(blocks in proptest::collection::vec(0u64..50, 1..10)) {
            let index = PersistentChainIndex::in_memory().unwrap();
            let mut tx_counter = 0u64;

            let mut stored_blocks = Vec::new();
            for &block_number in &blocks {
                if stored_blocks.iter().any(|(n, _)| *n == block_number) {
                    continue;
                }

                let (block, txs, receipts) = generate_block_data(block_number, 0, &mut tx_counter);
                let hash = block.hash;
                index.store_block(block, txs, receipts).unwrap();
                stored_blocks.push((block_number, hash));
            }

            for (number, hash) in &stored_blocks {
                let looked_up_number = index.get_block_number(*hash).unwrap();
                prop_assert_eq!(looked_up_number, Some(*number));

                let block = index.get_block(*number).unwrap().unwrap();
                prop_assert_eq!(block.hash, *hash);

                let block_by_hash = index.get_block_by_hash(*hash).unwrap().unwrap();
                prop_assert_eq!(block_by_hash.number, *number);
            }
        }
    }
}

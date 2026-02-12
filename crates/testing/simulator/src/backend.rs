// Testing code - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]

use commonware_runtime::tokio::{Config as TokioConfig, Runner as TokioRunner};
use commonware_runtime::Runner as RunnerTrait;
use evolve_core::{ErrorCode, ReadonlyKV};
use evolve_stf_traits::StateChange;
use evolve_storage::{CommitHash, MockStorage, Operation, QmdbStorage, Storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use tokio::sync::mpsc::{self as tokio_mpsc, UnboundedSender};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum StorageBackendConfig {
    #[default]
    Mock,
    Qmdb {
        path: Option<String>,
    },
}

impl StorageBackendConfig {
    pub fn from_env() -> Self {
        let raw = std::env::var("EVOLVE_SIM_STORAGE_MODE").unwrap_or_else(|_| "mock".into());
        match raw.trim().to_ascii_lowercase().as_str() {
            "mock" => Self::Mock,
            "qmdb" | "real" => {
                let path = std::env::var("EVOLVE_SIM_QMDB_PATH").ok();
                Self::Qmdb { path }
            }
            _ => Self::Mock,
        }
    }
}

enum QmdbRequest {
    Get {
        key: Vec<u8>,
        response: std_mpsc::Sender<Result<Option<Vec<u8>>, ErrorCode>>,
    },
    Batch {
        ops: Vec<Operation>,
        response: std_mpsc::Sender<Result<(), ErrorCode>>,
    },
    Commit {
        response: std_mpsc::Sender<Result<CommitHash, ErrorCode>>,
    },
}

#[derive(Clone)]
pub(crate) struct QmdbStorageProxy {
    tx: UnboundedSender<QmdbRequest>,
}

impl QmdbStorageProxy {
    fn new(path: PathBuf) -> Self {
        let (tx, mut rx) = tokio_mpsc::unbounded_channel::<QmdbRequest>();
        std::thread::spawn(move || {
            let _ = std::fs::remove_dir_all(&path);
            let _ = std::fs::create_dir_all(&path);
            let runtime_config = TokioConfig::default()
                .with_storage_directory(path.clone())
                .with_worker_threads(2);
            let runner = TokioRunner::new(runtime_config);
            runner.start(move |context| {
                let path = path.clone();
                async move {
                    let storage_config = evolve_storage::StorageConfig {
                        path,
                        ..Default::default()
                    };
                    let storage = QmdbStorage::new(context, storage_config)
                        .await
                        .expect("create qmdb storage");

                    while let Some(req) = rx.recv().await {
                        match req {
                            QmdbRequest::Get { key, response } => {
                                let _ = response.send(storage.get(&key));
                            }
                            QmdbRequest::Batch { ops, response } => {
                                let _ = response.send(storage.batch(ops).await);
                            }
                            QmdbRequest::Commit { response } => {
                                let _ = response.send(storage.commit().await);
                            }
                        }
                    }
                }
            });
        });
        Self { tx }
    }

    fn get_sync(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        let (response_tx, response_rx) = std_mpsc::channel();
        self.tx
            .send(QmdbRequest::Get {
                key: key.to_vec(),
                response: response_tx,
            })
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?;
        response_rx
            .recv()
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?
    }

    fn batch_sync(&self, ops: Vec<Operation>) -> Result<(), ErrorCode> {
        let (response_tx, response_rx) = std_mpsc::channel();
        self.tx
            .send(QmdbRequest::Batch {
                ops,
                response: response_tx,
            })
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?;
        response_rx
            .recv()
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?
    }

    fn commit_sync(&self) -> Result<CommitHash, ErrorCode> {
        let (response_tx, response_rx) = std_mpsc::channel();
        self.tx
            .send(QmdbRequest::Commit {
                response: response_tx,
            })
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?;
        response_rx
            .recv()
            .map_err(|_| evolve_storage::ERR_STORAGE_IO)?
    }
}

impl ReadonlyKV for QmdbStorageProxy {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.get_sync(key)
    }
}

pub(crate) enum StorageBackend {
    Mock(MockStorage),
    Qmdb(QmdbStorageProxy),
}

impl StorageBackend {
    pub fn new(seed: u64, backend_config: StorageBackendConfig) -> Self {
        match backend_config {
            StorageBackendConfig::Mock => Self::Mock(MockStorage::new()),
            StorageBackendConfig::Qmdb { path } => {
                let path = path.map(PathBuf::from).unwrap_or_else(|| {
                    std::env::temp_dir()
                        .join(format!("evolve-sim-qmdb-{}-{seed}", std::process::id()))
                });
                Self::Qmdb(QmdbStorageProxy::new(path))
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        match self {
            Self::Mock(storage) => storage.get(key),
            Self::Qmdb(storage) => storage.get(key),
        }
    }

    pub fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        match self {
            Self::Mock(storage) => {
                let ops = state_changes_to_ops(changes);
                futures::executor::block_on(storage.batch(ops))
            }
            Self::Qmdb(storage) => {
                let ops = state_changes_to_ops(changes);
                storage.batch_sync(ops)
            }
        }
    }

    pub fn commit(&self) -> Result<[u8; 32], ErrorCode> {
        let hash = match self {
            Self::Mock(storage) => futures::executor::block_on(storage.commit())?,
            Self::Qmdb(storage) => storage.commit_sync()?,
        };
        Ok(*hash.as_bytes())
    }
}

pub struct StorageBackendView<'a> {
    backend: &'a StorageBackend,
}

impl<'a> StorageBackendView<'a> {
    pub(crate) fn new(backend: &'a StorageBackend) -> Self {
        Self { backend }
    }
}

impl ReadonlyKV for StorageBackendView<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.backend.get(key)
    }
}

fn state_changes_to_ops(changes: Vec<StateChange>) -> Vec<Operation> {
    changes
        .into_iter()
        .map(|change| match change {
            StateChange::Set { key, value } => Operation::Set { key, value },
            StateChange::Remove { key } => Operation::Remove { key },
        })
        .collect()
}

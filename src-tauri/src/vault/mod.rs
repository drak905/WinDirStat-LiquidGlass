pub mod acl;
pub mod checksum;
pub mod db;
pub mod schema;
pub mod service;

use thiserror::Error;

pub use schema::{
    DEFAULT_MANIFEST_DB_FILENAME, DEFAULT_VAULT_DIR_NAME, EXPIRING_SOON_WINDOW_HOURS,
    QUARANTINE_TTL_HOURS, PurgeItemFailure, PurgeResult, QuarantineBatchResult,
    RollbackBatchResult, RollbackItemResult, VaultBatchMeta, VaultBatchSummary, VaultItemRecord,
    VaultItemStatus,
};
pub use service::{
    ExpirationWorkerHandle, VaultConfig, VaultService, default_manifest_db_path,
    start_expiration_worker, start_expiration_worker_with_interval,
};

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("cannot quarantine an empty item list")]
    EmptyItemList,

    #[error("item does not exist: {path}")]
    ItemNotFound { path: String },

    #[error("path is not a regular file (directories and symlinks are rejected): {path}")]
    NotAFile { path: String },

    #[error("file is locked by another process: {path}")]
    FileLocked {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("destination already exists and cannot be overwritten safely: {path}")]
    DestinationExists { path: String },

    #[error("vault item record not found: {id}")]
    RecordNotFound { id: String },

    #[error("vault batch not found: {batch_id}")]
    BatchNotFound { batch_id: String },

    #[error("vault item already restored: {item_id}")]
    AlreadyRestored { item_id: String },

    #[error("vault item already permanently purged after TTL expiration: {item_id}")]
    AlreadyPurged { item_id: String },

    #[error(
        "checksum mismatch for vault item {item_id} ({algorithm}): expected {expected}, actual {actual}"
    )]
    ChecksumMismatch {
        item_id: String,
        algorithm: &'static str,
        expected: String,
        actual: String,
    },

    #[error("Windows ACL/metadata error for {path}: {message}")]
    AclError { path: String, message: String },

    #[error("invalid UTC timestamp '{value}': {message}")]
    InvalidTimestamp { value: String, message: String },

    #[error("invalid vault item status '{value}'")]
    InvalidStatus { value: String },

    #[error("SQLite database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error ({context}): {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

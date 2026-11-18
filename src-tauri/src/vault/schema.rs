use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::VaultError;

pub const QUARANTINE_TTL_HOURS: i64 = 168;
pub const EXPIRING_SOON_WINDOW_HOURS: i64 = 24;
pub const DEFAULT_VAULT_DIR_NAME: &str = ".WinDirStatVault";
pub const DEFAULT_MANIFEST_DB_FILENAME: &str = "vault_manifest.db";

pub const VAULT_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS vault_batches (
    batch_id TEXT PRIMARY KEY,
    reason TEXT NOT NULL,
    agent_session_id TEXT NOT NULL,
    agent_cli_provider TEXT NOT NULL,
    quarantined_at_utc TEXT NOT NULL,
    expires_at_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vault_items (
    item_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL,
    original_full_path TEXT NOT NULL,
    vault_storage_path TEXT NOT NULL,
    logical_size_bytes INTEGER NOT NULL,
    physical_size_bytes INTEGER NOT NULL,
    xxhash64_checksum TEXT NOT NULL,
    sha256_checksum TEXT NOT NULL,
    ntfs_acl_sddl TEXT NOT NULL,
    original_creation_time TEXT NOT NULL,
    original_modified_time TEXT NOT NULL,
    original_attributes INTEGER NOT NULL,
    quarantined_at_utc TEXT NOT NULL,
    expires_at_utc TEXT NOT NULL,
    agent_cli_provider TEXT NOT NULL,
    agent_rationale_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('QUARANTINED', 'RESTORED', 'PURGED_EXPIRED'))
);

CREATE INDEX IF NOT EXISTS idx_vault_items_batch_id
    ON vault_items(batch_id);

CREATE INDEX IF NOT EXISTS idx_vault_items_status_expires
    ON vault_items(status, expires_at_utc);

CREATE INDEX IF NOT EXISTS idx_vault_items_quarantined_at
    ON vault_items(quarantined_at_utc);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultItemStatus {
    #[serde(rename = "QUARANTINED")]
    Quarantined,
    #[serde(rename = "RESTORED")]
    Restored,
    #[serde(rename = "PURGED_EXPIRED")]
    PurgedExpired,
}

impl VaultItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quarantined => "QUARANTINED",
            Self::Restored => "RESTORED",
            Self::PurgedExpired => "PURGED_EXPIRED",
        }
    }
}

impl fmt::Display for VaultItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for VaultItemStatus {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "QUARANTINED" => Ok(Self::Quarantined),
            "RESTORED" => Ok(Self::Restored),
            "PURGED_EXPIRED" => Ok(Self::PurgedExpired),
            _ => Err(VaultError::InvalidStatus {
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultItemRecord {
    pub item_id: String,
    pub batch_id: String,
    pub original_full_path: String,
    pub vault_storage_path: String,
    pub logical_size_bytes: i64,
    pub physical_size_bytes: i64,
    pub xxhash64_checksum: String,
    pub sha256_checksum: String,
    pub ntfs_acl_sddl: String,
    pub original_creation_time: String,
    pub original_modified_time: String,
    pub original_attributes: u32,
    pub quarantined_at_utc: String,
    pub expires_at_utc: String,
    pub agent_cli_provider: String,
    pub agent_rationale_json: String,
    pub status: VaultItemStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultBatchMeta {
    pub batch_id: String,
    pub reason: String,
    pub agent_session_id: String,
    pub agent_cli_provider: String,
    pub quarantined_at_utc: String,
    pub expires_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultBatchSummary {
    pub batch_id: String,
    pub reason: String,
    pub agent_session_id: String,
    pub agent_cli_provider: String,
    pub quarantined_at_utc: String,
    pub expires_at_utc: String,
    pub total_items: usize,
    pub quarantined_count: usize,
    pub restored_count: usize,
    pub purged_count: usize,
    pub total_logical_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineBatchResult {
    pub batch_id: String,
    pub quarantined_at_utc: DateTime<Utc>,
    pub expires_at_utc: DateTime<Utc>,
    pub items: Vec<VaultItemRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackItemResult {
    pub item_id: String,
    pub batch_id: String,
    pub original_full_path: String,
    pub vault_storage_path: String,
    pub logical_size_bytes: i64,
    pub xxhash64_checksum: String,
    pub sha256_checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackBatchResult {
    pub restored: Vec<RollbackItemResult>,
    pub skipped_already_restored: Vec<String>,
    pub failed_purged: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeItemFailure {
    pub item_id: String,
    pub vault_storage_path: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeResult {
    pub checked_at_utc: DateTime<Utc>,
    pub purged_item_ids: Vec<String>,
    pub purged_count: usize,
    pub freed_logical_bytes: i64,
    pub freed_physical_bytes: i64,
    pub failures: Vec<PurgeItemFailure>,
}

/// Formats UTC timestamps with fixed-width 9-digit nanoseconds so SQLite text
/// sorting matches chronological UTC order identically.
pub fn format_utc(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub fn parse_utc(raw: &str) -> Result<DateTime<Utc>, VaultError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| VaultError::InvalidTimestamp {
            value: raw.to_string(),
            message: err.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn status_round_trip_and_invalid_rejected() {
        for status in [
            VaultItemStatus::Quarantined,
            VaultItemStatus::Restored,
            VaultItemStatus::PurgedExpired,
        ] {
            let s = status.as_str();
            assert_eq!(VaultItemStatus::from_str(s).unwrap(), status);
        }
        assert!(matches!(
            VaultItemStatus::from_str("UNKNOWN"),
            Err(VaultError::InvalidStatus { .. })
        ));
    }

    #[test]
    fn formatted_utc_timestamps_preserve_nanos_and_lex_order() {
        let t1 = Utc.with_ymd_and_hms(2026, 11, 18, 13, 0, 0).unwrap();
        let t2 = t1 + chrono::Duration::nanoseconds(500);
        let s1 = format_utc(t1);
        let s2 = format_utc(t2);
        assert_eq!(s1.len(), s2.len());
        assert!(s1 < s2);
        assert_eq!(parse_utc(&s1).unwrap(), t1);
        assert_eq!(parse_utc(&s2).unwrap(), t2);
    }
}

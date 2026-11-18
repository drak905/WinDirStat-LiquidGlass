use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use super::schema::{
    parse_utc, VaultBatchMeta, VaultBatchSummary, VaultItemRecord, VaultItemStatus,
    VAULT_SCHEMA_SQL,
};
use super::VaultError;

const VAULT_ITEM_SELECT_COLUMNS: &str = "\
    item_id, batch_id, original_full_path, vault_storage_path, \
    logical_size_bytes, physical_size_bytes, xxhash64_checksum, sha256_checksum, \
    ntfs_acl_sddl, original_creation_time, original_modified_time, original_attributes, \
    quarantined_at_utc, expires_at_utc, agent_cli_provider, agent_rationale_json, status";

pub struct VaultDb {
    conn: Mutex<Connection>,
}

impl VaultDb {
    pub fn open(db_path: &Path) -> Result<Self, VaultError> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|err| VaultError::Io {
                    context: format!("creating manifest DB parent directory '{}'", parent.display()),
                    source: err,
                })?;
            }
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;\
             PRAGMA synchronous = FULL;\
             PRAGMA busy_timeout = 5000;\
             PRAGMA foreign_keys = ON;",
        )?;
        conn.execute_batch(VAULT_SCHEMA_SQL)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert_batch_and_items(
        &self,
        batch: &VaultBatchMeta,
        items: &[VaultItemRecord],
    ) -> Result<(), VaultError> {
        let mut conn = self.lock_conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO vault_batches (
                batch_id, reason, agent_session_id, agent_cli_provider,
                quarantined_at_utc, expires_at_utc
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                batch.batch_id,
                batch.reason,
                batch.agent_session_id,
                batch.agent_cli_provider,
                batch.quarantined_at_utc,
                batch.expires_at_utc,
            ],
        )?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO vault_items (
                    item_id, batch_id, original_full_path, vault_storage_path,
                    logical_size_bytes, physical_size_bytes, xxhash64_checksum, sha256_checksum,
                    ntfs_acl_sddl, original_creation_time, original_modified_time,
                    original_attributes, quarantined_at_utc, expires_at_utc,
                    agent_cli_provider, agent_rationale_json, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;

            for item in items {
                stmt.execute(params![
                    item.item_id,
                    item.batch_id,
                    item.original_full_path,
                    item.vault_storage_path,
                    item.logical_size_bytes,
                    item.physical_size_bytes,
                    item.xxhash64_checksum,
                    item.sha256_checksum,
                    item.ntfs_acl_sddl,
                    item.original_creation_time,
                    item.original_modified_time,
                    i64::from(item.original_attributes),
                    item.quarantined_at_utc,
                    item.expires_at_utc,
                    item.agent_cli_provider,
                    item.agent_rationale_json,
                    item.status.as_str(),
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_item(&self, item_id: &str) -> Result<Option<VaultItemRecord>, VaultError> {
        let conn = self.lock_conn();
        let sql = format!("SELECT {VAULT_ITEM_SELECT_COLUMNS} FROM vault_items WHERE item_id = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let item = stmt.query_row([item_id], map_item_row).optional()?;
        match item {
            Some(Ok(record)) => Ok(Some(record)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    pub fn get_batch_items(&self, batch_id: &str) -> Result<Vec<VaultItemRecord>, VaultError> {
        let conn = self.lock_conn();
        let sql = format!(
            "SELECT {VAULT_ITEM_SELECT_COLUMNS} FROM vault_items WHERE batch_id = ?1 ORDER BY item_id ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([batch_id], map_item_row)?;
        collect_item_rows(rows)
    }

    pub fn batch_exists(&self, batch_id: &str) -> Result<bool, VaultError> {
        let conn = self.lock_conn();
        let exists: i64 = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM vault_items WHERE batch_id = ?1
                UNION ALL
                SELECT 1 FROM vault_batches WHERE batch_id = ?1
            )",
            [batch_id],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    pub fn update_item_status(
        &self,
        item_id: &str,
        expected_current: VaultItemStatus,
        new_status: VaultItemStatus,
    ) -> Result<bool, VaultError> {
        let mut conn = self.lock_conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE vault_items SET status = ?1 WHERE item_id = ?2 AND status = ?3",
            params![new_status.as_str(), item_id, expected_current.as_str()],
        )?;
        tx.commit()?;
        Ok(updated == 1)
    }

    pub fn list_quarantined_items(&self) -> Result<Vec<VaultItemRecord>, VaultError> {
        let conn = self.lock_conn();
        let sql = format!(
            "SELECT {VAULT_ITEM_SELECT_COLUMNS} FROM vault_items WHERE status = 'QUARANTINED' ORDER BY quarantined_at_utc ASC, item_id ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_item_row)?;
        collect_item_rows(rows)
    }

    pub fn list_items_since(&self, since_utc: DateTime<Utc>) -> Result<Vec<VaultItemRecord>, VaultError> {
        let conn = self.lock_conn();
        let sql = format!(
            "SELECT {VAULT_ITEM_SELECT_COLUMNS} FROM vault_items ORDER BY quarantined_at_utc DESC, item_id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let all = collect_item_rows(stmt.query_map([], map_item_row)?)?;
        let mut filtered = Vec::new();
        for item in all {
            if parse_utc(&item.quarantined_at_utc)? >= since_utc {
                filtered.push(item);
            }
        }
        Ok(filtered)
    }

    pub fn list_batches_since(
        &self,
        since_utc: DateTime<Utc>,
    ) -> Result<Vec<VaultBatchSummary>, VaultError> {
        let items = self.list_items_since(since_utc)?;
        let meta_map = self.load_batch_meta_map()?;

        let mut grouped: BTreeMap<String, Vec<VaultItemRecord>> = BTreeMap::new();
        for item in items {
            grouped.entry(item.batch_id.clone()).or_default().push(item);
        }

        let mut summaries = Vec::with_capacity(grouped.len());
        for (batch_id, group) in grouped {
            let meta = meta_map.get(&batch_id);
            let first = &group[0];

            let mut quarantined_count = 0usize;
            let mut restored_count = 0usize;
            let mut purged_count = 0usize;
            let mut total_logical_bytes = 0i64;
            for item in &group {
                total_logical_bytes = total_logical_bytes.saturating_add(item.logical_size_bytes);
                match item.status {
                    VaultItemStatus::Quarantined => quarantined_count += 1,
                    VaultItemStatus::Restored => restored_count += 1,
                    VaultItemStatus::PurgedExpired => purged_count += 1,
                }
            }

            summaries.push(VaultBatchSummary {
                batch_id: batch_id.clone(),
                reason: meta.map(|m| m.reason.clone()).unwrap_or_default(),
                agent_session_id: meta.map(|m| m.agent_session_id.clone()).unwrap_or_default(),
                agent_cli_provider: meta
                    .map(|m| m.agent_cli_provider.clone())
                    .unwrap_or_else(|| first.agent_cli_provider.clone()),
                quarantined_at_utc: meta
                    .map(|m| m.quarantined_at_utc.clone())
                    .unwrap_or_else(|| first.quarantined_at_utc.clone()),
                expires_at_utc: meta
                    .map(|m| m.expires_at_utc.clone())
                    .unwrap_or_else(|| first.expires_at_utc.clone()),
                total_items: group.len(),
                quarantined_count,
                restored_count,
                purged_count,
                total_logical_bytes,
            });
        }

        summaries.sort_by(|a, b| b.quarantined_at_utc.cmp(&a.quarantined_at_utc));
        Ok(summaries)
    }

    pub fn list_columns(&self) -> Result<Vec<String>, VaultError> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("PRAGMA table_info(vault_items)")?;
        let cols = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(cols)
    }

    fn load_batch_meta_map(&self) -> Result<BTreeMap<String, VaultBatchMeta>, VaultError> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT batch_id, reason, agent_session_id, agent_cli_provider, quarantined_at_utc, expires_at_utc FROM vault_batches",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(VaultBatchMeta {
                batch_id: row.get(0)?,
                reason: row.get(1)?,
                agent_session_id: row.get(2)?,
                agent_cli_provider: row.get(3)?,
                quarantined_at_utc: row.get(4)?,
                expires_at_utc: row.get(5)?,
            })
        })?;
        let mut map = BTreeMap::new();
        for row in rows {
            let meta = row?;
            map.insert(meta.batch_id.clone(), meta);
        }
        Ok(map)
    }

    fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn map_item_row(row: &Row<'_>) -> rusqlite::Result<Result<VaultItemRecord, VaultError>> {
    let attrs_i64: i64 = row.get(11)?;
    let status_str: String = row.get(16)?;

    let parsed = (|| -> Result<VaultItemRecord, VaultError> {
        let status = VaultItemStatus::from_str(&status_str)?;
        let original_attributes = u32::try_from(attrs_i64).map_err(|_| {
            VaultError::InvalidArgument(format!("original_attributes out of u32 range: {attrs_i64}"))
        })?;
        Ok(VaultItemRecord {
            item_id: row.get(0)?,
            batch_id: row.get(1)?,
            original_full_path: row.get(2)?,
            vault_storage_path: row.get(3)?,
            logical_size_bytes: row.get(4)?,
            physical_size_bytes: row.get(5)?,
            xxhash64_checksum: row.get(6)?,
            sha256_checksum: row.get(7)?,
            ntfs_acl_sddl: row.get(8)?,
            original_creation_time: row.get(9)?,
            original_modified_time: row.get(10)?,
            original_attributes,
            quarantined_at_utc: row.get(12)?,
            expires_at_utc: row.get(13)?,
            agent_cli_provider: row.get(14)?,
            agent_rationale_json: row.get(15)?,
            status,
        })
    })();

    Ok(parsed)
}

fn collect_item_rows<I>(iter: I) -> Result<Vec<VaultItemRecord>, VaultError>
where
    I: Iterator<Item = rusqlite::Result<Result<VaultItemRecord, VaultError>>>,
{
    let mut items = Vec::new();
    for row in iter {
        items.push(row??);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::schema::format_utc;
    use chrono::TimeZone;
    use tempfile::tempdir;

    #[test]
    fn initializes_exact_vault_items_schema_and_persists_records() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("vault_manifest.db");
        let db = VaultDb::open(&db_path).unwrap();

        let cols = db.list_columns().unwrap();
        assert_eq!(
            cols,
            vec![
                "item_id",
                "batch_id",
                "original_full_path",
                "vault_storage_path",
                "logical_size_bytes",
                "physical_size_bytes",
                "xxhash64_checksum",
                "sha256_checksum",
                "ntfs_acl_sddl",
                "original_creation_time",
                "original_modified_time",
                "original_attributes",
                "quarantined_at_utc",
                "expires_at_utc",
                "agent_cli_provider",
                "agent_rationale_json",
                "status",
            ]
        );

        let now = Utc.with_ymd_and_hms(2026, 11, 18, 13, 0, 0).unwrap();
        let expires = now + chrono::Duration::hours(168);
        let batch = VaultBatchMeta {
            batch_id: "01933f10-0000-7000-8000-000000000001".to_string(),
            reason: "cache cleanup".to_string(),
            agent_session_id: "sess-1".to_string(),
            agent_cli_provider: "codex".to_string(),
            quarantined_at_utc: format_utc(now),
            expires_at_utc: format_utc(expires),
        };
        let item = VaultItemRecord {
            item_id: "01933f10-0000-7000-8000-000000000002".to_string(),
            batch_id: batch.batch_id.clone(),
            original_full_path: r"C:\temp\a.log".to_string(),
            vault_storage_path: r"C:\.WinDirStatVault\objects\b\a.log".to_string(),
            logical_size_bytes: 128,
            physical_size_bytes: 128,
            xxhash64_checksum: "0123456789abcdef".to_string(),
            sha256_checksum: "a".repeat(64),
            ntfs_acl_sddl: "O:BAG:BAD:(A;;FA;;;SY)".to_string(),
            original_creation_time: format_utc(now),
            original_modified_time: format_utc(now),
            original_attributes: 0x20,
            quarantined_at_utc: format_utc(now),
            expires_at_utc: format_utc(expires),
            agent_cli_provider: "codex".to_string(),
            agent_rationale_json: r#"{"confidence":0.99}"#.to_string(),
            status: VaultItemStatus::Quarantined,
        };

        db.insert_batch_and_items(&batch, std::slice::from_ref(&item))
            .unwrap();
        let loaded = db.get_item(&item.item_id).unwrap().unwrap();
        assert_eq!(loaded, item);

        assert!(db
            .update_item_status(
                &item.item_id,
                VaultItemStatus::Quarantined,
                VaultItemStatus::Restored
            )
            .unwrap());
        let after = db.get_item(&item.item_id).unwrap().unwrap();
        assert_eq!(after.status, VaultItemStatus::Restored);
    }
}

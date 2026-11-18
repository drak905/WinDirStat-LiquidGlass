use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use uuid::Uuid;

use super::acl;
use super::checksum;
use super::db::VaultDb;
use super::schema::{
    format_utc, parse_utc, PurgeItemFailure, PurgeResult, QuarantineBatchResult,
    RollbackBatchResult, RollbackItemResult, VaultBatchMeta, VaultBatchSummary, VaultItemRecord,
    VaultItemStatus, DEFAULT_MANIFEST_DB_FILENAME, DEFAULT_VAULT_DIR_NAME,
    EXPIRING_SOON_WINDOW_HOURS, QUARANTINE_TTL_HOURS,
};
use super::VaultError;

const EXPIRATION_CHECK_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct VaultConfig {
    pub manifest_db_path: PathBuf,
    pub vault_root_override: Option<PathBuf>,
    pub enable_ntfs_compression: bool,
}

impl VaultConfig {
    pub fn default_standard() -> Self {
        Self {
            manifest_db_path: default_manifest_db_path(),
            vault_root_override: None,
            enable_ntfs_compression: true,
        }
    }

    pub fn with_custom_root(
        db_path: impl Into<PathBuf>,
        vault_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            manifest_db_path: db_path.into(),
            vault_root_override: Some(vault_root.into()),
            enable_ntfs_compression: true,
        }
    }
}

pub fn default_manifest_db_path() -> PathBuf {
    if let Some(env_path) = env::var_os("WDS_VAULT_DB") {
        if !env_path.is_empty() {
            return PathBuf::from(env_path);
        }
    }
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        if !local_app_data.is_empty() {
            return PathBuf::from(local_app_data)
                .join("WinDirStat")
                .join(DEFAULT_MANIFEST_DB_FILENAME);
        }
    }
    if let Some(program_data) = env::var_os("ProgramData") {
        if !program_data.is_empty() {
            return PathBuf::from(program_data)
                .join("WinDirStat")
                .join(DEFAULT_MANIFEST_DB_FILENAME);
        }
    }
    PathBuf::from(DEFAULT_MANIFEST_DB_FILENAME)
}

pub struct VaultService {
    config: VaultConfig,
    db: VaultDb,
}

struct PreparedQuarantineItem {
    original_path: PathBuf,
    vault_path: PathBuf,
    record: VaultItemRecord,
}

impl VaultService {
    pub fn new(config: VaultConfig) -> Result<Self, VaultError> {
        let db = VaultDb::open(&config.manifest_db_path)?;
        Ok(Self { config, db })
    }

    pub fn open_default() -> Result<Self, VaultError> {
        Self::new(VaultConfig::default_standard())
    }

    pub fn with_db_path(db_path: impl Into<PathBuf>) -> Result<Self, VaultError> {
        Self::new(VaultConfig {
            manifest_db_path: db_path.into(),
            vault_root_override: None,
            enable_ntfs_compression: true,
        })
    }

    pub fn with_custom_root(
        db_path: impl Into<PathBuf>,
        vault_root: impl Into<PathBuf>,
    ) -> Result<Self, VaultError> {
        Self::new(VaultConfig::with_custom_root(db_path, vault_root))
    }

    pub fn db(&self) -> &VaultDb {
        &self.db
    }

    pub fn quarantine_items<I, P>(
        &self,
        items: I,
        reason: &str,
        agent_session_id: &str,
        provider: &str,
        rationale_json: &str,
        now_utc: DateTime<Utc>,
    ) -> Result<QuarantineBatchResult, VaultError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut unique_paths = Vec::new();
        let mut seen = HashSet::new();
        for item in items {
            let abs = std::path::absolute(item.as_ref()).map_err(|err| VaultError::Io {
                context: format!(
                    "resolving absolute path for '{}'",
                    item.as_ref().display()
                ),
                source: err,
            })?;
            let dedup_key = abs.to_string_lossy().to_lowercase();
            if seen.insert(dedup_key) {
                unique_paths.push(abs);
            }
        }

        if unique_paths.is_empty() {
            return Err(VaultError::EmptyItemList);
        }

        let batch_id = Uuid::now_v7().to_string();
        let expires_at_utc = now_utc + ChronoDuration::hours(QUARANTINE_TTL_HOURS);
        let quarantined_str = format_utc(now_utc);
        let expires_str = format_utc(expires_at_utc);

        // Phase 1: Preflight lock detection, checksums, SDDL ACL, and timestamps.
        // Zero filesystem or DB mutations occur during Phase 1.
        let mut prepared = Vec::with_capacity(unique_paths.len());
        for original_path in unique_paths {
            let file = acl::ensure_not_locked_and_open_read(&original_path)?;
            let snap = acl::capture_security_snapshot_from_handle(&original_path, &file)?;
            let digests = checksum::compute_reader_checksums(file)?;

            let item_id = Uuid::now_v7().to_string();
            let vault_dir = self.resolve_batch_objects_dir(&original_path, &batch_id)?;
            let vault_filename = build_vault_filename(&item_id, &original_path);
            let vault_path = vault_dir.join(vault_filename);

            let logical_i64 = i64::try_from(snap.logical_size_bytes).map_err(|_| {
                VaultError::InvalidArgument(format!(
                    "logical_size_bytes exceeds i64 range for '{}'",
                    original_path.display()
                ))
            })?;
            let physical_i64 = i64::try_from(snap.physical_size_bytes).map_err(|_| {
                VaultError::InvalidArgument(format!(
                    "physical_size_bytes exceeds i64 range for '{}'",
                    original_path.display()
                ))
            })?;

            let record = VaultItemRecord {
                item_id,
                batch_id: batch_id.clone(),
                original_full_path: original_path.to_string_lossy().into_owned(),
                vault_storage_path: vault_path.to_string_lossy().into_owned(),
                logical_size_bytes: logical_i64,
                physical_size_bytes: physical_i64,
                xxhash64_checksum: digests.xxhash64,
                sha256_checksum: digests.sha256,
                ntfs_acl_sddl: snap.ntfs_acl_sddl,
                original_creation_time: format_utc(snap.creation_time_utc),
                original_modified_time: format_utc(snap.modified_time_utc),
                original_attributes: snap.attributes,
                quarantined_at_utc: quarantined_str.clone(),
                expires_at_utc: expires_str.clone(),
                agent_cli_provider: provider.to_string(),
                agent_rationale_json: rationale_json.to_string(),
                status: VaultItemStatus::Quarantined,
            };

            prepared.push(PreparedQuarantineItem {
                original_path,
                vault_path,
                record,
            });
        }

        // Phase 2: Move each item atomically into the vault; roll back all moves on any error.
        let allow_cross_vol = self.config.vault_root_override.is_some();
        let mut moved_pairs: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(prepared.len());
        let mut created_dirs: BTreeSet<PathBuf> = BTreeSet::new();

        for item in &prepared {
            if let Some(parent) = item.vault_path.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    rollback_moved_pairs(&moved_pairs, &created_dirs, allow_cross_vol);
                    return Err(VaultError::Io {
                        context: format!(
                            "creating quarantine directory '{}'",
                            parent.display()
                        ),
                        source: err,
                    });
                }
                created_dirs.insert(parent.to_path_buf());
            }

            if let Err(err) = acl::atomic_move_file(&item.original_path, &item.vault_path, allow_cross_vol) {
                rollback_moved_pairs(&moved_pairs, &created_dirs, allow_cross_vol);
                return Err(err);
            }
            moved_pairs.push((item.original_path.clone(), item.vault_path.clone()));
        }

        // Phase 3: Best-effort NTFS compression on quarantined files.
        if self.config.enable_ntfs_compression {
            for (_, vault_path) in &moved_pairs {
                let _ = acl::try_set_ntfs_compression(vault_path, true);
            }
        }

        // Phase 4: Transactional SQLite manifest commit; roll back moves if DB fails.
        let batch_meta = VaultBatchMeta {
            batch_id: batch_id.clone(),
            reason: reason.to_string(),
            agent_session_id: agent_session_id.to_string(),
            agent_cli_provider: provider.to_string(),
            quarantined_at_utc: quarantined_str,
            expires_at_utc: expires_str,
        };
        let records: Vec<VaultItemRecord> = prepared.into_iter().map(|p| p.record).collect();

        if let Err(err) = self.db.insert_batch_and_items(&batch_meta, &records) {
            rollback_moved_pairs(&moved_pairs, &created_dirs, allow_cross_vol);
            return Err(err);
        }

        Ok(QuarantineBatchResult {
            batch_id,
            quarantined_at_utc: now_utc,
            expires_at_utc,
            items: records,
        })
    }

    pub fn rollback_item(&self, item_id: &str) -> Result<RollbackItemResult, VaultError> {
        let record = self
            .db
            .get_item(item_id)?
            .ok_or_else(|| VaultError::RecordNotFound {
                id: item_id.to_string(),
            })?;
        self.rollback_item_record(&record)
    }

    pub fn rollback_batch(&self, batch_id: &str) -> Result<RollbackBatchResult, VaultError> {
        let items = self.db.get_batch_items(batch_id)?;
        if items.is_empty() && !self.db.batch_exists(batch_id)? {
            return Err(VaultError::BatchNotFound {
                batch_id: batch_id.to_string(),
            });
        }

        let result = self.rollback_item_list(items)?;
        if result.restored.is_empty()
            && result.skipped_already_restored.is_empty()
            && !result.failed_purged.is_empty()
        {
            return Err(VaultError::AlreadyPurged {
                item_id: result.failed_purged[0].clone(),
            });
        }

        Ok(result)
    }

    pub fn rollback_all_recent(&self, days: i64) -> Result<RollbackBatchResult, VaultError> {
        self.rollback_all_recent_at(days, Utc::now())
    }

    pub fn rollback_all_recent_at(
        &self,
        days: i64,
        now_utc: DateTime<Utc>,
    ) -> Result<RollbackBatchResult, VaultError> {
        if days < 0 {
            return Err(VaultError::InvalidArgument(format!(
                "days must be non-negative, got {days}"
            )));
        }

        let since = now_utc - ChronoDuration::days(days);
        let items = self.db.list_items_since(since)?;
        self.rollback_item_list(items)
    }

    fn rollback_item_list(
        &self,
        items: Vec<VaultItemRecord>,
    ) -> Result<RollbackBatchResult, VaultError> {
        // Pre-verify every QUARANTINED item before moving anything so checksum
        // tampering or destination collisions abort without partial batch restores.
        for item in &items {
            if item.status == VaultItemStatus::Quarantined {
                self.preflight_rollback_item(item)?;
            }
        }

        let mut result = RollbackBatchResult {
            restored: Vec::new(),
            skipped_already_restored: Vec::new(),
            failed_purged: Vec::new(),
        };

        for item in items {
            match item.status {
                VaultItemStatus::Quarantined => {
                    result.restored.push(self.rollback_item_record(&item)?);
                }
                VaultItemStatus::Restored => {
                    result.skipped_already_restored.push(item.item_id);
                }
                VaultItemStatus::PurgedExpired => {
                    result.failed_purged.push(item.item_id);
                }
            }
        }

        Ok(result)
    }

    pub fn purge_expired(&self, current_utc: DateTime<Utc>) -> Result<PurgeResult, VaultError> {
        let quarantined = self.db.list_quarantined_items()?;
        let mut purged_item_ids = Vec::new();
        let mut freed_logical_bytes = 0i64;
        let mut freed_physical_bytes = 0i64;
        let mut failures = Vec::new();

        for item in quarantined {
            let expires_at = parse_utc(&item.expires_at_utc)?;
            if current_utc < expires_at {
                continue;
            }

            let vault_path = PathBuf::from(&item.vault_storage_path);
            match acl::clear_readonly_and_remove_file(&vault_path) {
                Ok(()) => {
                    self.db.update_item_status(
                        &item.item_id,
                        VaultItemStatus::Quarantined,
                        VaultItemStatus::PurgedExpired,
                    )?;
                    cleanup_empty_parent_dir(&vault_path);
                    purged_item_ids.push(item.item_id);
                    freed_logical_bytes = freed_logical_bytes.saturating_add(item.logical_size_bytes);
                    freed_physical_bytes =
                        freed_physical_bytes.saturating_add(item.physical_size_bytes);
                }
                Err(err) => {
                    failures.push(PurgeItemFailure {
                        item_id: item.item_id,
                        vault_storage_path: item.vault_storage_path,
                        error: err.to_string(),
                    });
                }
            }
        }

        let purged_count = purged_item_ids.len();
        Ok(PurgeResult {
            checked_at_utc: current_utc,
            purged_item_ids,
            purged_count,
            freed_logical_bytes,
            freed_physical_bytes,
            failures,
        })
    }

    pub fn items_expiring_within_24h(
        &self,
        current_utc: DateTime<Utc>,
    ) -> Result<Vec<VaultItemRecord>, VaultError> {
        let horizon = current_utc + ChronoDuration::hours(EXPIRING_SOON_WINDOW_HOURS);
        let quarantined = self.db.list_quarantined_items()?;
        let mut expiring = Vec::new();

        for item in quarantined {
            let expires_at = parse_utc(&item.expires_at_utc)?;
            if expires_at > current_utc && expires_at <= horizon {
                expiring.push(item);
            }
        }

        Ok(expiring)
    }

    pub fn list_items_last_days(
        &self,
        days: i64,
        now_utc: DateTime<Utc>,
    ) -> Result<Vec<VaultItemRecord>, VaultError> {
        if days < 0 {
            return Err(VaultError::InvalidArgument(format!(
                "days must be non-negative, got {days}"
            )));
        }
        self.db.list_items_since(now_utc - ChronoDuration::days(days))
    }

    pub fn list_batches_last_days(
        &self,
        days: i64,
        now_utc: DateTime<Utc>,
    ) -> Result<Vec<VaultBatchSummary>, VaultError> {
        if days < 0 {
            return Err(VaultError::InvalidArgument(format!(
                "days must be non-negative, got {days}"
            )));
        }
        self.db
            .list_batches_since(now_utc - ChronoDuration::days(days))
    }

    pub fn get_item(&self, item_id: &str) -> Result<Option<VaultItemRecord>, VaultError> {
        self.db.get_item(item_id)
    }

    pub fn get_batch_items(&self, batch_id: &str) -> Result<Vec<VaultItemRecord>, VaultError> {
        self.db.get_batch_items(batch_id)
    }

    fn resolve_batch_objects_dir(
        &self,
        original_path: &Path,
        batch_id: &str,
    ) -> Result<PathBuf, VaultError> {
        let vault_root = match &self.config.vault_root_override {
            Some(override_root) => override_root.clone(),
            None => acl::get_volume_root(original_path)?.join(DEFAULT_VAULT_DIR_NAME),
        };
        Ok(vault_root.join("objects").join(batch_id))
    }

    fn preflight_rollback_item(&self, item: &VaultItemRecord) -> Result<(), VaultError> {
        match item.status {
            VaultItemStatus::Restored => {
                return Err(VaultError::AlreadyRestored {
                    item_id: item.item_id.clone(),
                });
            }
            VaultItemStatus::PurgedExpired => {
                return Err(VaultError::AlreadyPurged {
                    item_id: item.item_id.clone(),
                });
            }
            VaultItemStatus::Quarantined => {}
        }

        let vault_path = PathBuf::from(&item.vault_storage_path);
        let original_path = PathBuf::from(&item.original_full_path);

        if !vault_path.exists() {
            return Err(VaultError::ItemNotFound {
                path: item.vault_storage_path.clone(),
            });
        }

        // Verify bit-exact xxHash64 and SHA-256 BEFORE touching original location
        checksum::verify_file_checksums(
            &vault_path,
            &item.xxhash64_checksum,
            &item.sha256_checksum,
            &item.item_id,
        )?;

        if original_path.symlink_metadata().is_ok() {
            return Err(VaultError::DestinationExists {
                path: item.original_full_path.clone(),
            });
        }

        Ok(())
    }

    fn rollback_item_record(&self, item: &VaultItemRecord) -> Result<RollbackItemResult, VaultError> {
        self.preflight_rollback_item(item)?;

        let vault_path = PathBuf::from(&item.vault_storage_path);
        let original_path = PathBuf::from(&item.original_full_path);

        if let Some(parent) = original_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|err| VaultError::Io {
                    context: format!(
                        "recreating original parent directory '{}'",
                        parent.display()
                    ),
                    source: err,
                })?;
            }
        }

        let ctime = parse_utc(&item.original_creation_time)?;
        let mtime = parse_utc(&item.original_modified_time)?;
        let allow_cross_vol = self.config.vault_root_override.is_some();

        acl::atomic_move_file(&vault_path, &original_path, allow_cross_vol)?;

        if let Err(err) = acl::restore_file_metadata_and_acl(
            &original_path,
            &item.ntfs_acl_sddl,
            ctime,
            mtime,
            item.original_attributes,
        ) {
            // Keep disk and DB consistent if metadata restoration fails
            let _ = acl::atomic_move_file(&original_path, &vault_path, allow_cross_vol);
            return Err(err);
        }

        self.db.update_item_status(
            &item.item_id,
            VaultItemStatus::Quarantined,
            VaultItemStatus::Restored,
        )?;
        cleanup_empty_parent_dir(&vault_path);

        Ok(RollbackItemResult {
            item_id: item.item_id.clone(),
            batch_id: item.batch_id.clone(),
            original_full_path: item.original_full_path.clone(),
            vault_storage_path: item.vault_storage_path.clone(),
            logical_size_bytes: item.logical_size_bytes,
            xxhash64_checksum: item.xxhash64_checksum.clone(),
            sha256_checksum: item.sha256_checksum.clone(),
        })
    }
}

pub struct ExpirationWorkerHandle {
    shutdown: Arc<(Mutex<bool>, Condvar)>,
    join_handle: Option<JoinHandle<()>>,
}

impl ExpirationWorkerHandle {
    pub fn stop(mut self) {
        self.signal_and_join();
    }

    fn signal_and_join(&mut self) {
        let (lock, cvar) = &*self.shutdown;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
            cvar.notify_all();
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ExpirationWorkerHandle {
    fn drop(&mut self) {
        self.signal_and_join();
    }
}

pub fn start_expiration_worker(service: Arc<VaultService>) -> ExpirationWorkerHandle {
    start_expiration_worker_with_interval(service, EXPIRATION_CHECK_INTERVAL)
}

pub fn start_expiration_worker_with_interval(
    service: Arc<VaultService>,
    interval: Duration,
) -> ExpirationWorkerHandle {
    let shutdown = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_shutdown = Arc::clone(&shutdown);

    let join_handle = thread::Builder::new()
        .name("wds-vault-expiration-worker".to_string())
        .spawn(move || {
            let (lock, cvar) = &*worker_shutdown;
            loop {
                let _ = service.purge_expired(Utc::now());

                let guard = lock.lock().unwrap_or_else(|p| p.into_inner());
                if *guard {
                    break;
                }
                let (guard, _) = cvar
                    .wait_timeout_while(guard, interval, |stopped| !*stopped)
                    .unwrap_or_else(|p| p.into_inner());
                if *guard {
                    break;
                }
            }
        })
        .expect("failed to spawn vault expiration worker thread");

    ExpirationWorkerHandle {
        shutdown,
        join_handle: Some(join_handle),
    }
}

fn build_vault_filename(item_id: &str, original_path: &Path) -> String {
    let raw_name = original_path
        .file_name()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let sanitized: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    if sanitized.is_empty() {
        item_id.to_string()
    } else {
        format!("{item_id}_{sanitized}")
    }
}

fn rollback_moved_pairs(
    moved_pairs: &[(PathBuf, PathBuf)],
    created_dirs: &BTreeSet<PathBuf>,
    allow_cross_vol: bool,
) {
    for (orig_path, vault_path) in moved_pairs.iter().rev() {
        let _ = acl::atomic_move_file(vault_path, orig_path, allow_cross_vol);
    }
    for dir in created_dirs.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
}

fn cleanup_empty_parent_dir(file_path: &Path) {
    if let Some(parent) = file_path.parent() {
        let _ = fs::remove_dir(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::schema::VaultItemStatus;
    use chrono::TimeZone;
    use tempfile::tempdir;

    #[test]
    fn expiring_within_24h_toast_window_is_exact() {
        let dir = tempdir().unwrap();
        let svc = VaultService::with_custom_root(
            dir.path().join("vault_manifest.db"),
            dir.path().join("vault"),
        )
        .unwrap();
        let file = dir.path().join("toast.log");
        std::fs::write(&file, b"toast data").unwrap();

        let t0 = Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap();
        let batch = svc
            .quarantine_items(
                [&file],
                "old log",
                "sess-toast",
                "claude",
                r#"{"safe":true}"#,
                t0,
            )
            .unwrap();
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].status, VaultItemStatus::Quarantined);

        // 24h01m before expiry -> NOT in 24h warning window
        let before_window = t0 + chrono::Duration::hours(143) + chrono::Duration::minutes(59);
        assert!(svc.items_expiring_within_24h(before_window).unwrap().is_empty());

        // Exactly 24h00m before expiry -> IN 24h warning window
        let at_window = t0 + chrono::Duration::hours(144);
        assert_eq!(svc.items_expiring_within_24h(at_window).unwrap().len(), 1);

        // 168h00m -> already expired, belongs to purge_expired
        let at_expiry = t0 + chrono::Duration::hours(168);
        assert!(svc.items_expiring_within_24h(at_expiry).unwrap().is_empty());
    }

    #[test]
    fn background_worker_purges_expired_items_and_stops_cleanly() {
        let dir = tempdir().unwrap();
        let svc = Arc::new(
            VaultService::with_custom_root(
                dir.path().join("vault_manifest.db"),
                dir.path().join("vault"),
            )
            .unwrap(),
        );
        let expired_file = dir.path().join("stale_for_worker.log");
        std::fs::write(&expired_file, b"worker purge me").unwrap();

        let old_time = Utc::now() - chrono::Duration::hours(170);
        let batch = svc
            .quarantine_items(
                [&expired_file],
                "stale",
                "sess-worker",
                "codex",
                "{}",
                old_time,
            )
            .unwrap();
        let item_id = batch.items[0].item_id.clone();

        let handle =
            start_expiration_worker_with_interval(Arc::clone(&svc), Duration::from_millis(25));

        // Poll briefly for the worker thread's first cycle to finish
        for _ in 0..40 {
            if svc.get_item(&item_id).unwrap().unwrap().status == VaultItemStatus::PurgedExpired {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }

        handle.stop();
        assert_eq!(
            svc.get_item(&item_id).unwrap().unwrap().status,
            VaultItemStatus::PurgedExpired
        );
    }

    #[test]
    fn quarantine_rejects_empty_directories_and_missing_files() {
        let dir = tempdir().unwrap();
        let svc = VaultService::with_custom_root(
            dir.path().join("vault_manifest.db"),
            dir.path().join("vault"),
        )
        .unwrap();
        let now = Utc::now();

        let empty_err = svc
            .quarantine_items(
                std::iter::empty::<PathBuf>(),
                "empty",
                "s",
                "codex",
                "{}",
                now,
            )
            .unwrap_err();
        assert!(matches!(empty_err, VaultError::EmptyItemList));

        let missing_err = svc
            .quarantine_items([dir.path().join("missing.txt")], "m", "s", "codex", "{}", now)
            .unwrap_err();
        assert!(matches!(missing_err, VaultError::ItemNotFound { .. }));

        let sub_dir = dir.path().join("subdir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let dir_err = svc
            .quarantine_items([&sub_dir], "d", "s", "codex", "{}", now)
            .unwrap_err();
        assert!(matches!(dir_err, VaultError::NotAFile { .. }));
    }

    #[test]
    fn per_volume_vault_root_resolves_drive_dot_windirstatvault_objects() {
        let dir = tempdir().unwrap();
        let probe = dir.path().join("probe.bin");
        std::fs::write(&probe, b"probe").unwrap();

        let svc = VaultService::with_db_path(dir.path().join("vault_manifest.db")).unwrap();
        let resolved = svc
            .resolve_batch_objects_dir(&probe, "01933f10-0000-7000-8000-000000000099")
            .unwrap();

        let drive_root = acl::get_volume_root(&probe).unwrap();
        let expected = drive_root
            .join(DEFAULT_VAULT_DIR_NAME)
            .join("objects")
            .join("01933f10-0000-7000-8000-000000000099");
        assert_eq!(resolved, expected);
    }
}

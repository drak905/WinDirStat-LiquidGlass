use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Duration, TimeZone, Utc};
use tempfile::tempdir;
use windirstat_liquid_glass::vault::acl::capture_file_security_snapshot;
use windirstat_liquid_glass::vault::checksum::compute_file_checksums;
use windirstat_liquid_glass::vault::{
    VaultError, VaultItemStatus, VaultService,
};

fn collect_files_recursive(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
    }
    out
}

#[test]
fn chaos_bit_exact_checksum_sddl_acl_and_timestamps_on_rollback() {
    let workspace = tempdir().unwrap();
    let src_dir = workspace.path().join("source_tree");
    fs::create_dir_all(&src_dir).unwrap();
    let vault_root = workspace.path().join(".WinDirStatVault");
    let db_path = workspace.path().join("vault_manifest.db");

    let target_file = src_dir.join("critical_artifact.bin");
    let mut payload = Vec::with_capacity(300_000);
    for i in 0..300_000usize {
        payload.push(((i * 31 + 7) % 251) as u8);
    }
    fs::write(&target_file, &payload).unwrap();

    // Apply a custom explicit ACL rule via icacls to ensure non-trivial SDDL preservation
    let icacls_status = Command::new("icacls")
        .arg(&target_file)
        .args(["/inheritance:d", "/grant:r", "BUILTIN\\Users:(R)"])
        .status()
        .expect("icacls must run on Windows");
    assert!(icacls_status.success(), "icacls failed to set custom ACL");

    // Set distinct creation/modified timestamps and readonly attribute before snapshot
    let pre_snap = capture_file_security_snapshot(&target_file).unwrap();
    let expected_ctime = Utc.with_ymd_and_hms(2023, 8, 10, 14, 15, 16).unwrap()
        + Duration::nanoseconds(250_000_000);
    let expected_mtime = Utc.with_ymd_and_hms(2024, 12, 24, 23, 59, 58).unwrap()
        + Duration::nanoseconds(750_000_000);
    let expected_attrs = pre_snap.attributes | 0x1; // FILE_ATTRIBUTE_READONLY
    windirstat_liquid_glass::vault::acl::restore_file_metadata_and_acl(
        &target_file,
        &pre_snap.ntfs_acl_sddl,
        expected_ctime,
        expected_mtime,
        expected_attrs,
    )
    .unwrap();

    let baseline_digest = compute_file_checksums(&target_file).unwrap();
    let baseline_snap = capture_file_security_snapshot(&target_file).unwrap();

    let service = VaultService::with_custom_root(&db_path, &vault_root).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();

    let batch = service
        .quarantine_items(
            [&target_file],
            "Tier-3 bit-exact test",
            "sess-chaos-1",
            "codex",
            r#"{"rule":"cache_cleanup","confidence":0.98}"#,
            t0,
        )
        .unwrap();

    assert_eq!(batch.items.len(), 1);
    assert!(!target_file.exists(), "original file should be moved out");
    let record = &batch.items[0];
    assert_eq!(record.xxhash64_checksum, baseline_digest.xxhash64);
    assert_eq!(record.sha256_checksum, baseline_digest.sha256);
    assert_eq!(record.ntfs_acl_sddl, baseline_snap.ntfs_acl_sddl);
    assert!(Path::new(&record.vault_storage_path).exists());

    // Delete parent directory completely to test parent recreation during rollback
    windirstat_liquid_glass::vault::acl::clear_readonly_and_remove_file(&target_file).ok();
    fs::remove_dir_all(&src_dir).unwrap();
    assert!(!src_dir.exists());

    let restored = service.rollback_batch(&batch.batch_id).unwrap();
    assert_eq!(restored.restored.len(), 1);
    assert!(target_file.exists(), "file must be restored to original path");
    assert!(!Path::new(&record.vault_storage_path).exists());

    // Verify bit-exact bytes, hashes, SDDL ACL, timestamps, and attributes
    let restored_bytes = fs::read(&target_file).unwrap();
    assert_eq!(restored_bytes, payload);

    let after_digest = compute_file_checksums(&target_file).unwrap();
    assert_eq!(after_digest, baseline_digest);

    let after_snap = capture_file_security_snapshot(&target_file).unwrap();
    assert_eq!(after_snap.ntfs_acl_sddl, baseline_snap.ntfs_acl_sddl);
    assert_eq!(after_snap.creation_time_utc, baseline_snap.creation_time_utc);
    assert_eq!(after_snap.modified_time_utc, baseline_snap.modified_time_utc);
    assert_ne!(after_snap.attributes & 0x1, 0, "READONLY attribute must survive rollback");

    let db_record = service.get_item(&record.item_id).unwrap().unwrap();
    assert_eq!(db_record.status, VaultItemStatus::Restored);

    // Clean up readonly file cleanly on drop
    windirstat_liquid_glass::vault::acl::clear_readonly_and_remove_file(&target_file).unwrap();
}

#[test]
fn chaos_locked_file_aborts_gracefully_with_zero_partial_state() {
    let workspace = tempdir().unwrap();
    let src_dir = workspace.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let vault_root = workspace.path().join(".WinDirStatVault");
    let db_path = workspace.path().join("vault_manifest.db");

    let unlocked_a = src_dir.join("a_unlocked.log");
    let locked_b = src_dir.join("b_locked.db");
    let unlocked_c = src_dir.join("c_unlocked.tmp");

    fs::write(&unlocked_a, b"alpha-contents").unwrap();
    fs::write(&locked_b, b"beta-locked-contents").unwrap();
    fs::write(&unlocked_c, b"gamma-contents").unwrap();

    // Hold exclusive Windows lock (FILE_SHARE_NONE) on locked_b
    let lock_guard = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&locked_b)
        .unwrap();

    let service = VaultService::with_custom_root(&db_path, &vault_root).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 18, 13, 0, 0).unwrap();

    let err = service
        .quarantine_items(
            [&unlocked_a, &locked_b, &unlocked_c],
            "Batch with locked file",
            "sess-lock",
            "claude",
            "{}",
            t0,
        )
        .unwrap_err();

    assert!(
        matches!(err, VaultError::FileLocked { .. }),
        "expected VaultError::FileLocked, got {err:?}"
    );
    drop(lock_guard);

    // Zero partial moves: all 3 files must still exist untouched at original paths
    assert_eq!(fs::read(&unlocked_a).unwrap(), b"alpha-contents");
    assert_eq!(fs::read(&locked_b).unwrap(), b"beta-locked-contents");
    assert_eq!(fs::read(&unlocked_c).unwrap(), b"gamma-contents");

    // Zero partial objects in vault storage
    let leftover_objects = collect_files_recursive(&vault_root.join("objects"));
    assert!(
        leftover_objects.is_empty(),
        "expected zero objects in vault after aborted batch, found {leftover_objects:?}"
    );

    // Zero committed rows in vault_manifest.db
    let items = service
        .list_items_last_days(30, t0 + Duration::minutes(1))
        .unwrap();
    assert!(items.is_empty(), "manifest must remain empty after lock abort");
}

#[test]
fn chaos_simulated_168h_ttl_clock_progression_enforces_exact_expiration() {
    let workspace = tempdir().unwrap();
    let src_dir = workspace.path().join("ttl_src");
    fs::create_dir_all(&src_dir).unwrap();
    let vault_root = workspace.path().join(".WinDirStatVault");
    let db_path = workspace.path().join("vault_manifest.db");

    let target = src_dir.join("readonly_stale.dmp");
    fs::write(&target, b"dump-bytes-for-168h-ttl").unwrap();

    // Mark file read-only to prove purge_expired unlinks even read-only quarantined objects
    let mut perms = fs::metadata(&target).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&target, perms).unwrap();

    let service = VaultService::with_custom_root(&db_path, &vault_root).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 18, 10, 0, 0).unwrap();

    let batch = service
        .quarantine_items(
            [&target],
            "Crash dump cleanup",
            "sess-ttl",
            "opencode",
            r#"{"category":"crash_dump"}"#,
            t0,
        )
        .unwrap();
    let item = &batch.items[0];
    let vault_obj_path = PathBuf::from(&item.vault_storage_path);
    assert!(vault_obj_path.exists());

    // 167h 59m -> MUST NOT purge!
    let t_before_expiry = t0 + Duration::hours(167) + Duration::minutes(59);
    let purge_early = service.purge_expired(t_before_expiry).unwrap();
    assert_eq!(purge_early.purged_count, 0);
    assert!(purge_early.purged_item_ids.is_empty());
    assert!(vault_obj_path.exists(), "vault file must survive at 167h 59m");
    assert_eq!(
        service.get_item(&item.item_id).unwrap().unwrap().status,
        VaultItemStatus::Quarantined
    );

    // Verify 24h warning toast window includes the item at 167h 59m
    let expiring_soon = service.items_expiring_within_24h(t_before_expiry).unwrap();
    assert_eq!(expiring_soon.len(), 1);
    assert_eq!(expiring_soon[0].item_id, item.item_id);

    // 168h 01m -> MUST cleanly unlink file and set status = PURGED_EXPIRED!
    let t_after_expiry = t0 + Duration::hours(168) + Duration::minutes(1);
    let purge_due = service.purge_expired(t_after_expiry).unwrap();
    assert_eq!(purge_due.purged_count, 1);
    assert_eq!(purge_due.purged_item_ids, vec![item.item_id.clone()]);
    assert!(purge_due.failures.is_empty());
    assert!(!vault_obj_path.exists(), "vault file must be unlinked after 168h 01m");

    let final_record = service.get_item(&item.item_id).unwrap().unwrap();
    assert_eq!(final_record.status, VaultItemStatus::PurgedExpired);

    // Rollback after permanent purge must fail gracefully
    let rollback_err = service.rollback_item(&item.item_id).unwrap_err();
    assert!(matches!(rollback_err, VaultError::AlreadyPurged { .. }));
}

#[test]
fn chaos_tampered_vault_object_fails_checksum_verification_on_rollback() {
    let workspace = tempdir().unwrap();
    let src_dir = workspace.path().join("tamper_src");
    fs::create_dir_all(&src_dir).unwrap();
    let vault_root = workspace.path().join(".WinDirStatVault");
    let db_path = workspace.path().join("vault_manifest.db");

    let target = src_dir.join("integrity.dat");
    fs::write(&target, b"TRUSTED_ORIGINAL_DATA").unwrap();

    let service = VaultService::with_custom_root(&db_path, &vault_root).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
    let batch = service
        .quarantine_items([&target], "integrity test", "sess-tamper", "codex", "{}", t0)
        .unwrap();
    let item = &batch.items[0];

    // Corrupt 1 byte in the vaulted object
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&item.vault_storage_path)
        .unwrap();
    file.write_all(b"X").unwrap();
    file.flush().unwrap();

    let err = service.rollback_item(&item.item_id).unwrap_err();
    assert!(
        matches!(err, VaultError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
    assert!(!target.exists(), "corrupted data must never be restored to original path");
    assert_eq!(
        service.get_item(&item.item_id).unwrap().unwrap().status,
        VaultItemStatus::Quarantined
    );
}

#[test]
fn chaos_wds_vault_cli_lists_and_rolls_back_batches_end_to_end() {
    let workspace = tempdir().unwrap();
    let src_dir = workspace.path().join("cli_src");
    fs::create_dir_all(&src_dir).unwrap();
    let vault_root = workspace.path().join(".WinDirStatVault");
    let db_path = workspace.path().join("vault_manifest.db");

    let f1 = src_dir.join("one.log");
    let f2 = src_dir.join("two.log");
    let f3 = src_dir.join("three.log");
    fs::write(&f1, b"first-batch-file-1").unwrap();
    fs::write(&f2, b"first-batch-file-2").unwrap();
    fs::write(&f3, b"second-batch-file-3").unwrap();

    let service = VaultService::with_custom_root(&db_path, &vault_root).unwrap();
    let now = Utc::now();

    let batch1 = service
        .quarantine_items(
            [&f1, &f2],
            "CLI test batch 1",
            "sess-cli-1",
            "antigravity",
            r#"{"kind":"temp"}"#,
            now - Duration::hours(2),
        )
        .unwrap();

    let batch2 = service
        .quarantine_items(
            [&f3],
            "CLI test batch 2",
            "sess-cli-2",
            "kiro",
            r#"{"kind":"cache"}"#,
            now - Duration::hours(1),
        )
        .unwrap();

    assert!(!f1.exists() && !f2.exists() && !f3.exists());

    let cli_bin = env!("CARGO_BIN_EXE_wds-vault");

    // 1. wds-vault list --last 7d --db <db_path>
    let list_out = Command::new(cli_bin)
        .args(["list", "--last", "7d", "--db"])
        .arg(&db_path)
        .output()
        .expect("failed to execute wds-vault list");
    assert!(
        list_out.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(stdout.contains(&batch1.batch_id), "stdout missing batch1: {stdout}");
    assert!(stdout.contains(&batch2.batch_id), "stdout missing batch2: {stdout}");
    assert!(stdout.contains("QUARANTINED"), "stdout missing QUARANTINED: {stdout}");

    // 2. wds-vault rollback --batch <batch1_id> --db <db_path>
    let rb_batch_out = Command::new(cli_bin)
        .args(["rollback", "--batch", &batch1.batch_id, "--db"])
        .arg(&db_path)
        .output()
        .expect("failed to execute wds-vault rollback --batch");
    assert!(
        rb_batch_out.status.success(),
        "rollback --batch failed: stderr={}",
        String::from_utf8_lossy(&rb_batch_out.stderr)
    );
    assert_eq!(fs::read(&f1).unwrap(), b"first-batch-file-1");
    assert_eq!(fs::read(&f2).unwrap(), b"first-batch-file-2");
    assert!(!f3.exists(), "batch2 item should still be quarantined");

    // 3. wds-vault --db <db_path> rollback --all-recent
    let rb_all_out = Command::new(cli_bin)
        .arg("--db")
        .arg(&db_path)
        .args(["rollback", "--all-recent"])
        .output()
        .expect("failed to execute wds-vault rollback --all-recent");
    assert!(
        rb_all_out.status.success(),
        "rollback --all-recent failed: stderr={}",
        String::from_utf8_lossy(&rb_all_out.stderr)
    );
    assert_eq!(fs::read(&f3).unwrap(), b"second-batch-file-3");

    // 4. wds-vault --json list --last 7d --db <db_path> emits valid JSON with RESTORED status
    let json_list_out = Command::new(cli_bin)
        .args(["--json", "list", "--last", "7d", "--db"])
        .arg(&db_path)
        .output()
        .expect("failed to execute wds-vault --json list");
    assert!(json_list_out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&json_list_out.stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["batches"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["items"].as_array().unwrap().len(), 3);

    // 5. Unknown batch ID must exit non-zero cleanly
    let unknown_out = Command::new(cli_bin)
        .args([
            "rollback",
            "--batch",
            "00000000-0000-7000-8000-000000000000",
            "--db",
        ])
        .arg(&db_path)
        .output()
        .expect("failed to execute wds-vault rollback unknown batch");
    assert!(!unknown_out.status.success());
}

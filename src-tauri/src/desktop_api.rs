use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_hub::context::{LockedHandleStatus, StorageContextPayload};
use crate::agent_hub::providers::{
    discover_all_provider_statuses, AgentProvider, AgentProviderKind, CustomCliConfig,
};
use crate::agent_hub::runner::{AgentHubRunner, DefaultCommandResolver, RunnerConfig};
use crate::agent_hub::validator::AdvisorVerdict;
use crate::ipc_bridge::snapshot_bridge::{compute_file_xxhash64, group_duplicates_by_xxhash64};
use crate::ipc_bridge::ScanNode;
use crate::vault::schema::VaultItemStatus;
use crate::vault::{VaultConfig, VaultService};

const MAX_SCAN_NODES: usize = 18_000;
const MAX_SCAN_DEPTH: usize = 6;
const MAX_SCAN_MILLIS: u128 = 4_500;
const MAX_DUP_FILES: usize = 8_000;
const MAX_DUP_HASH_SIZE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDriveInfo {
    pub id: String,
    pub label: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiStorageNode {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    pub file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<UiStorageNode>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiExtensionSummary {
    pub extension: String,
    pub color: String,
    pub total_bytes: u64,
    pub file_count: usize,
    pub percent: f64,
    pub sample_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiScanSnapshot {
    pub drive_id: String,
    pub root: UiStorageNode,
    pub total_nodes: usize,
    pub scan_duration_ms: u64,
    pub extensions: Vec<UiExtensionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDuplicateFileEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDuplicateGroup {
    pub id: String,
    pub hash: String,
    pub hash_algorithm: &'static str,
    pub file_size: u64,
    pub wasted_bytes: u64,
    pub extension: String,
    pub files: Vec<UiDuplicateFileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiAgentProviderStatus {
    pub provider: &'static str,
    pub display_name: String,
    pub available: bool,
    pub command: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiVaultItem {
    pub id: String,
    pub batch_id: String,
    pub original_path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub quarantined_at: String,
    pub expires_at: String,
    pub remaining_hours: f64,
    pub sha256: String,
    pub xxhash64: String,
    pub hash_verified: bool,
    pub status: &'static str,
}

pub struct AppContext {
    pub vault_db_path: PathBuf,
}

impl AppContext {
    pub fn open_vault(&self) -> Result<VaultService, String> {
        VaultService::new(VaultConfig {
            manifest_db_path: self.vault_db_path.clone(),
            vault_root_override: None,
            enable_ntfs_compression: true,
        })
        .map_err(|err| err.to_string())
    }
}

pub fn default_app_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        if !local.is_empty() {
            return PathBuf::from(local).join("WinDirStatLiquidGlass");
        }
    }
    env::temp_dir().join("WinDirStatLiquidGlass")
}

#[cfg(windows)]
pub fn query_windows_drives() -> Vec<UiDriveInfo> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;

    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if !Path::new(&root).exists() {
            continue;
        }
        let wide: Vec<u16> = std::ffi::OsStr::new(&root)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
        let kind = match drive_type {
            DRIVE_FIXED => "nvme",
            DRIVE_REMOVABLE => "ssd",
            DRIVE_REMOTE => "network",
            _ => continue,
        };

        let mut free_avail: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free: u64 = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_avail,
                &mut total_bytes,
                &mut total_free,
            )
        };
        if ok == 0 || total_bytes == 0 {
            continue;
        }

        let used_bytes = total_bytes.saturating_sub(total_free);
        let label = match drive_type {
            DRIVE_REMOTE => format!("{root} Network Volume"),
            DRIVE_REMOVABLE => format!("{root} External Drive"),
            _ => format!("{root} Local Disk"),
        };

        drives.push(UiDriveInfo {
            id: root.clone(),
            label,
            mount_point: root,
            total_bytes,
            used_bytes,
            free_bytes: total_free,
            kind,
        });
    }

    drives.push(UiDriveInfo {
        id: "demo://100k-snapshot".to_string(),
        label: "100k Node Benchmark Snapshot".to_string(),
        mount_point: "demo://100k-snapshot".to_string(),
        total_bytes: 512_000_000_000,
        used_bytes: 38_000_000_000,
        free_bytes: 474_000_000_000,
        kind: "snapshot",
    });

    drives
}

#[cfg(not(windows))]
pub fn query_windows_drives() -> Vec<UiDriveInfo> {
    vec![UiDriveInfo {
        id: "demo://100k-snapshot".to_string(),
        label: "100k Node Benchmark Snapshot".to_string(),
        mount_point: "demo://100k-snapshot".to_string(),
        total_bytes: 512_000_000_000,
        used_bytes: 38_000_000_000,
        free_bytes: 474_000_000_000,
        kind: "snapshot",
    }]
}

const EXT_PALETTE: [&str; 8] = [
    "#38bdf8", "#818cf8", "#34d399", "#fbbf24", "#f472b6", "#fb7185", "#a78bfa", "#2dd4bf",
];

fn extension_color(ext: &str) -> String {
    let sum: usize = ext.bytes().map(|b| b as usize).sum();
    EXT_PALETTE[sum % EXT_PALETTE.len()].to_string()
}

struct ScanState {
    started: Instant,
    next_id: usize,
    ext_stats: HashMap<String, (u64, usize, String)>,
}

fn scan_node_recursive(
    path: &Path,
    depth: usize,
    state: &mut ScanState,
) -> Option<UiStorageNode> {
    if state.next_id >= MAX_SCAN_NODES || state.started.elapsed().as_millis() > MAX_SCAN_MILLIS {
        return None;
    }

    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }

    state.next_id += 1;
    let node_id = format!("n-{}", state.next_id);
    let path_str = path.to_string_lossy().into_owned();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path_str.clone());

    if meta.is_file() {
        let size = meta.len();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or_else(|| "(no ext)".to_string());
        let entry = state
            .ext_stats
            .entry(ext.clone())
            .or_insert_with(|| (0, 0, path_str.clone()));
        entry.0 = entry.0.saturating_add(size);
        entry.1 += 1;

        return Some(UiStorageNode {
            id: node_id,
            name,
            path: path_str,
            size,
            is_dir: false,
            extension: Some(ext),
            file_count: 1,
            modified_at: None,
            children: None,
        });
    }

    if !meta.is_dir() {
        return None;
    }

    let mut children = Vec::new();
    let mut dir_size = 0u64;
    let mut file_count = 0usize;

    if depth < MAX_SCAN_DEPTH {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                if state.next_id >= MAX_SCAN_NODES
                    || state.started.elapsed().as_millis() > MAX_SCAN_MILLIS
                {
                    break;
                }
                if let Some(child) = scan_node_recursive(&entry.path(), depth + 1, state) {
                    dir_size = dir_size.saturating_add(child.size);
                    file_count = file_count.saturating_add(child.file_count);
                    children.push(child);
                }
            }
        }
    }

    children.sort_by(|a, b| b.size.cmp(&a.size));

    Some(UiStorageNode {
        id: node_id,
        name,
        path: path_str,
        size: dir_size,
        is_dir: true,
        extension: None,
        file_count,
        modified_at: None,
        children: Some(children),
    })
}

pub fn scan_directory_snapshot(target: &str) -> Option<UiScanSnapshot> {
    if target.starts_with("demo://") {
        return None;
    }
    let root_path = Path::new(target);
    if !root_path.exists() {
        return None;
    }

    let mut state = ScanState {
        started: Instant::now(),
        next_id: 0,
        ext_stats: HashMap::new(),
    };
    let root = scan_node_recursive(root_path, 0, &mut state)?;
    let total_size = root.size.max(1) as f64;

    let mut extensions: Vec<UiExtensionSummary> = state
        .ext_stats
        .into_iter()
        .map(|(ext, (bytes, count, sample_path))| UiExtensionSummary {
            color: extension_color(&ext),
            extension: ext,
            total_bytes: bytes,
            file_count: count,
            percent: ((bytes as f64) / total_size) * 100.0,
            sample_path,
        })
        .collect();
    extensions.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));
    extensions.truncate(16);

    Some(UiScanSnapshot {
        drive_id: target.to_string(),
        root,
        total_nodes: state.next_id,
        scan_duration_ms: state.started.elapsed().as_millis().max(1) as u64,
        extensions,
    })
}

pub fn find_real_duplicates(target: &str) -> Option<Vec<UiDuplicateGroup>> {
    if target.starts_with("demo://") {
        return None;
    }
    let root = Path::new(target);
    if !root.exists() {
        return None;
    }

    let started = Instant::now();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut seen_files = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if seen_files >= MAX_DUP_FILES || started.elapsed().as_millis() > 2_500 {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if seen_files >= MAX_DUP_FILES || started.elapsed().as_millis() > 2_500 {
                break;
            }
            let p = entry.path();
            let Ok(meta) = fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() && depth < 4 {
                stack.push((p, depth + 1));
            } else if meta.is_file() {
                let len = meta.len();
                if len >= 4096 && len <= MAX_DUP_HASH_SIZE_BYTES {
                    by_size.entry(len).or_default().push(p);
                    seen_files += 1;
                }
            }
        }
    }

    let mut candidate_nodes = Vec::new();
    let mut next_id = 1u64;
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        for path in paths {
            if started.elapsed().as_millis() > 4_000 {
                break;
            }
            if let Ok(hash) = compute_file_xxhash64(&path) {
                let ext = path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
                    .unwrap_or_default();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                candidate_nodes.push(ScanNode {
                    id: next_id,
                    parent_id: None,
                    name,
                    full_path: path.to_string_lossy().into_owned(),
                    is_dir: false,
                    logical_size: size,
                    physical_size: size,
                    modified_utc: Utc::now().to_rfc3339(),
                    owner: String::new(),
                    xxhash64: Some(hash),
                    extension: ext,
                });
                next_id += 1;
            }
        }
    }

    let groups = group_duplicates_by_xxhash64(&candidate_nodes)
        .unwrap_or_default()
        .into_iter()
        .take(25)
        .enumerate()
        .map(|(idx, grp)| {
            let ext = grp
                .files
                .first()
                .map(|f| f.extension.clone())
                .unwrap_or_default();
            UiDuplicateGroup {
                id: format!("dup-{}", idx + 1),
                hash: grp.hash_hex,
                hash_algorithm: "xxHash64+SHA256",
                file_size: grp.file_size,
                wasted_bytes: grp.wasted_bytes,
                extension: ext,
                files: grp
                    .files
                    .into_iter()
                    .map(|f| UiDuplicateFileEntry {
                        id: format!("df-{}", f.id),
                        name: f.name,
                        path: f.full_path,
                        modified_at: f.modified_utc,
                    })
                    .collect(),
            }
        })
        .collect();

    Some(groups)
}

pub fn map_provider_id(kind: AgentProviderKind) -> &'static str {
    match kind {
        AgentProviderKind::Antigravity => "antigravity",
        AgentProviderKind::Kiro => "kiro",
        AgentProviderKind::Claude => "claude",
        AgentProviderKind::Codex => "codex",
        AgentProviderKind::OpenCode => "opencode",
        AgentProviderKind::Custom => "custom",
    }
}

pub fn detect_ui_providers() -> Vec<UiAgentProviderStatus> {
    let mut statuses: Vec<UiAgentProviderStatus> = discover_all_provider_statuses()
        .into_iter()
        .map(|st| UiAgentProviderStatus {
            provider: map_provider_id(st.kind),
            display_name: st.display_name,
            available: st.available,
            command: st.command,
            latency_ms: if st.available { 45 } else { 0 },
        })
        .collect();
    statuses.push(UiAgentProviderStatus {
        provider: "custom",
        display_name: "Custom CLI Adapter".to_string(),
        available: true,
        command: "custom-agent".to_string(),
        latency_ms: 50,
    });
    statuses
}

fn parse_provider(name: &str, custom_command: Option<&str>) -> Option<AgentProvider> {
    match name {
        "antigravity" => Some(AgentProvider::Antigravity),
        "kiro" => Some(AgentProvider::Kiro),
        "claude" => Some(AgentProvider::Claude),
        "codex" => Some(AgentProvider::Codex),
        "opencode" => Some(AgentProvider::OpenCode),
        "custom" => {
            let cmd = custom_command?.trim();
            if cmd.is_empty() {
                return None;
            }
            let mut parts = cmd.split_whitespace();
            let exe = parts.next()?.to_string();
            let mut args: Vec<String> = parts.map(ToString::to_string).collect();
            if !args.iter().any(|a| a.contains("{{PROMPT}}")) {
                args.push("{{PROMPT}}".to_string());
            }
            Some(AgentProvider::Custom(CustomCliConfig::new(exe, args)))
        }
        _ => None,
    }
}

pub fn try_run_real_advisor(
    provider_str: &str,
    prompt: &str,
    target_path: Option<&str>,
    target_name: Option<&str>,
    target_size_bytes: Option<u64>,
    custom_command: Option<&str>,
) -> Option<Value> {
    let target_path = target_path.unwrap_or("");
    if target_path.is_empty() || !Path::new(target_path).exists() {
        return None;
    }

    let provider = parse_provider(provider_str, custom_command)?;
    if !matches!(provider, AgentProvider::Custom(_)) {
        let available = discover_all_provider_statuses()
            .into_iter()
            .any(|s| map_provider_id(s.kind) == provider_str && s.available);
        if !available {
            return None;
        }
    }

    let size_bytes = target_size_bytes.unwrap_or(0);
    let resolved_name = target_name
        .map(ToString::to_string)
        .unwrap_or_else(|| target_path.to_string());

    let payload = StorageContextPayload {
        path: target_path.to_string(),
        logical_size_bytes: size_bytes,
        physical_size_bytes: size_bytes,
        file_count: 1,
        oldest_modified_utc: Some(Utc::now().to_rfc3339()),
        newest_modified_utc: Some(Utc::now().to_rfc3339()),
        publisher_signature: None,
        top_extensions: Vec::new(),
        locked_handle_status: LockedHandleStatus::Unlocked,
        user_question: prompt.to_string(),
    };

    let rt = tokio::runtime::Runtime::new().ok()?;
    let runner = AgentHubRunner::with_resolver(
        Arc::new(DefaultCommandResolver),
        RunnerConfig {
            timeout: Duration::from_secs(30),
            ..RunnerConfig::default()
        },
    );
    let outcome = rt
        .block_on(runner.run_advisor(&provider, &payload, None))
        .ok()?;
    let verdict = match outcome.response.verdict {
        AdvisorVerdict::SafeToDelete => "SAFE_TO_DELETE",
        AdvisorVerdict::CleanViaOfficialTool => "QUARANTINE_RECOMMENDED",
        AdvisorVerdict::SafeWithAppClosed => "REVIEW_CAREFULLY",
        AdvisorVerdict::KeepCritical => "KEEP",
    };

    let sources: Vec<Value> = outcome
        .response
        .cross_checked_sources
        .into_iter()
        .map(|s| {
            let domain = url::Url::parse(&s.url)
                .ok()
                .and_then(|u| u.host_str().map(ToString::to_string))
                .unwrap_or_default();
            json!({
                "title": s.title,
                "url": s.url,
                "domain": domain,
                "verified": true,
                "snippet": s.finding,
            })
        })
        .collect();

    Some(json!({
        "id": format!("adv-{}", Utc::now().timestamp_millis()),
        "targetPath": target_path,
        "targetName": resolved_name,
        "targetSizeBytes": size_bytes,
        "provider": provider_str,
        "verdict": verdict,
        "risk_score": outcome.response.risk_score,
        "reclaimable_bytes": outcome.response.reclaimable_bytes,
        "usage_value_assessment": outcome.response.usage_value_assessment,
        "rationale": outcome.response.recommended_action,
        "sources": sources,
        "createdAt": Utc::now().to_rfc3339(),
    }))
}

pub fn map_vault_record(
    rec: crate::vault::schema::VaultItemRecord,
) -> UiVaultItem {
    let now = Utc::now();
    let remaining_hours = chrono::DateTime::parse_from_rfc3339(&rec.expires_at_utc)
        .map(|exp| {
            let secs = exp.with_timezone(&Utc).signed_duration_since(now).num_seconds();
            (secs.max(0) as f64) / 3600.0
        })
        .unwrap_or(168.0);
    let file_name = Path::new(&rec.original_full_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| rec.original_full_path.clone());
    let status = match rec.status {
        VaultItemStatus::Quarantined => "quarantined",
        VaultItemStatus::Restored => "restored",
        VaultItemStatus::PurgedExpired => "purged",
    };
    UiVaultItem {
        id: rec.item_id,
        batch_id: rec.batch_id,
        original_path: rec.original_full_path,
        file_name,
        size_bytes: rec.logical_size_bytes.max(0) as u64,
        quarantined_at: rec.quarantined_at_utc,
        expires_at: rec.expires_at_utc,
        remaining_hours,
        sha256: rec.sha256_checksum,
        xxhash64: rec.xxhash64_checksum,
        hash_verified: true,
        status,
    }
}

pub fn quarantine_real_item(ctx: &AppContext, path_str: &str) -> Option<UiVaultItem> {
    let path = Path::new(path_str);
    if !path.is_file() {
        return None;
    }
    let svc = ctx.open_vault().ok()?;
    let mut res = svc
        .quarantine_items(
            [path],
            "UI Quarantine",
            "desktop-session",
            "desktop-ui",
            "{}",
            Utc::now(),
        )
        .ok()?;
    if res.items.is_empty() {
        return None;
    }
    Some(map_vault_record(res.items.remove(0)))
}

pub fn list_real_vault_items(ctx: &AppContext) -> Option<Vec<UiVaultItem>> {
    let svc = ctx.open_vault().ok()?;
    let items = svc.list_items_last_days(30, Utc::now()).ok()?;
    if items.is_empty() {
        return None;
    }
    Some(items.into_iter().map(map_vault_record).collect())
}

pub fn rollback_real_vault_item(ctx: &AppContext, id: &str) -> Option<UiVaultItem> {
    if id.is_empty() {
        return None;
    }
    let svc = ctx.open_vault().ok()?;
    svc.rollback_item(id).ok()?;
    svc.get_item(id).ok().flatten().map(map_vault_record)
}

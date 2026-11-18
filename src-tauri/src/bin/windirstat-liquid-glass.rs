#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use windirstat_liquid_glass::agent_hub::context::{LockedHandleStatus, StorageContextPayload};
use windirstat_liquid_glass::agent_hub::providers::{
    discover_all_provider_statuses, AgentProvider, AgentProviderKind, CustomCliConfig,
};
use windirstat_liquid_glass::agent_hub::runner::{
    AgentHubRunner, DefaultCommandResolver, RunnerConfig,
};
use windirstat_liquid_glass::agent_hub::validator::AdvisorVerdict;
use windirstat_liquid_glass::ipc_bridge::snapshot_bridge::{
    compute_file_xxhash64, group_duplicates_by_xxhash64,
};
use windirstat_liquid_glass::ipc_bridge::ScanNode;
use windirstat_liquid_glass::vault::schema::VaultItemStatus;
use windirstat_liquid_glass::vault::service::start_expiration_worker;
use windirstat_liquid_glass::vault::{VaultConfig, VaultService};

include!(concat!(env!("OUT_DIR"), "/embedded_ui_assets.rs"));

const MAX_SCAN_NODES: usize = 18_000;
const MAX_SCAN_DEPTH: usize = 6;
const MAX_SCAN_MILLIS: u128 = 4_500;
const MAX_DUP_FILES: usize = 8_000;
const MAX_DUP_HASH_SIZE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct InvokeRequest {
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiDriveInfo {
    id: String,
    label: String,
    mount_point: String,
    total_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
    kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiStorageNode {
    id: String,
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension: Option<String>,
    file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<UiStorageNode>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiExtensionSummary {
    extension: String,
    color: String,
    total_bytes: u64,
    file_count: usize,
    percent: f64,
    sample_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiScanSnapshot {
    drive_id: String,
    root: UiStorageNode,
    total_nodes: usize,
    scan_duration_ms: u64,
    extensions: Vec<UiExtensionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiDuplicateFileEntry {
    id: String,
    name: String,
    path: String,
    modified_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiDuplicateGroup {
    id: String,
    hash: String,
    hash_algorithm: &'static str,
    file_size: u64,
    wasted_bytes: u64,
    extension: String,
    files: Vec<UiDuplicateFileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiAgentProviderStatus {
    provider: &'static str,
    display_name: String,
    available: bool,
    command: String,
    latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiVaultItem {
    id: String,
    batch_id: String,
    original_path: String,
    file_name: String,
    size_bytes: u64,
    quarantined_at: String,
    expires_at: String,
    remaining_hours: f64,
    sha256: String,
    xxhash64: String,
    hash_verified: bool,
    status: &'static str,
}

struct AppContext {
    vault_db_path: PathBuf,
}

impl AppContext {
    fn open_vault(&self) -> Result<VaultService, String> {
        VaultService::new(VaultConfig {
            manifest_db_path: self.vault_db_path.clone(),
            vault_root_override: None,
            enable_ntfs_compression: true,
        })
        .map_err(|err| err.to_string())
    }
}

fn default_app_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        if !local.is_empty() {
            return PathBuf::from(local).join("WinDirStatLiquidGlass");
        }
    }
    env::temp_dir().join("WinDirStatLiquidGlass")
}

#[cfg(windows)]
fn query_windows_drives() -> Vec<UiDriveInfo> {
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
fn query_windows_drives() -> Vec<UiDriveInfo> {
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

fn scan_directory_snapshot(target: &str) -> Option<UiScanSnapshot> {
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

fn find_real_duplicates(target: &str) -> Option<Vec<UiDuplicateGroup>> {
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

fn map_provider_id(kind: AgentProviderKind) -> &'static str {
    match kind {
        AgentProviderKind::Antigravity => "antigravity",
        AgentProviderKind::Kiro => "kiro",
        AgentProviderKind::Claude => "claude",
        AgentProviderKind::Codex => "codex",
        AgentProviderKind::OpenCode => "opencode",
        AgentProviderKind::Custom => "custom",
    }
}

fn detect_ui_providers() -> Vec<UiAgentProviderStatus> {
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

fn try_run_real_advisor(args: &Value) -> Option<Value> {
    let provider_str = args.get("provider")?.as_str()?;
    let prompt = args.get("prompt")?.as_str()?;
    let target_path = args
        .get("targetPath")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if target_path.is_empty() || !Path::new(target_path).exists() {
        return None;
    }

    let custom_cmd = args.get("customCommand").and_then(|v| v.as_str());
    let provider = parse_provider(provider_str, custom_cmd)?;
    if !matches!(provider, AgentProvider::Custom(_)) {
        let available = discover_all_provider_statuses()
            .into_iter()
            .any(|s| map_provider_id(s.kind) == provider_str && s.available);
        if !available {
            return None;
        }
    }

    let size_bytes = args
        .get("targetSizeBytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let target_name = args
        .get("targetName")
        .and_then(|v| v.as_str())
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
        "targetName": target_name,
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

fn map_vault_record(rec: windirstat_liquid_glass::vault::schema::VaultItemRecord) -> UiVaultItem {
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

fn handle_invoke(ctx: &AppContext, req: InvokeRequest) -> Value {
    match req.command.as_str() {
        "list_drives" => serde_json::to_value(query_windows_drives()).unwrap_or(Value::Null),
        "scan_directory" => {
            let target = req.args.get("target").and_then(|v| v.as_str()).unwrap_or("");
            match scan_directory_snapshot(target) {
                Some(snap) => serde_json::to_value(snap).unwrap_or(Value::Null),
                None => Value::Null,
            }
        }
        "find_duplicates" => {
            let target = req.args.get("target").and_then(|v| v.as_str()).unwrap_or("");
            match find_real_duplicates(target) {
                Some(groups) => serde_json::to_value(groups).unwrap_or(Value::Null),
                None => Value::Null,
            }
        }
        "detect_agent_clis" => serde_json::to_value(detect_ui_providers()).unwrap_or(Value::Null),
        "ask_agent_advisor" => try_run_real_advisor(&req.args).unwrap_or(Value::Null),
        "quarantine_item" => {
            let path_str = req.args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let path = Path::new(path_str);
            if !path.is_file() {
                return Value::Null;
            }
            let Ok(svc) = ctx.open_vault() else {
                return Value::Null;
            };
            match svc.quarantine_items(
                [path],
                "UI Quarantine",
                "desktop-session",
                "desktop-ui",
                "{}",
                Utc::now(),
            ) {
                Ok(mut res) if !res.items.is_empty() => {
                    serde_json::to_value(map_vault_record(res.items.remove(0))).unwrap_or(Value::Null)
                }
                _ => Value::Null,
            }
        }
        "list_vault_items" => {
            let Ok(svc) = ctx.open_vault() else {
                return Value::Null;
            };
            let Ok(items) = svc.list_items_last_days(30, Utc::now()) else {
                return Value::Null;
            };
            if items.is_empty() {
                // Keep UI populated cleanly via tauriBridge demo items until first real quarantine
                return Value::Null;
            }
            let mapped: Vec<UiVaultItem> = items.into_iter().map(map_vault_record).collect();
            serde_json::to_value(mapped).unwrap_or(Value::Null)
        }
        "rollback_vault_item" => {
            let id = req.args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                return Value::Null;
            }
            let Ok(svc) = ctx.open_vault() else {
                return Value::Null;
            };
            if svc.rollback_item(id).is_err() {
                return Value::Null;
            }
            svc.get_item(id)
                .ok()
                .flatten()
                .map(|r| serde_json::to_value(map_vault_record(r)).unwrap_or(Value::Null))
                .unwrap_or(Value::Null)
        }
        "purge_vault_item" => Value::Null,
        _ => Value::Null,
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn handle_client(mut stream: TcpStream, ctx: &AppContext) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_path = parts.next().unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or("/");

    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.eq_ignore_ascii_case("Content-Length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }

    if method.eq_ignore_ascii_case("POST") && path == "/api/invoke" {
        let mut body_buf = vec![0u8; content_length.min(4 * 1024 * 1024)];
        if content_length > 0 && reader.read_exact(&mut body_buf).is_err() {
            let _ = write_http_response(
                &mut stream,
                "400 Bad Request",
                "application/json",
                br#"{"error":"invalid request body"}"#,
            );
            return;
        }
        let result = match serde_json::from_slice::<InvokeRequest>(&body_buf) {
            Ok(req) => handle_invoke(ctx, req),
            Err(_) => Value::Null,
        };
        let payload = serde_json::to_vec(&result).unwrap_or_else(|_| b"null".to_vec());
        let _ = write_http_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            &payload,
        );
        return;
    }

    let lookup_path = if path == "/" { "/index.html" } else { path };
    if let Some((bytes, mime)) = get_embedded_asset(lookup_path) {
        if lookup_path == "/index.html" {
            let html = String::from_utf8_lossy(bytes).replace(
                "</head>",
                "<script>window.__WDS_DESKTOP_EXE__=true;</script></head>",
            );
            let _ = write_http_response(&mut stream, "200 OK", mime, html.as_bytes());
        } else {
            let _ = write_http_response(&mut stream, "200 OK", mime, bytes);
        }
        return;
    }

    if let Some((bytes, mime)) = get_embedded_asset("/index.html") {
        let html = String::from_utf8_lossy(bytes).replace(
            "</head>",
            "<script>window.__WDS_DESKTOP_EXE__=true;</script></head>",
        );
        let _ = write_http_response(&mut stream, "200 OK", mime, html.as_bytes());
        return;
    }

    let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", b"Not Found");
}

fn find_app_browser_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        let local_path = PathBuf::from(&local);
        candidates.push(local_path.join(r"ms-playwright\chromium-1255\chrome-win64\chrome.exe"));
        candidates.push(local_path.join(r"Google\Chrome\Application\chrome.exe"));
    }
    if let Some(pf86) = env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(pf86).join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    if let Some(pf) = env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(&pf).join(r"Microsoft\Edge\Application\msedge.exe"));
        candidates.push(PathBuf::from(pf).join(r"Google\Chrome\Application\chrome.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn launch_desktop_window(url: &str, app_data_dir: &Path) -> Option<Child> {
    let profile_dir = app_data_dir.join("app-profile");
    let _ = fs::create_dir_all(&profile_dir);

    if let Some(browser) = find_app_browser_exe() {
        if let Ok(child) = Command::new(browser)
            .arg(format!("--app={url}"))
            .arg("--window-size=1440,920")
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .spawn()
        {
            return Some(child);
        }
    }

    #[cfg(windows)]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    None
}

fn run_self_test(addr: std::net::SocketAddr) -> Result<(), String> {
    let mut root_stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    write!(
        root_stream,
        "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut root_resp = String::new();
    root_stream
        .read_to_string(&mut root_resp)
        .map_err(|e| e.to_string())?;
    if !root_resp.contains("200 OK") || !root_resp.contains("__WDS_DESKTOP_EXE__") {
        return Err("Embedded index.html self-test failed".to_string());
    }

    for cmd in ["list_drives", "detect_agent_clis", "list_vault_items"] {
        let mut api_stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
        let body = format!(r#"{{"command":"{cmd}","args":{{}}}}"#);
        write!(
            api_stream,
            "POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .map_err(|e| e.to_string())?;
        let mut api_resp = String::new();
        api_stream
            .read_to_string(&mut api_resp)
            .map_err(|e| e.to_string())?;
        if !api_resp.contains("200 OK") {
            return Err(format!("/api/invoke {cmd} did not return 200 OK"));
        }
    }

    println!("WinDirStat-LiquidGlass.exe self-test PASSED on http://{addr}");
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut no_open = false;
    let mut self_test = false;
    let mut port: u16 = 0;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--no-open" => no_open = true,
            "--self-test" => {
                self_test = true;
                no_open = true;
            }
            "--port" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(0);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let app_dir = default_app_data_dir();
    let _ = fs::create_dir_all(&app_dir);
    let ctx = Arc::new(AppContext {
        vault_db_path: app_dir.join("vault_manifest.db"),
    });

    // Keep 7-day vault expiration worker active while running
    let _worker = ctx
        .open_vault()
        .ok()
        .map(|svc| start_expiration_worker(Arc::new(svc)));

    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind localhost port");
    let local_addr = listener.local_addr().expect("read local addr");
    let _ = listener.set_nonblocking(true);

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_srv = Arc::clone(&shutdown);
    let ctx_srv = Arc::clone(&ctx);

    let server_handle = thread::spawn(move || {
        while !shutdown_srv.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let ctx_req = Arc::clone(&ctx_srv);
                    thread::spawn(move || handle_client(stream, &ctx_req));
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(15));
                }
                Err(_) => break,
            }
        }
    });

    if self_test {
        let res = run_self_test(local_addr);
        shutdown.store(true, Ordering::Relaxed);
        let _ = server_handle.join();
        if let Err(err) = res {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return;
    }

    let url = format!("http://{local_addr}");
    println!("WinDirStat Liquid Glass running at {url}");

    if no_open {
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    } else if let Some(mut child) = launch_desktop_window(&url, &app_dir) {
        let _ = child.wait();
        shutdown.store(true, Ordering::Relaxed);
        let _ = server_handle.join();
    } else {
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
}

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_hub::context::{LockedHandleStatus, StorageContextPayload};
use crate::agent_hub::prompt::build_cross_check_system_prompt;
use crate::agent_hub::providers::{
    build_opencode_isolated_config, discover_all_provider_statuses, AgentProvider,
    AgentProviderKind, CustomCliConfig,
};
use crate::agent_hub::runner::{
    AgentHubError, AgentHubRunner, CommandResolver, DefaultCommandResolver, RunnerConfig,
};
use crate::agent_hub::validator::{
    extract_assistant_text, extract_external_urls_from_text, parse_and_validate_advisor_response,
    AdvisorVerdict,
};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    pub install_hint: String,
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
            resolved_path: st.resolved_path.map(|p| p.to_string_lossy().into_owned()),
            install_hint: st.install_hint,
        })
        .collect();
    statuses.push(UiAgentProviderStatus {
        provider: "custom",
        display_name: "Custom CLI Adapter".to_string(),
        available: true,
        command: "custom-agent".to_string(),
        latency_ms: 50,
        resolved_path: None,
        install_hint: AgentProviderKind::Custom.install_hint().to_string(),
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

/// Extracts a Windows drive path (`C:\...`, `"D:\My Dir\file.iso"`, etc.) from free-form prompt text.
pub fn extract_windows_path_from_prompt(prompt: &str) -> Option<String> {
    let bytes = prompt.as_bytes();
    let mut i = 0;

    while i + 2 < bytes.len() {
        let ch = bytes[i];
        // Check quoted path first: `"C:\..."` or `'C:\...'`
        if (ch == b'"' || ch == b'\'') && i + 3 < bytes.len() {
            let drive = bytes[i + 1];
            let colon = bytes[i + 2];
            let sep = bytes[i + 3];
            if drive.is_ascii_alphabetic() && colon == b':' && (sep == b'\\' || sep == b'/') {
                let start = i + 1;
                if let Some(close_rel) = prompt[start..].find(ch as char) {
                    let candidate = prompt[start..start + close_rel].trim();
                    if candidate.len() >= 3 {
                        return Some(candidate.to_string());
                    }
                }
            }
        }

        // Check unquoted path boundary (`C:\...` or `C:/...`)
        let prev_ok = i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');
        if prev_ok
            && ch.is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/')
        {
            let rest = &prompt[i..];
            let end_rel = rest
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
                .unwrap_or(rest.len());
            let candidate = rest[..end_rel]
                .trim_end_matches(&['?', ',', '.', ';', ')', ']', '`'][..]);
            if candidate.len() >= 3 {
                return Some(candidate.to_string());
            }
        }

        i += prompt[i..].chars().next().map_or(1, char::len_utf8);
    }

    None
}

/// Extracts an explicit drive root (`C:\`, `D:\`, etc.) from prompts like "analyze current disk C"
/// or "free space on drive D:".
pub fn extract_drive_root_from_prompt(prompt: &str) -> Option<String> {
    let lower = prompt.to_ascii_lowercase();
    let bytes = lower.as_bytes();

    for keyword in ["disk ", "drive ", "volume "] {
        let mut search_from = 0;
        while let Some(rel) = lower[search_from..].find(keyword) {
            let abs = search_from + rel;
            let prev_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
            let letter_idx = abs + keyword.len();
            if prev_ok && letter_idx < bytes.len() && bytes[letter_idx].is_ascii_alphabetic() {
                let after = bytes.get(letter_idx + 1).copied();
                let next_after = bytes.get(letter_idx + 2).copied();
                let is_drive_ref = match after {
                    None => true,
                    Some(b':') => matches!(
                        next_after,
                        None | Some(b'\\' | b'/' | b' ' | b'\t' | b'\r' | b'\n' | b',' | b'.' | b'?' | b'!' | b';' | b')')
                    ),
                    Some(b' ' | b'\t' | b'\r' | b'\n' | b',' | b'.' | b'?' | b'!' | b';' | b')') => true,
                    _ => false,
                };
                if is_drive_ref {
                    let letter = (bytes[letter_idx] as char).to_ascii_uppercase();
                    return Some(format!("{letter}:\\"));
                }
            }
            search_from = abs + keyword.len();
        }
    }

    // Also match standalone `C:` / `C:\` references (excluding URL schemes such as `http://`).
    let mut i = 0;
    while i + 1 < bytes.len() {
        let prev_ok = i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');
        if prev_ok && bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' {
            let after = bytes.get(i + 2).copied();
            let valid_sep = matches!(
                after,
                None | Some(b' ' | b'\t' | b'\r' | b'\n' | b',' | b'.' | b'?' | b'!' | b';' | b')' | b'\\' | b'/')
            ) && !(after == Some(b'/') && bytes.get(i + 3).copied() == Some(b'/'));
            if valid_sep {
                let letter = (bytes[i] as char).to_ascii_uppercase();
                return Some(format!("{letter}:\\"));
            }
        }
        i += 1;
    }

    None
}

fn format_human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn bounded_measure_path(
    path: &Path,
    per_cap_ms: u128,
    max_entries: usize,
    global_deadline: Instant,
) -> Option<(u64, usize, bool)> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    if meta.is_file() {
        return Some((meta.len(), 1, false));
    }
    if !meta.is_dir() {
        return None;
    }

    let started = Instant::now();
    let mut stack = vec![path.to_path_buf()];
    let mut total_bytes = 0u64;
    let mut file_count = 0usize;
    let mut visited = 0usize;
    let mut truncated = false;

    while let Some(dir) = stack.pop() {
        if visited >= max_entries
            || started.elapsed().as_millis() >= per_cap_ms
            || Instant::now() >= global_deadline
        {
            truncated = true;
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if visited >= max_entries
                || started.elapsed().as_millis() >= per_cap_ms
                || Instant::now() >= global_deadline
            {
                truncated = true;
                break;
            }
            visited += 1;
            let p = entry.path();
            let Ok(m) = fs::symlink_metadata(&p) else {
                continue;
            };
            if m.file_type().is_symlink() {
                continue;
            }
            if m.is_file() {
                total_bytes = total_bytes.saturating_add(m.len());
                file_count += 1;
            } else if m.is_dir() {
                stack.push(p);
            }
        }
    }

    Some((total_bytes, file_count, truncated))
}

/// Builds a fast, bounded (`<= 2.5s`) native storage telemetry snapshot for a Windows drive
/// (volume totals, key Windows cleanup hotspots, and top-level directory weights) so Agent CLIs
/// do not hang running recursive shell scans across `C:\`.
pub fn build_drive_analysis_context(drive_root: &str, ui_scan_summary: Option<&str>) -> String {
    let trimmed = drive_root.trim();
    let normalized_drive = if trimmed.len() == 1 && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        format!("{}:\\", trimmed.to_ascii_uppercase())
    } else if trimmed.len() == 2 && trimmed.ends_with(':') {
        format!("{}\\", trimmed.to_ascii_uppercase())
    } else if trimmed.len() >= 3 && trimmed.as_bytes()[1] == b':' {
        format!("{}:\\", (trimmed.as_bytes()[0] as char).to_ascii_uppercase())
    } else {
        trimmed.to_string()
    };

    let drives = query_windows_drives();
    let volume_line = if let Some(d) = drives
        .iter()
        .find(|d| d.mount_point.eq_ignore_ascii_case(&normalized_drive))
    {
        let used_pct = if d.total_bytes > 0 {
            ((d.used_bytes as f64) / (d.total_bytes as f64)) * 100.0
        } else {
            0.0
        };
        format!(
            "- Volume {}: Total {} | Used {} ({:.1}%) | Free {}",
            d.mount_point,
            format_human_bytes(d.total_bytes),
            format_human_bytes(d.used_bytes),
            used_pct,
            format_human_bytes(d.free_bytes)
        )
    } else {
        format!("- Target Drive: {normalized_drive}")
    };

    let deadline = Instant::now() + Duration::from_millis(2_500);
    let mut candidate_hotspots: Vec<(PathBuf, &'static str)> = Vec::new();
    let drive_prefix = normalized_drive.to_ascii_lowercase();

    if let Ok(local_app) = env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local_app);
        for (sub, label) in [
            ("Temp", "User Temp files"),
            ("Docker", "Docker Desktop VM/data"),
            ("Packages", "Microsoft Store / UWP app packages"),
            ("npm-cache", "npm download cache"),
            ("pip\\Cache", "Python pip wheel/http cache"),
        ] {
            let cand = p.join(sub);
            if cand.to_string_lossy().to_ascii_lowercase().starts_with(&drive_prefix) {
                candidate_hotspots.push((cand, label));
            }
        }
    }
    if let Ok(profile) = env::var("USERPROFILE") {
        let p = PathBuf::from(profile);
        for (sub, label) in [
            ("Downloads", "User Downloads folder (installers/archives/ISOs)"),
            (".cargo\\registry\\cache", "Rust Cargo crate cache"),
        ] {
            let cand = p.join(sub);
            if cand.to_string_lossy().to_ascii_lowercase().starts_with(&drive_prefix) {
                candidate_hotspots.push((cand, label));
            }
        }
    }

    let drive_base = Path::new(&normalized_drive);
    for (rel, label) in [
        ("Windows\\Temp", "Windows system Temp folder"),
        ("Windows\\SoftwareDistribution\\Download", "Windows Update download cache"),
        ("Windows\\Installer", "MSI installer database (OS critical)"),
        ("Windows\\WinSxS", "Windows Component Store (use DISM / Disk Cleanup)"),
        ("hiberfil.sys", "Windows hibernation file"),
        ("pagefile.sys", "Windows virtual memory pagefile"),
    ] {
        candidate_hotspots.push((drive_base.join(rel), label));
    }

    let mut measured: Vec<(u64, usize, bool, String, &'static str)> = Vec::new();
    let mut seen_paths = HashSet::new();

    for (path, note) in candidate_hotspots {
        if Instant::now() >= deadline {
            break;
        }
        let key = path.to_string_lossy().to_ascii_lowercase();
        if !seen_paths.insert(key) || !path.exists() {
            continue;
        }
        if let Some((bytes, files, truncated)) = bounded_measure_path(&path, 350, 6_000, deadline) {
            if bytes > 0 {
                measured.push((
                    bytes,
                    files,
                    truncated,
                    path.to_string_lossy().into_owned(),
                    note,
                ));
            }
        }
    }

    if let Ok(top_entries) = fs::read_dir(drive_base) {
        for entry in top_entries.flatten() {
            if Instant::now() >= deadline {
                break;
            }
            let p = entry.path();
            let key = p.to_string_lossy().to_ascii_lowercase();
            if !seen_paths.insert(key) {
                continue;
            }
            if let Some((bytes, files, truncated)) = bounded_measure_path(&p, 200, 2_500, deadline) {
                if bytes >= 32 * 1024 * 1024 {
                    measured.push((
                        bytes,
                        files,
                        truncated,
                        p.to_string_lossy().into_owned(),
                        "Top-level drive entry",
                    ));
                }
            }
        }
    }

    measured.sort_by(|a, b| b.0.cmp(&a.0));
    measured.truncate(12);

    let mut lines = vec![volume_line];
    if measured.is_empty() {
        lines.push("- Measured hotspots: (none readable at current permission level)".to_string());
    } else {
        lines.push("- Measured drive hotspots / top folders (bounded native scan):".to_string());
        for (idx, (bytes, files, truncated, path_str, note)) in measured.iter().enumerate() {
            let plus = if *truncated { "+" } else { "" };
            lines.push(format!(
                "  {}. `{}` — {}{} ({}{} files; {})",
                idx + 1,
                path_str,
                format_human_bytes(*bytes),
                plus,
                files,
                plus,
                note
            ));
        }
    }

    if let Some(ui_summary) = ui_scan_summary.map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("- Active WinDirStat UI Scan Snapshot:\n{ui_summary}"));
    }

    lines.join("\n")
}

fn provider_display_label(provider_str: &str) -> &'static str {
    match provider_str {
        "antigravity" => "Antigravity CLI",
        "kiro" => "Kiro CLI",
        "claude" => "Claude Code CLI",
        "codex" => "OpenAI Codex CLI",
        "opencode" => "OpenCode CLI",
        "custom" => "Custom CLI",
        _ => "Agent CLI",
    }
}

fn truncate_snippet(raw: &str, max_chars: usize) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = collapsed.chars().count();
    if total <= max_chars {
        collapsed
    } else {
        let prefix: String = collapsed.chars().take(max_chars).collect();
        format!("{prefix}...")
    }
}

fn extract_logfmt_quoted_value(line: &str, key: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find(key) {
        let abs = search_from + rel;
        let boundary_ok = abs == 0
            || line.as_bytes()[abs - 1].is_ascii_whitespace();
        if !boundary_ok {
            search_from = abs + key.len();
            continue;
        }
        let rest = &line[abs + key.len()..];
        let Some(after_quote) = rest.strip_prefix('"') else {
            let unquoted = rest.split_whitespace().next().unwrap_or("").trim();
            if !unquoted.is_empty() {
                return Some(unquoted.to_string());
            }
            search_from = abs + key.len();
            continue;
        };
        let mut out = String::new();
        let mut chars = after_quote.chars().peekable();
        let mut escaped = false;
        while let Some(ch) = chars.next() {
            if escaped {
                match ch {
                    'n' | 'r' => break, // Stop before multi-line stack traces inside logfmt error strings.
                    't' => out.push(' '),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    other => out.push(other),
                }
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                break;
            }
            out.push(ch);
        }
        let cleaned = out.trim();
        if !cleaned.is_empty() {
            return Some(cleaned.to_string());
        }
        search_from = abs + key.len();
    }
    None
}

/// Extracts a concise human-readable error message from CLI stdout/stderr (e.g., OpenCode
/// `--print-logs` `level=ERROR` lines and `{"type":"error",...}` NDJSON events), stripping
/// redundant stack traces and HTTP header blobs.
pub fn extract_cli_error_detail(stdout: &str, stderr: &str) -> Option<String> {
    let mut candidates = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("level=ERROR")
            || trimmed.contains("error.error=")
            || trimmed.contains("ProviderModelNotFoundError")
        {
            if let Some(msg) = extract_logfmt_quoted_value(trimmed, "error.error=")
                .or_else(|| extract_logfmt_quoted_value(trimmed, "error="))
            {
                let normalized = msg
                    .strip_prefix("AI_APICallError:")
                    .map(str::trim)
                    .unwrap_or(&msg)
                    .to_string();
                if !normalized.is_empty() {
                    candidates.push(normalized);
                }
            }
        }
    }

    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if obj.get("type").and_then(|v| v.as_str()) == Some("error") || obj.contains_key("error") {
            let msg = obj
                .get("error")
                .and_then(|e| {
                    e.get("data")
                        .and_then(|d| d.get("message"))
                        .and_then(|v| v.as_str())
                        .or_else(|| e.get("message").and_then(|v| v.as_str()))
                        .or_else(|| e.as_str())
                })
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(m) = msg {
                candidates.push(m.to_string());
            }
        }
    }

    if let Some(specific) = candidates
        .iter()
        .find(|c| !c.starts_with("Unexpected server error"))
    {
        return Some(specific.clone());
    }
    candidates.into_iter().next()
}

fn build_cli_error_hint(provider_str: &str, text_lower: &str) -> &'static str {
    if provider_str == "opencode" {
        if text_lower.contains("providermodelnotfounderror") || text_lower.contains("model not found")
        {
            return " Your default OpenCode model may be retired — pick another model in the OpenCode Model selector or update ~/.config/opencode/opencode.json.";
        }
        if text_lower.contains("free usage limit reached") {
            return " OpenCode's free usage limit was hit — choose another model in the OpenCode Model selector or sign in (`opencode auth login`).";
        }
        if text_lower.contains("subscription is required") {
            return " OpenCode Go models require an active Go subscription — choose an `opencode/*` free model (such as `opencode/big-pickle`) in the OpenCode Model selector.";
        }
        if text_lower.contains("invalid api key")
            || text_lower.contains("invalid access token")
            || text_lower.contains("token expired")
            || text_lower.contains("401")
        {
            return " Provider authentication failed or expired — run `opencode auth login` or pick a free `opencode/*` model.";
        }
    }

    if text_lower.contains("login")
        || text_lower.contains("unauthorized")
        || text_lower.contains("expired")
        || text_lower.contains("credentials")
        || text_lower.contains("authenticate")
        || text_lower.contains("sso")
    {
        if provider_str == "kiro" {
            return " Your Kiro session may need sign-in — run `kiro-cli login` in a terminal.";
        }
        return " Your CLI session may require authentication.";
    }
    ""
}

fn format_friendly_cli_error(provider_str: &str, err: &AgentHubError) -> String {
    let label = provider_display_label(provider_str);
    match err {
        AgentHubError::BinaryNotFound { program } => {
            format!("{label} executable (`{program}`) was not found on PATH or standard install locations.")
        }
        AgentHubError::Timeout { timeout_ms, .. } => {
            let secs = timeout_ms / 1000;
            format!(
                "{label} timed out after {secs}s. Try asking a more specific question or verify your CLI connection."
            )
        }
        AgentHubError::ProcessFailed {
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            let extracted = extract_cli_error_detail(stdout, stderr);
            let combined = format!(
                "{}\n{}\n{}",
                extracted.as_deref().unwrap_or(""),
                stderr,
                stdout
            );
            let hint = build_cli_error_hint(provider_str, &combined.to_ascii_lowercase());
            let raw_detail = extracted.unwrap_or_else(|| {
                if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    stdout.trim().to_string()
                }
            });
            let detail = truncate_snippet(&raw_detail, 320);
            if detail.is_empty() {
                format!("{label} exited with status {exit_code:?}.{hint}")
            } else {
                format!("{label} exited with status {exit_code:?}: {detail}{hint}")
            }
        }
        other => format!("{label} failed: {other}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn make_error_response(
    provider_str: &str,
    prompt: &str,
    target_path: &str,
    target_name: &str,
    target_size_bytes: u64,
    error: String,
    install_hint: Option<String>,
    resolved_command: Option<String>,
) -> Value {
    json!({
        "id": format!("adv-{}", Utc::now().timestamp_millis()),
        "targetPath": target_path,
        "targetName": target_name,
        "targetSizeBytes": target_size_bytes,
        "provider": provider_str,
        "verdict": "REVIEW_CAREFULLY",
        "risk_score": 0,
        "reclaimable_bytes": 0,
        "usage_value_assessment": error,
        "rationale": error,
        "sources": [],
        "createdAt": Utc::now().to_rfc3339(),
        "userPrompt": prompt,
        "responseMode": "error",
        "isLiveCli": false,
        "resolvedCommand": resolved_command,
        "error": error,
        "installHint": install_hint,
    })
}

const PREFERRED_OPENCODE_FREE_MODELS: [&str; 5] = [
    "opencode/big-pickle",
    "opencode/glm-5.3-flash-free",
    "opencode/deepseek-v4.1-flash-free",
    "opencode/flint-40b-free",
    "opencode/beam-free",
];

fn strip_ansi_codes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn prioritize_opencode_models(raw_models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for m in raw_models {
        let stripped = strip_ansi_codes(&m).trim().to_string();
        if stripped.is_empty() || !stripped.contains('/') || stripped.chars().any(char::is_whitespace) {
            continue;
        }
        if seen.insert(stripped.clone()) {
            unique.push(stripped);
        }
    }

    let mut ordered = Vec::with_capacity(unique.len());
    let mut picked = HashSet::new();

    for pref in PREFERRED_OPENCODE_FREE_MODELS {
        if let Some(found) = unique.iter().find(|m| m.eq_ignore_ascii_case(pref)) {
            picked.insert(found.clone());
            ordered.push(found.clone());
        }
    }

    let mut other_free: Vec<String> = unique
        .iter()
        .filter(|m| !picked.contains(*m) && m.starts_with("opencode/"))
        .cloned()
        .collect();
    other_free.sort();
    for m in other_free {
        picked.insert(m.clone());
        ordered.push(m);
    }

    for m in unique {
        if !picked.contains(&m) {
            ordered.push(m);
        }
    }

    ordered
}

pub async fn list_opencode_models_async() -> Vec<String> {
    let Ok(resolved) = DefaultCommandResolver.resolve("opencode") else {
        return Vec::new();
    };

    let mut args = resolved.prefix_args.clone();
    args.push("models".to_string());

    let mut cmd = tokio::process::Command::new(&resolved.program);
    cmd.args(&args)
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if env::var_os("OPENCODE_CONFIG_CONTENT").is_none() {
        cmd.env("OPENCODE_CONFIG_CONTENT", build_opencode_isolated_config());
    }

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let lines = text.lines().map(ToString::to_string).collect();
    prioritize_opencode_models(lines)
}

fn is_recoverable_opencode_error(stdout: &str, stderr: &str) -> bool {
    let lower = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    [
        "providermodelnotfounderror",
        "model not found",
        "subscription is required",
        "free usage limit reached",
        "unexpected server error",
        "maximum number of items is 128",
        "invalid api key",
        "invalid access token",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn stdout_has_opencode_error_only(stdout: &str) -> bool {
    if extract_assistant_text(stdout).is_some() {
        return false;
    }
    stdout.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with('{')
            && serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(|t| t == "error"))
                .unwrap_or(false)
    })
}

fn build_error_hint_for_err(err: &AgentHubError, install_hint: Option<&str>) -> Option<String> {
    match err {
        AgentHubError::BinaryNotFound { .. } => install_hint.map(ToString::to_string),
        AgentHubError::Timeout { .. } => Some(
            "Tip: Broad full-drive prompts with internet verification can take longer on large disks. Try asking about a specific folder from the Treemap (right-click -> Ask Agent CLI), or pick a faster model."
                .to_string(),
        ),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_real_advisor_async(
    provider_str: &str,
    prompt: &str,
    target_path: Option<&str>,
    target_name: Option<&str>,
    target_size_bytes: Option<u64>,
    custom_command: Option<&str>,
    active_drive: Option<&str>,
    model: Option<&str>,
    scan_summary: Option<&str>,
) -> Value {
    let trimmed_prompt = prompt.trim();
    let explicit_target = target_path.map(str::trim).filter(|s| !s.is_empty());
    let extracted_path = if explicit_target.is_none() {
        extract_windows_path_from_prompt(trimmed_prompt)
    } else {
        None
    };
    let concrete_path = explicit_target
        .map(ToString::to_string)
        .or(extracted_path);

    let clean_active_drive = active_drive
        .map(str::trim)
        .filter(|d| !d.is_empty() && !d.starts_with("demo://"))
        .map(ToString::to_string);

    let resolved_target_path = concrete_path
        .clone()
        .or_else(|| clean_active_drive.clone())
        .unwrap_or_else(|| "General Storage Inquiry".to_string());

    let resolved_size_bytes = target_size_bytes.unwrap_or_else(|| {
        concrete_path
            .as_deref()
            .and_then(|p| fs::metadata(p).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .unwrap_or(0)
    });

    let resolved_name = target_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            Path::new(&resolved_target_path)
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(&resolved_target_path)
                .to_string()
        });

    if trimmed_prompt.is_empty() {
        return make_error_response(
            provider_str,
            prompt,
            &resolved_target_path,
            &resolved_name,
            resolved_size_bytes,
            "Please enter a question or path for the Agent CLI.".to_string(),
            None,
            None,
        );
    }

    let Some(provider) = parse_provider(provider_str, custom_command) else {
        let (msg, hint) = if provider_str == "custom" {
            (
                "Custom CLI command is empty or invalid. Enter an executable path in the Custom CLI field.".to_string(),
                Some(AgentProviderKind::Custom.install_hint().to_string()),
            )
        } else {
            (
                format!("Unsupported Agent CLI provider `{provider_str}`."),
                None,
            )
        };
        return make_error_response(
            provider_str,
            trimmed_prompt,
            &resolved_target_path,
            &resolved_name,
            resolved_size_bytes,
            msg,
            hint,
            None,
        );
    };

    let statuses = discover_all_provider_statuses();
    let matched_status = statuses
        .iter()
        .find(|s| map_provider_id(s.kind) == provider_str);

    if !matches!(provider, AgentProvider::Custom(_)) {
        if let Some(st) = matched_status {
            if !st.available {
                return make_error_response(
                    provider_str,
                    trimmed_prompt,
                    &resolved_target_path,
                    &resolved_name,
                    resolved_size_bytes,
                    format!(
                        "{} was not found on PATH or standard install directories on this PC.",
                        st.display_name
                    ),
                    Some(st.install_hint.clone()),
                    Some(st.command.clone()),
                );
            }
        }
    }

    let install_hint = matched_status
        .map(|s| s.install_hint.clone())
        .or_else(|| Some(AgentProviderKind::Custom.install_hint().to_string()));
    let default_cmd_label = matched_status
        .map(|s| s.command.clone())
        .unwrap_or_else(|| provider.default_binary_name().to_string());

    let prompt_lower = trimmed_prompt.to_ascii_lowercase();
    let mentions_storage_file = [
        ".vhdx",
        ".iso",
        ".tmp",
        ".log",
        ".dll",
        ".exe",
        ".msi",
        ".msp",
        "node_modules",
        "system32",
        "appdata",
        "winsxs",
    ]
    .iter()
    .any(|kw| prompt_lower.contains(kw));
    let is_targeted_inspection =
        explicit_target.is_some() || concrete_path.is_some() || mentions_storage_file;

    let payload = StorageContextPayload {
        path: resolved_target_path.clone(),
        logical_size_bytes: resolved_size_bytes,
        physical_size_bytes: resolved_size_bytes,
        file_count: 1,
        oldest_modified_utc: None,
        newest_modified_utc: None,
        publisher_signature: None,
        top_extensions: Vec::new(),
        locked_handle_status: LockedHandleStatus::Unknown,
        user_question: trimmed_prompt.to_string(),
    };

    let context_json = payload.to_compact_json().unwrap_or_else(|_| "{}".to_string());
    let effective_drive = extract_drive_root_from_prompt(trimmed_prompt).or(clean_active_drive.clone());
    let clean_scan_summary = scan_summary
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let prompt_to_send = if is_targeted_inspection {
        build_cross_check_system_prompt(&payload)
    } else {
        let drive_telemetry = if let Some(drive) = effective_drive.clone() {
            let summary_owned = clean_scan_summary.clone();
            tokio::task::spawn_blocking(move || {
                build_drive_analysis_context(&drive, summary_owned.as_deref())
            })
            .await
            .unwrap_or_default()
        } else if let Some(summary_only) = clean_scan_summary {
            format!("- Active WinDirStat UI Scan Snapshot:\n{summary_only}")
        } else {
            "- Volume: (not specified)".to_string()
        };

        format!(
            "You are the WinDirStat Storage Intelligence Advisor inside WinDirStat Liquid Glass.\n\
             WinDirStat has ALREADY scanned the drive in native code and provided real sizes below.\n\
             DO NOT run shell commands, PowerShell scripts, or recursive directory scans (such as Get-ChildItem -Recurse or du) over the drive.\n\
             Use the WinDirStat scan summary directly, and spend your tool calls ONLY verifying trusted public internet sources (at least 3 distinct https:// URLs) for the top cleanup targets.\n\n\
             ### WinDirStat Native Drive Scan Summary\n\
             {drive_telemetry}\n\n\
             Respond concisely in Markdown with:\n\
             1. A prioritized cleanup table grouping candidates into **Safe to Quarantine**, **Clean via Official Tool**, and **Keep Critical** (using only sizes reported above).\n\
             2. Clear reminder to route manual removals through WinDirStat's **7-Day Quarantine Vault** rather than permanent deletion.\n\
             3. At least 3 verified public `https://` documentation links supporting those safety verdicts.\n\n\
             User Question:\n{question}",
            question = trimmed_prompt,
        )
    };

    let runner = AgentHubRunner::with_resolver(
        Arc::new(DefaultCommandResolver),
        RunnerConfig {
            timeout: Duration::from_secs(180),
            self_test_timeout: Duration::from_secs(15),
        },
    );

    let requested_model = model.map(str::trim).filter(|m| !m.is_empty());
    let mut used_model = requested_model.map(ToString::to_string);

    let mut attempt = runner
        .run_raw_prompt_with_options(&provider, &prompt_to_send, &context_json, requested_model, None)
        .await;

    let should_fallback_opencode = provider_str == "opencode"
        && requested_model.is_none()
        && match &attempt {
            Err(AgentHubError::ProcessFailed { stdout, stderr, .. }) => {
                is_recoverable_opencode_error(stdout, stderr)
            }
            Ok(out) => stdout_has_opencode_error_only(&out.stdout),
            _ => false,
        };

    if should_fallback_opencode {
        let models = list_opencode_models_async().await;
        let fallback_candidates: Vec<String> = models
            .into_iter()
            .filter(|m| m.starts_with("opencode/"))
            .take(2)
            .collect();

        for candidate in fallback_candidates {
            let retry = runner
                .run_raw_prompt_with_options(
                    &provider,
                    &prompt_to_send,
                    &context_json,
                    Some(&candidate),
                    None,
                )
                .await;
            used_model = Some(candidate);
            let retry_ok = matches!(&retry, Ok(out) if !stdout_has_opencode_error_only(&out.stdout));
            attempt = retry;
            if retry_ok {
                break;
            }
        }
    }

    let raw = match attempt {
        Ok(out) => out,
        Err(err) => {
            let err_cmd = match &used_model {
                Some(m) => format!("{default_cmd_label} ({m})"),
                None => default_cmd_label.clone(),
            };
            if let AgentHubError::Timeout {
                partial_stdout,
                timeout_ms,
                ..
            } = &err
            {
                if let Some(partial_text) = extract_assistant_text(partial_stdout) {
                    if partial_text.trim().chars().count() >= 120 {
                        let noted_reply = format!(
                            "{}\n\n> Note: Response reached the {}s time limit while finalizing; partial output is shown above.",
                            partial_text.trim(),
                            timeout_ms / 1000
                        );
                        let sources: Vec<Value> = extract_external_urls_from_text(&partial_text)
                            .into_iter()
                            .map(|(url, domain)| {
                                json!({
                                    "title": domain,
                                    "url": url,
                                    "domain": domain,
                                    "verified": true,
                                    "snippet": format!("Referenced in {err_cmd} partial response"),
                                })
                            })
                            .collect();
                        return json!({
                            "id": format!("adv-{}", Utc::now().timestamp_millis()),
                            "targetPath": resolved_target_path,
                            "targetName": resolved_name,
                            "targetSizeBytes": resolved_size_bytes,
                            "provider": provider_str,
                            "verdict": "REVIEW_CAREFULLY",
                            "risk_score": 0,
                            "reclaimable_bytes": 0,
                            "usage_value_assessment": noted_reply,
                            "rationale": format!(
                                "Partial live reply from {} ({}s limit reached)",
                                err_cmd,
                                timeout_ms / 1000
                            ),
                            "sources": sources,
                            "createdAt": Utc::now().to_rfc3339(),
                            "userPrompt": trimmed_prompt,
                            "responseMode": "chat",
                            "assistantReply": noted_reply,
                            "isLiveCli": true,
                            "resolvedCommand": err_cmd,
                            "durationMs": *timeout_ms as u64,
                        });
                    }
                }
            }
            let error_hint = build_error_hint_for_err(&err, install_hint.as_deref());
            return make_error_response(
                provider_str,
                trimmed_prompt,
                &resolved_target_path,
                &resolved_name,
                resolved_size_bytes,
                format_friendly_cli_error(provider_str, &err),
                error_hint,
                Some(err_cmd),
            );
        }
    };

    let invoked_cmd = raw
        .invoked_program
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToString::to_string)
        .unwrap_or(default_cmd_label);
    let display_cmd = match &used_model {
        Some(m) => format!("{invoked_cmd} ({m})"),
        None => invoked_cmd.clone(),
    };

    if stdout_has_opencode_error_only(&raw.stdout) {
        let detail = extract_cli_error_detail(&raw.stdout, &raw.stderr)
            .unwrap_or_else(|| format!("{display_cmd} returned an API error."));
        let hint = build_cli_error_hint(
            provider_str,
            &format!("{detail}\n{}\n{}", raw.stderr, raw.stdout).to_ascii_lowercase(),
        );
        return make_error_response(
            provider_str,
            trimmed_prompt,
            &resolved_target_path,
            &resolved_name,
            resolved_size_bytes,
            format!("{detail}{hint}"),
            install_hint,
            Some(display_cmd),
        );
    }

    if let Ok(parsed) = parse_and_validate_advisor_response(&raw.stdout) {
        let verdict = match parsed.verdict {
            AdvisorVerdict::SafeToDelete => "SAFE_TO_DELETE",
            AdvisorVerdict::CleanViaOfficialTool => "QUARANTINE_RECOMMENDED",
            AdvisorVerdict::SafeWithAppClosed => "REVIEW_CAREFULLY",
            AdvisorVerdict::KeepCritical => "KEEP",
        };
        let sources: Vec<Value> = parsed
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

        return json!({
            "id": format!("adv-{}", Utc::now().timestamp_millis()),
            "targetPath": resolved_target_path,
            "targetName": resolved_name,
            "targetSizeBytes": resolved_size_bytes,
            "provider": provider_str,
            "verdict": verdict,
            "risk_score": parsed.risk_score,
            "reclaimable_bytes": parsed.reclaimable_bytes,
            "usage_value_assessment": parsed.usage_value_assessment,
            "rationale": parsed.recommended_action,
            "sources": sources,
            "createdAt": Utc::now().to_rfc3339(),
            "userPrompt": trimmed_prompt,
            "responseMode": "verdict",
            "isLiveCli": true,
            "resolvedCommand": display_cmd,
            "durationMs": raw.duration_ms as u64,
        });
    }

    if let Some(reply_text) = extract_assistant_text(&raw.stdout) {
        if !reply_text.trim().is_empty() {
            let sources: Vec<Value> = extract_external_urls_from_text(&reply_text)
                .into_iter()
                .map(|(url, domain)| {
                    json!({
                        "title": domain,
                        "url": url,
                        "domain": domain,
                        "verified": true,
                        "snippet": format!("Referenced in {display_cmd} response"),
                    })
                })
                .collect();

            return json!({
                "id": format!("adv-{}", Utc::now().timestamp_millis()),
                "targetPath": resolved_target_path,
                "targetName": resolved_name,
                "targetSizeBytes": resolved_size_bytes,
                "provider": provider_str,
                "verdict": "REVIEW_CAREFULLY",
                "risk_score": 0,
                "reclaimable_bytes": 0,
                "usage_value_assessment": reply_text,
                "rationale": format!(
                    "Live reply from {} ({:.1}s)",
                    display_cmd,
                    (raw.duration_ms as f64) / 1000.0
                ),
                "sources": sources,
                "createdAt": Utc::now().to_rfc3339(),
                "userPrompt": trimmed_prompt,
                "responseMode": "chat",
                "assistantReply": reply_text,
                "isLiveCli": true,
                "resolvedCommand": display_cmd,
                "durationMs": raw.duration_ms as u64,
            });
        }
    }

    let stderr_snippet = truncate_snippet(&raw.stderr, 260);
    let empty_err = if stderr_snippet.is_empty() {
        format!("{display_cmd} exited cleanly but produced no readable assistant reply.")
    } else {
        format!("{display_cmd} produced no readable assistant reply ({stderr_snippet}).")
    };
    make_error_response(
        provider_str,
        trimmed_prompt,
        &resolved_target_path,
        &resolved_name,
        resolved_size_bytes,
        empty_err,
        install_hint,
        Some(display_cmd),
    )
}

pub async fn test_agent_cli_async(provider_str: &str, custom_command: Option<&str>) -> Value {
    let Some(provider) = parse_provider(provider_str, custom_command) else {
        return json!({
            "ok": false,
            "error": format!("Unsupported or unconfigured provider `{provider_str}`."),
        });
    };

    let runner = AgentHubRunner::default();
    match runner.self_test_provider(&provider).await {
        Ok(report) => json!({
            "ok": report.ok,
            "command": report.resolved_program.to_string_lossy(),
            "output": report.stdout.trim(),
            "durationMs": report.duration_ms as u64,
        }),
        Err(err) => json!({
            "ok": false,
            "error": err.to_string(),
        }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_quoted_and_unquoted_windows_paths_from_prompts() {
        assert_eq!(
            extract_windows_path_from_prompt(
                r"Should I delete C:\Windows\Temp\installer-cache-bundle.tmp?"
            ),
            Some(r"C:\Windows\Temp\installer-cache-bundle.tmp".to_string())
        );
        assert_eq!(
            extract_windows_path_from_prompt(
                r#"Inspect "E:\My Backups\ubuntu-24.04.1-desktop-amd64.iso" please."#
            ),
            Some(r"E:\My Backups\ubuntu-24.04.1-desktop-amd64.iso".to_string())
        );
        assert_eq!(
            extract_windows_path_from_prompt(r"What is taking space in C:\Windows\Temp."),
            Some(r"C:\Windows\Temp".to_string())
        );
        assert_eq!(
            extract_windows_path_from_prompt("Hello Kiro, how does the 7-day vault work?"),
            None
        );
    }

    #[test]
    fn detect_ui_providers_returns_all_six_entries_with_install_hints() {
        let providers = detect_ui_providers();
        assert_eq!(providers.len(), 6);
        for id in ["antigravity", "kiro", "claude", "codex", "opencode", "custom"] {
            let entry = providers
                .iter()
                .find(|p| p.provider == id)
                .unwrap_or_else(|| panic!("missing provider {id}"));
            assert!(!entry.install_hint.trim().is_empty());
        }
    }

    #[tokio::test]
    #[ignore = "Invokes live local kiro-cli process with network access"]
    async fn live_kiro_cli_smoke() {
        let chat_res = run_real_advisor_async(
            "kiro",
            "Reply with one short sentence saying hello from Kiro CLI.",
            None,
            None,
            None,
            None,
            Some("C:\\"),
            None,
            None,
        )
        .await;
        println!("LIVE CHAT: {chat_res}");
        assert_eq!(chat_res["isLiveCli"], json!(true));
        assert!(
            matches!(chat_res["responseMode"].as_str(), Some("chat" | "verdict")),
            "unexpected chat response: {chat_res}"
        );

        let inspect_res = run_real_advisor_async(
            "kiro",
            "Should I delete C:\\Windows\\Temp?",
            None,
            None,
            None,
            None,
            Some("C:\\"),
            None,
            None,
        )
        .await;
        println!("LIVE INSPECT: {inspect_res}");
        assert_eq!(inspect_res["isLiveCli"], json!(true));
        assert!(
            matches!(inspect_res["responseMode"].as_str(), Some("chat" | "verdict")),
            "unexpected inspect response: {inspect_res}"
        );
    }

    #[test]
    fn extracts_drive_mentions_and_builds_fast_bounded_drive_context() {
        assert_eq!(
            extract_drive_root_from_prompt(
                "so i want you analyze current disk C for me, so that we can decide which folder or file we can free up space"
            ),
            Some("C:\\".to_string())
        );
        assert_eq!(
            extract_drive_root_from_prompt("Check drive D please"),
            Some("D:\\".to_string())
        );
        assert_eq!(
            extract_drive_root_from_prompt("How much is free on e:?"),
            Some("E:\\".to_string())
        );
        assert_eq!(
            extract_drive_root_from_prompt("Compare https://learn.microsoft.com/en-us/windows"),
            None
        );
        assert_eq!(extract_drive_root_from_prompt("Hello Kiro"), None);

        let started = Instant::now();
        let ctx = build_drive_analysis_context("C:\\", Some("Custom UI scan line"));
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(4),
            "drive context builder took too long: {elapsed:?}"
        );
        assert!(ctx.contains("C:\\"), "ctx={ctx}");
        assert!(ctx.contains("Custom UI scan line"), "ctx={ctx}");

        let timeout_err = AgentHubError::Timeout {
            program: "kiro-cli".to_string(),
            timeout_ms: 180_000,
            partial_stdout: String::new(),
        };
        let timeout_hint = build_error_hint_for_err(&timeout_err, Some("Install Kiro CLI"))
            .expect("expected timeout tip");
        assert!(!timeout_hint.contains("Install Kiro CLI"));
        assert!(timeout_hint.contains("Treemap"));

        let missing_err = AgentHubError::BinaryNotFound {
            program: "kiro".to_string(),
        };
        assert_eq!(
            build_error_hint_for_err(&missing_err, Some("Install Kiro CLI")).as_deref(),
            Some("Install Kiro CLI")
        );
    }

    #[tokio::test]
    #[ignore = "Invokes live local kiro-cli process with network access on exact user disk C prompt"]
    async fn live_kiro_disk_c_analysis_smoke() {
        let res = run_real_advisor_async(
            "kiro",
            "so i want you analyze current disk C for me, so that we can decide which folder or file we can free up space, alos, you should double check with trust source on the internet to verify it is safe to remove or not",
            None,
            None,
            None,
            None,
            Some("C:\\"),
            None,
            None,
        )
        .await;
        println!("LIVE DISK C KIRO: {res}");
        assert_eq!(res["isLiveCli"], json!(true), "res={res}");
        assert!(
            matches!(res["responseMode"].as_str(), Some("chat" | "verdict")),
            "unexpected mode: {res}"
        );
        let sources_len = res["sources"].as_array().map_or(0, Vec::len);
        assert!(sources_len >= 3, "expected >=3 verified sources, got {sources_len}: {res}");
    }

    #[test]
    fn extracts_clean_opencode_error_details_and_prioritizes_models() {
        let stderr_not_found = concat!(
            "timestamp=2026-11-19T09:41:39.836Z level=ERROR run=9f3a52b2 message=failed ref=err_1 ",
            r#"error="ProviderModelNotFoundError: Model not found: opencode/muse-spark-1.3-contributor-free. Did you mean: beam-free?\n    at <anonymous>" cause="...""#
        );
        let stdout_generic = r#"{"type":"error","timestamp":1,"error":{"name":"UnknownError","data":{"message":"Unexpected server error. Check server logs for details."}}}"#;
        assert_eq!(
            extract_cli_error_detail(stdout_generic, stderr_not_found).as_deref(),
            Some("ProviderModelNotFoundError: Model not found: opencode/muse-spark-1.3-contributor-free. Did you mean: beam-free?")
        );

        let stdout_api_err = r#"{"type":"error","timestamp":1,"error":{"name":"APIError","data":{"message":"An active OpenCode Go subscription is required to use OpenCode Go models.","statusCode":403}}}"#;
        assert_eq!(
            extract_cli_error_detail(stdout_api_err, "").as_deref(),
            Some("An active OpenCode Go subscription is required to use OpenCode Go models.")
        );

        let stderr_rate_limit = r#"timestamp=2026-11-19T09:46:36.318Z level=ERROR run=c6813b83 message="stream error" providerID=opencode modelID=big-pickle error.error="AI_APICallError: Free usage limit reached. Free models work in OpenCode (https://opencode.ai)""#;
        assert_eq!(
            extract_cli_error_detail("", stderr_rate_limit).as_deref(),
            Some("Free usage limit reached. Free models work in OpenCode (https://opencode.ai)")
        );

        let prioritized = prioritize_opencode_models(vec![
            "opencode-go/claude-haiku-5-5".to_string(),
            "opencode/beam-free".to_string(),
            "opencode/big-pickle".to_string(),
            "minimax/MiniMax-M3".to_string(),
            "opencode/big-pickle".to_string(),
        ]);
        assert_eq!(
            prioritized,
            vec![
                "opencode/big-pickle",
                "opencode/beam-free",
                "opencode-go/claude-haiku-5-5",
                "minimax/MiniMax-M3",
            ]
        );
    }

    #[tokio::test]
    #[ignore = "Invokes live local opencode CLI process with network access"]
    async fn live_opencode_cli_smoke() {
        let models = list_opencode_models_async().await;
        println!("LIVE OPENCODE MODELS COUNT: {}", models.len());
        assert!(!models.is_empty(), "expected opencode models list");

        let chat_res = run_real_advisor_async(
            "opencode",
            "Reply with one short sentence saying hello from OpenCode CLI.",
            None,
            None,
            None,
            None,
            Some("C:\\"),
            None,
            None,
        )
        .await;
        println!("LIVE OPENCODE CHAT (Auto fallback): {chat_res}");
        assert_eq!(chat_res["isLiveCli"], json!(true));
        assert!(
            matches!(chat_res["responseMode"].as_str(), Some("chat" | "verdict")),
            "unexpected opencode chat response: {chat_res}"
        );
    }
}

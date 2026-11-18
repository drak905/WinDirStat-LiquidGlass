pub mod pipe_server;
pub mod snapshot_bridge;

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\wds_liquid_glass";
pub const MSGPACK_CHUNK_MAGIC: [u8; 4] = *b"WDSM";
pub const TARGET_FRAME_INTERVAL_MS: u64 = 16;
pub const DEFAULT_FRAME_BATCH_SIZE: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum IpcBridgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse/serialize error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MessagePack decode error: {0}")]
    MsgPackDecode(#[from] rmp_serde::decode::Error),
    #[error("MessagePack encode error: {0}")]
    MsgPackEncode(#[from] rmp_serde::encode::Error),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Snapshot error: {0}")]
    Snapshot(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanNode {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub logical_size: u64,
    pub physical_size: u64,
    #[serde(default)]
    pub modified_utc: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub xxhash64: Option<u64>,
    #[serde(default)]
    pub extension: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    Nodes,
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcStreamFrame {
    pub frame_type: FrameKind,
    pub seq: u64,
    #[serde(default)]
    pub scan_root: String,
    #[serde(default)]
    pub scanned_nodes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub nodes: Vec<ScanNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub xxhash64: u64,
    pub hash_hex: String,
    pub file_size: u64,
    pub count: usize,
    pub wasted_bytes: u64,
    pub files: Vec<ScanNode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub scan_root: String,
    pub total_nodes: usize,
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_logical_bytes: u64,
    pub total_physical_bytes: u64,
    pub duplicate_groups: usize,
    pub total_wasted_bytes: u64,
}

pub trait ScanIngestor: Send + Sync {
    fn on_frame(&self, frame: IpcStreamFrame) -> Result<(), IpcBridgeError>;
    fn on_complete(&self, summary: SnapshotSummary) -> Result<(), IpcBridgeError>;
}

#[derive(Debug, Default)]
pub struct MemoryScanIngestor {
    state: Mutex<MemoryIngestorState>,
}

#[derive(Debug, Default)]
struct MemoryIngestorState {
    frames: Vec<IpcStreamFrame>,
    nodes: Vec<ScanNode>,
    summary: Option<SnapshotSummary>,
}

impl MemoryScanIngestor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frames(&self) -> Vec<IpcStreamFrame> {
        self.state.lock().expect("ingestor mutex poisoned").frames.clone()
    }

    pub fn nodes(&self) -> Vec<ScanNode> {
        self.state.lock().expect("ingestor mutex poisoned").nodes.clone()
    }

    pub fn summary(&self) -> Option<SnapshotSummary> {
        self.state.lock().expect("ingestor mutex poisoned").summary.clone()
    }
}

impl ScanIngestor for MemoryScanIngestor {
    fn on_frame(&self, frame: IpcStreamFrame) -> Result<(), IpcBridgeError> {
        let mut guard = self.state.lock().expect("ingestor mutex poisoned");
        guard.nodes.extend(frame.nodes.iter().cloned());
        guard.frames.push(frame);
        Ok(())
    }

    fn on_complete(&self, summary: SnapshotSummary) -> Result<(), IpcBridgeError> {
        let mut guard = self.state.lock().expect("ingestor mutex poisoned");
        guard.summary = Some(summary);
        Ok(())
    }
}

pub fn summarize_nodes(
    scan_root: &str,
    nodes: &[ScanNode],
    duplicates: &[DuplicateGroup],
) -> SnapshotSummary {
    let mut total_files = 0u64;
    let mut total_dirs = 0u64;
    let mut total_logical_bytes = 0u64;
    let mut total_physical_bytes = 0u64;

    for node in nodes {
        if node.is_dir {
            total_dirs += 1;
        } else {
            total_files += 1;
            total_logical_bytes = total_logical_bytes.saturating_add(node.logical_size);
            total_physical_bytes = total_physical_bytes.saturating_add(node.physical_size);
        }
    }

    let total_wasted_bytes = duplicates
        .iter()
        .fold(0u64, |acc, group| acc.saturating_add(group.wasted_bytes));

    SnapshotSummary {
        scan_root: scan_root.to_string(),
        total_nodes: nodes.len(),
        total_files,
        total_dirs,
        total_logical_bytes,
        total_physical_bytes,
        duplicate_groups: duplicates.len(),
        total_wasted_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_nodes_aggregates_files_dirs_and_waste() {
        let nodes = vec![
            ScanNode {
                id: 1,
                parent_id: None,
                name: "C:\\Root".into(),
                full_path: "C:\\Root".into(),
                is_dir: true,
                logical_size: 300,
                physical_size: 4096,
                modified_utc: String::new(),
                owner: String::new(),
                xxhash64: None,
                extension: String::new(),
            },
            ScanNode {
                id: 2,
                parent_id: Some(1),
                name: "a.bin".into(),
                full_path: "C:\\Root\\a.bin".into(),
                is_dir: false,
                logical_size: 150,
                physical_size: 4096,
                modified_utc: String::new(),
                owner: String::new(),
                xxhash64: Some(0xABCD),
                extension: ".bin".into(),
            },
            ScanNode {
                id: 3,
                parent_id: Some(1),
                name: "b.bin".into(),
                full_path: "C:\\Root\\b.bin".into(),
                is_dir: false,
                logical_size: 150,
                physical_size: 4096,
                modified_utc: String::new(),
                owner: String::new(),
                xxhash64: Some(0xABCD),
                extension: ".bin".into(),
            },
        ];
        let dupes = vec![DuplicateGroup {
            xxhash64: 0xABCD,
            hash_hex: "000000000000abcd".into(),
            file_size: 150,
            count: 2,
            wasted_bytes: 150,
            files: vec![nodes[1].clone(), nodes[2].clone()],
        }];

        let summary = summarize_nodes("C:\\Root", &nodes, &dupes);
        assert_eq!(summary.total_nodes, 3);
        assert_eq!(summary.total_dirs, 1);
        assert_eq!(summary.total_files, 2);
        assert_eq!(summary.total_logical_bytes, 300);
        assert_eq!(summary.total_physical_bytes, 8192);
        assert_eq!(summary.duplicate_groups, 1);
        assert_eq!(summary.total_wasted_bytes, 150);
    }
}

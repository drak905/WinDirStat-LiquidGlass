use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned when validating or serializing a [`StorageContextPayload`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContextError {
    #[error("storage path must not be empty")]
    EmptyPath,
    #[error("user question must not be empty")]
    EmptyUserQuestion,
    #[error("failed to serialize storage context payload: {0}")]
    Serialization(String),
}

/// File lock / open handle inspection status for the target path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LockedHandleStatus {
    /// No open handles detected on the target path.
    Unlocked,
    /// One or more active processes currently hold open handles on the path.
    Locked { locking_processes: Vec<String> },
    /// Handle status could not be determined or was not queried.
    #[default]
    Unknown,
}

impl LockedHandleStatus {
    /// Returns a human-readable summary of the lock status.
    pub fn describe(&self) -> String {
        match self {
            LockedHandleStatus::Unlocked => {
                "Unlocked (no open file handles detected)".to_string()
            }
            LockedHandleStatus::Locked { locking_processes } => {
                if locking_processes.is_empty() {
                    "Locked (active handles detected; process names unavailable)".to_string()
                } else {
                    format!("Locked by active processes: {}", locking_processes.join(", "))
                }
            }
            LockedHandleStatus::Unknown => {
                "Unknown (open-handle inspection was not run)".to_string()
            }
        }
    }
}

/// Summary metrics for one file extension inside an inspected directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionStat {
    pub extension: String,
    pub file_count: u64,
    pub total_bytes: u64,
}

impl ExtensionStat {
    pub fn new(extension: impl Into<String>, file_count: u64, total_bytes: u64) -> Self {
        Self {
            extension: extension.into(),
            file_count,
            total_bytes,
        }
    }
}

/// Structured WinDirStat scan metadata injected into the Agent CLI prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageContextPayload {
    pub path: String,
    pub logical_size_bytes: u64,
    pub physical_size_bytes: u64,
    pub file_count: u64,
    pub oldest_modified_utc: Option<String>,
    pub newest_modified_utc: Option<String>,
    pub publisher_signature: Option<String>,
    pub top_extensions: Vec<ExtensionStat>,
    pub locked_handle_status: LockedHandleStatus,
    pub user_question: String,
}

impl StorageContextPayload {
    /// Validates required fields before dispatching to an Agent CLI.
    pub fn validate(&self) -> Result<(), ContextError> {
        if self.path.trim().is_empty() {
            return Err(ContextError::EmptyPath);
        }
        if self.user_question.trim().is_empty() {
            return Err(ContextError::EmptyUserQuestion);
        }
        Ok(())
    }

    /// Serializes the payload into compact single-line JSON for CLI argument templates.
    pub fn to_compact_json(&self) -> Result<String, ContextError> {
        serde_json::to_string(self).map_err(|e| ContextError::Serialization(e.to_string()))
    }

    /// Serializes the payload into pretty-printed JSON for prompt context embedding.
    pub fn to_pretty_json(&self) -> Result<String, ContextError> {
        serde_json::to_string_pretty(self).map_err(|e| ContextError::Serialization(e.to_string()))
    }

    /// Formats a deterministic human-readable summary of all context fields.
    pub fn format_human_summary(&self) -> String {
        let oldest = self.oldest_modified_utc.as_deref().unwrap_or("N/A");
        let newest = self.newest_modified_utc.as_deref().unwrap_or("N/A");
        let signature = self
            .publisher_signature
            .as_deref()
            .unwrap_or("Unsigned / Unknown");

        let extensions = if self.top_extensions.is_empty() {
            "None recorded".to_string()
        } else {
            self.top_extensions
                .iter()
                .map(|ext| {
                    format!(
                        "{} ({} files, {})",
                        ext.extension,
                        ext.file_count,
                        format_bytes(ext.total_bytes)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        format!(
            "- Path: {path}\n\
             - Logical Size: {logical_hr} ({logical_bytes} bytes)\n\
             - Physical Size on Disk: {physical_hr} ({physical_bytes} bytes)\n\
             - File Count: {file_count}\n\
             - Oldest Modified (UTC): {oldest}\n\
             - Newest Modified (UTC): {newest}\n\
             - Publisher / Digital Signature: {signature}\n\
             - Top Extensions: {extensions}\n\
             - Locked Handle Status: {lock_status}\n\
             - User Question: {question}",
            path = self.path,
            logical_hr = format_bytes(self.logical_size_bytes),
            logical_bytes = self.logical_size_bytes,
            physical_hr = format_bytes(self.physical_size_bytes),
            physical_bytes = self.physical_size_bytes,
            file_count = self.file_count,
            lock_status = self.locked_handle_status.describe(),
            question = self.user_question,
        )
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    format!("{value:.2} {}", UNITS[unit_idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> StorageContextPayload {
        StorageContextPayload {
            path: r"C:\Users\dev\.cargo\registry\cache".to_string(),
            logical_size_bytes: 3_221_225_472,
            physical_size_bytes: 3_250_000_000,
            file_count: 4210,
            oldest_modified_utc: Some("2024-03-10T11:20:00Z".to_string()),
            newest_modified_utc: Some("2026-11-10T09:15:00Z".to_string()),
            publisher_signature: None,
            top_extensions: vec![
                ExtensionStat::new(".crate", 4100, 3_190_000_000),
                ExtensionStat::new(".index", 110, 31_225_472),
            ],
            locked_handle_status: LockedHandleStatus::Unlocked,
            user_question: "Why is .cargo\\registry\\cache taking 3 GB? Should I delete it?"
                .to_string(),
        }
    }

    #[test]
    fn validates_non_empty_path_and_question() {
        let mut p = sample_payload();
        assert!(p.validate().is_ok());

        p.path = "   ".to_string();
        assert_eq!(p.validate(), Err(ContextError::EmptyPath));

        p = sample_payload();
        p.user_question = "".to_string();
        assert_eq!(p.validate(), Err(ContextError::EmptyUserQuestion));
    }

    #[test]
    fn serializes_round_trip_json() {
        let p = sample_payload();
        let compact = p.to_compact_json().expect("compact json");
        let parsed: StorageContextPayload = serde_json::from_str(&compact).expect("round-trip");
        assert_eq!(parsed, p);
        assert!(!compact.contains('\n'));
    }

    #[test]
    fn formats_human_summary_with_locked_processes() {
        let mut p = sample_payload();
        p.locked_handle_status = LockedHandleStatus::Locked {
            locking_processes: vec!["cargo.exe".to_string(), "rust-analyzer.exe".to_string()],
        };
        let summary = p.format_human_summary();
        assert!(summary.contains(r"C:\Users\dev\.cargo\registry\cache"));
        assert!(summary.contains(".crate"));
        assert!(summary.contains("cargo.exe"));
        assert!(summary.contains("rust-analyzer.exe"));
    }
}

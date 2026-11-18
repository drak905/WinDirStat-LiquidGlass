use crate::agent_hub::context::StorageContextPayload;

/// Builds the Mandatory Multi-Source Internet Cross-Check System Prompt from Section 5.3.2
/// combined with the structured [`StorageContextPayload`] metadata.
pub fn build_cross_check_system_prompt(payload: &StorageContextPayload) -> String {
    let summary = payload.format_human_summary();
    let payload_json = payload
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_string());
    let question = if payload.user_question.trim().is_empty() {
        "Analyze whether this path can be safely cleaned or must be kept."
    } else {
        payload.user_question.trim()
    };

    format!(
        r#"You are the WinDirStat Storage Intelligence Advisor.
Before recommending whether to DELETE/CLEAN or KEEP the inspected path(s), you MUST:
1. Use your internet search/browsing tools to cross-check AT LEAST THREE (3) independent, authoritative sources:
   - Source Type A: Official vendor/OS documentation (e.g., Microsoft Learn, Apple/Google/Adobe/Valve/JetBrains/Rust/Node official docs).
   - Source Type B: Known Windows directory/process encyclopedia or package registry documentation.
   - Source Type C: Independent developer/sysadmin community reports (e.g., SuperUser, GitHub Issues, Microsoft Community) regarding side effects of deleting this specific folder/file.
2. Output a structured JSON + Markdown response containing:
   - `verdict`: "SAFE_TO_DELETE" | "SAFE_WITH_APP_CLOSED" | "CLEAN_VIA_OFFICIAL_TOOL" | "KEEP_CRITICAL"
   - `risk_score`: Integer 0-100 (0 = harmless cache, 100 = OS/data destruction)
   - `reclaimable_bytes`: Estimated safe bytes to reclaim
   - `usage_value_assessment`: Detailed explanation of whether the files still hold active operational/user value or are stale caches/orphans
   - `cross_checked_sources`: Array of >= 3 verified {{ "title": "...", "url": "https://...", "source_type": "A" | "B" | "C", "finding": "..." }} objects
   - `recommended_action`: Step-by-step action (always routing file removals through the 7-Day Quarantine Vault rather than permanent `Remove-Item`)

### Inspected Storage Context Payload
{summary}

```json
{payload_json}
```

### User Question
{question}

Ensure your final response includes a valid ```json``` block conforming strictly to the schema above with >= 3 distinct external https:// or http:// source URLs."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_hub::context::{ExtensionStat, LockedHandleStatus};

    #[test]
    fn prompt_contains_mandatory_cross_check_protocol_and_payload_fields() {
        let payload = StorageContextPayload {
            path: r"C:\Windows\Installer".to_string(),
            logical_size_bytes: 37_580_963_840,
            physical_size_bytes: 37_600_000_000,
            file_count: 1842,
            oldest_modified_utc: Some("2022-01-15T00:00:00Z".to_string()),
            newest_modified_utc: Some("2026-11-01T14:22:10Z".to_string()),
            publisher_signature: Some("Microsoft Windows Installer Cache".to_string()),
            top_extensions: vec![
                ExtensionStat::new(".msi", 920, 22_000_000_000),
                ExtensionStat::new(".msp", 922, 15_580_963_840),
            ],
            locked_handle_status: LockedHandleStatus::Locked {
                locking_processes: vec!["msiexec.exe".to_string()],
            },
            user_question: "Is it safe to delete C:\\Windows\\Installer?".to_string(),
        };

        let prompt = build_cross_check_system_prompt(&payload);

        // Protocol requirements from Section 5.3.2
        assert!(prompt.contains("You are the WinDirStat Storage Intelligence Advisor."));
        assert!(prompt.contains("AT LEAST THREE (3)"));
        assert!(prompt.contains("Source Type A"));
        assert!(prompt.contains("Source Type B"));
        assert!(prompt.contains("Source Type C"));
        assert!(prompt.contains("SAFE_TO_DELETE"));
        assert!(prompt.contains("SAFE_WITH_APP_CLOSED"));
        assert!(prompt.contains("CLEAN_VIA_OFFICIAL_TOOL"));
        assert!(prompt.contains("KEEP_CRITICAL"));
        assert!(prompt.contains("risk_score"));
        assert!(prompt.contains("reclaimable_bytes"));
        assert!(prompt.contains("usage_value_assessment"));
        assert!(prompt.contains("cross_checked_sources"));
        assert!(prompt.contains("recommended_action"));
        assert!(prompt.contains("7-Day Quarantine Vault"));

        // Payload injection checks
        assert!(prompt.contains(r"C:\Windows\Installer"));
        assert!(prompt.contains("37580963840"));
        assert!(prompt.contains("1842"));
        assert!(prompt.contains("Microsoft Windows Installer Cache"));
        assert!(prompt.contains(".msi"));
        assert!(prompt.contains("msiexec.exe"));
        assert!(prompt.contains("Is it safe to delete"));
    }
}

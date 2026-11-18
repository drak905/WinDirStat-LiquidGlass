use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::net::IpAddr;
use thiserror::Error;
use url::{Host, Url};

/// Strict recommendation verdicts allowed by the WinDirStat Storage Intelligence Advisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdvisorVerdict {
    SafeToDelete,
    SafeWithAppClosed,
    CleanViaOfficialTool,
    KeepCritical,
}

impl AdvisorVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdvisorVerdict::SafeToDelete => "SAFE_TO_DELETE",
            AdvisorVerdict::SafeWithAppClosed => "SAFE_WITH_APP_CLOSED",
            AdvisorVerdict::CleanViaOfficialTool => "CLEAN_VIA_OFFICIAL_TOOL",
            AdvisorVerdict::KeepCritical => "KEEP_CRITICAL",
        }
    }

    pub fn allows_quarantine_removal(&self) -> bool {
        !matches!(self, AdvisorVerdict::KeepCritical)
    }

    fn parse_str(value: &str) -> Result<Self, ValidationError> {
        match value.trim() {
            "SAFE_TO_DELETE" => Ok(AdvisorVerdict::SafeToDelete),
            "SAFE_WITH_APP_CLOSED" => Ok(AdvisorVerdict::SafeWithAppClosed),
            "CLEAN_VIA_OFFICIAL_TOOL" => Ok(AdvisorVerdict::CleanViaOfficialTool),
            "KEEP_CRITICAL" => Ok(AdvisorVerdict::KeepCritical),
            other => Err(ValidationError::InvalidVerdict(other.to_string())),
        }
    }
}

/// An independent internet source verified by the Agent CLI during cross-checking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossCheckedSource {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub source_type: String,
    pub finding: String,
}

/// Validated response payload returned by an Agent CLI advisor session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorResponse {
    pub verdict: AdvisorVerdict,
    pub risk_score: u8,
    pub reclaimable_bytes: u64,
    pub usage_value_assessment: String,
    pub cross_checked_sources: Vec<CrossCheckedSource>,
    pub recommended_action: String,
}

/// Errors produced when parsing or strictly validating an Agent CLI response.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("agent output was empty")]
    EmptyOutput,
    #[error("no valid advisor JSON object found in agent output")]
    NoValidJsonFound,
    #[error("invalid advisor JSON structure: {0}")]
    InvalidSchema(String),
    #[error("missing required field `verdict`")]
    MissingVerdict,
    #[error("invalid `verdict` value: {0}")]
    InvalidVerdict(String),
    #[error("missing required field `risk_score`")]
    MissingRiskScore,
    #[error("`risk_score` value {0} is out of valid range 0..=100")]
    RiskScoreOutOfRange(i64),
    #[error("missing required field `reclaimable_bytes`")]
    MissingReclaimableBytes,
    #[error("invalid `reclaimable_bytes` value: {0}")]
    InvalidReclaimableBytes(String),
    #[error("`usage_value_assessment` must not be empty")]
    EmptyUsageValueAssessment,
    #[error("`recommended_action` must not be empty")]
    EmptyRecommendedAction,
    #[error("insufficient cross-checked sources: found {found}, minimum required is {required}")]
    InsufficientSources { found: usize, required: usize },
    #[error("cross-checked source at index {index} is invalid: {reason}")]
    InvalidSourceEntry { index: usize, reason: String },
    #[error("cross-checked source URL at index {index} (`{url}`) is invalid: {reason}")]
    InvalidSourceUrl {
        index: usize,
        url: String,
        reason: String,
    },
    #[error("duplicate cross-checked source URL at index {index}: {normalized_url}")]
    DuplicateSourceUrl {
        index: usize,
        normalized_url: String,
    },
    #[error("recommended_action advocates bypassing the 7-Day Quarantine Vault: {0}")]
    QuarantineBypassAdvocated(String),
}

/// Minimum number of independent external internet sources required by Section 5.3.2.
pub const MIN_CROSS_CHECKED_SOURCES: usize = 3;

const QUARANTINE_ANNOTATION: &str =
    "[7-Day Quarantine Vault Policy: All file removals must be routed through the 7-Day Quarantine Vault for safe rollback.]";

/// Parses an Agent CLI output (raw JSON, fenced ```json``` Markdown, or stream-json NDJSON)
/// and enforces all Mandatory Multi-Source Internet Cross-Check invariants.
pub fn parse_and_validate_advisor_response(
    raw_output: &str,
) -> Result<AdvisorResponse, ValidationError> {
    let trimmed = raw_output.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::EmptyOutput);
    }

    let candidates = collect_advisor_json_candidates(trimmed);
    let best = select_best_candidate(&candidates).ok_or(ValidationError::NoValidJsonFound)?;
    validate_advisor_object(best)
}

fn collect_advisor_json_candidates(raw: &str) -> Vec<Map<String, Value>> {
    let mut candidates = Vec::new();
    let mut text_fragments = Vec::new();
    let mut delta_stream = String::new();

    // 1. If the entire output is a single JSON object, inspect it directly.
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) {
        if has_any_advisor_key(&obj) || !is_stream_envelope(&obj) {
            candidates.push(obj.clone());
        }
        extract_text_from_event_value(&Value::Object(obj), &mut text_fragments, &mut delta_stream);
    }

    // 2. Scan NDJSON / stream-json lines (Claude, Codex, OpenCode, Antigravity).
    for line in raw.lines() {
        let line_trimmed = line.trim();
        if !line_trimmed.starts_with('{') {
            continue;
        }
        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line_trimmed) {
            if has_any_advisor_key(&obj) || !is_stream_envelope(&obj) {
                candidates.push(obj.clone());
            }
            extract_text_from_event_value(
                &Value::Object(obj),
                &mut text_fragments,
                &mut delta_stream,
            );
        }
    }

    if !delta_stream.trim().is_empty() {
        text_fragments.push(delta_stream);
    }
    text_fragments.push(raw.to_string());

    // 3. Extract JSON from fenced code blocks and balanced `{ ... }` substrings.
    for text in text_fragments {
        for block in extract_fenced_blocks(&text) {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(block.trim()) {
                if has_any_advisor_key(&obj) || !is_stream_envelope(&obj) {
                    candidates.push(obj);
                }
            }
        }
        for slice in extract_balanced_json_objects(&text) {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(slice) {
                if has_any_advisor_key(&obj) || !is_stream_envelope(&obj) {
                    candidates.push(obj);
                }
            }
        }
    }

    candidates
}

fn has_any_advisor_key(obj: &Map<String, Value>) -> bool {
    const KEYS: [&str; 6] = [
        "verdict",
        "risk_score",
        "reclaimable_bytes",
        "usage_value_assessment",
        "cross_checked_sources",
        "recommended_action",
    ];
    KEYS.iter().any(|k| obj.contains_key(*k))
}

fn advisor_key_score(obj: &Map<String, Value>) -> usize {
    const KEYS: [&str; 6] = [
        "verdict",
        "risk_score",
        "reclaimable_bytes",
        "usage_value_assessment",
        "cross_checked_sources",
        "recommended_action",
    ];
    let count = KEYS.iter().filter(|k| obj.contains_key(**k)).count();
    let verdict_bonus = if obj.contains_key("verdict") { 10 } else { 0 };
    count + verdict_bonus
}

fn is_stream_envelope(obj: &Map<String, Value>) -> bool {
    obj.contains_key("type")
        || obj.contains_key("subtype")
        || obj.contains_key("thread_id")
        || obj.contains_key("delta")
        || obj.contains_key("part")
        || obj.contains_key("item")
        || obj.contains_key("message")
}

fn select_best_candidate(candidates: &[Map<String, Value>]) -> Option<&Map<String, Value>> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, obj)| has_any_advisor_key(obj) || !is_stream_envelope(obj))
        .max_by_key(|(idx, obj)| (advisor_key_score(obj), *idx))
        .map(|(_, obj)| obj)
}

fn extract_text_from_event_value(
    value: &Value,
    messages: &mut Vec<String>,
    delta_stream: &mut String,
) {
    let Some(obj) = value.as_object() else {
        return;
    };

    // Streaming delta chunks (e.g. Claude `content_block_delta`)
    if let Some(delta_text) = obj
        .get("delta")
        .and_then(|d| d.get("text").or_else(|| d.get("delta")))
        .and_then(|v| v.as_str())
    {
        delta_stream.push_str(delta_text);
    }

    // Claude `assistant` -> `message.content[*].text`
    if let Some(msg) = obj.get("message") {
        collect_message_content(msg, messages);
    }

    // Codex `item.completed` -> `item.text` / `item.content`
    if let Some(item) = obj.get("item") {
        collect_message_content(item, messages);
    }

    // OpenCode `part.text`
    if let Some(part) = obj.get("part") {
        collect_message_content(part, messages);
    }

    // Generic nested `msg` wrapper
    if let Some(msg) = obj.get("msg") {
        collect_message_content(msg, messages);
    }

    // Direct text fields on event objects
    for key in ["result", "content", "text", "response", "output"] {
        if let Some(field) = obj.get(key) {
            collect_message_content(field, messages);
        }
    }
}

fn collect_message_content(val: &Value, messages: &mut Vec<String>) {
    match val {
        Value::String(s) if !s.trim().is_empty() => messages.push(s.clone()),
        Value::Array(items) => {
            let mut combined = String::new();
            for item in items {
                if let Some(s) = item.as_str() {
                    combined.push_str(s);
                } else if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    combined.push_str(text);
                }
            }
            if !combined.trim().is_empty() {
                messages.push(combined);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                if !text.trim().is_empty() {
                    messages.push(text.to_string());
                }
            }
            if let Some(content) = map.get("content") {
                collect_message_content(content, messages);
            }
            if let Some(message) = map.get("message").and_then(|v| v.as_str()) {
                if !message.trim().is_empty() {
                    messages.push(message.to_string());
                }
            }
        }
        _ => {}
    }
}

fn extract_fenced_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut search_from = 0;
    while let Some(open_rel) = text[search_from..].find("```") {
        let open_idx = search_from + open_rel;
        let after_ticks = open_idx + 3;
        let Some(newline_rel) = text[after_ticks..].find('\n') else {
            break;
        };
        let lang = text[after_ticks..after_ticks + newline_rel].trim();
        let body_start = after_ticks + newline_rel + 1;
        let Some(close_rel) = text[body_start..].find("```") else {
            break;
        };
        let body_end = body_start + close_rel;
        if lang.is_empty() || lang.eq_ignore_ascii_case("json") {
            blocks.push(&text[body_start..body_end]);
        }
        search_from = body_end + 3;
    }
    blocks
}

fn extract_balanced_json_objects(text: &str) -> Vec<&str> {
    let mut slices = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' && looks_like_json_object_start(&text[i + 1..]) {
            if let Some(end_idx) = find_matching_brace_end(text, i) {
                slices.push(&text[i..=end_idx]);
                i = end_idx + 1;
                continue;
            }
        }
        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
    }

    slices
}

fn looks_like_json_object_start(rest: &str) -> bool {
    matches!(rest.trim_start().as_bytes().first(), Some(b'"' | b'}'))
}

fn find_matching_brace_end(text: &str, start_idx: usize) -> Option<usize> {
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (rel_idx, ch) in text[start_idx..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start_idx + rel_idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn validate_advisor_object(obj: &Map<String, Value>) -> Result<AdvisorResponse, ValidationError> {
    let verdict_raw = obj
        .get("verdict")
        .filter(|v| !v.is_null())
        .ok_or(ValidationError::MissingVerdict)?;
    let verdict_str = verdict_raw
        .as_str()
        .ok_or_else(|| ValidationError::InvalidVerdict(verdict_raw.to_string()))?;
    let verdict = AdvisorVerdict::parse_str(verdict_str)?;

    let risk_raw = obj
        .get("risk_score")
        .filter(|v| !v.is_null())
        .ok_or(ValidationError::MissingRiskScore)?;
    let risk_i64 = risk_raw.as_i64().ok_or_else(|| {
        ValidationError::InvalidSchema(format!("`risk_score` must be an integer, got {risk_raw}"))
    })?;
    if !(0..=100).contains(&risk_i64) {
        return Err(ValidationError::RiskScoreOutOfRange(risk_i64));
    }
    let risk_score = risk_i64 as u8;

    let reclaimable_raw = obj
        .get("reclaimable_bytes")
        .filter(|v| !v.is_null())
        .ok_or(ValidationError::MissingReclaimableBytes)?;
    let reclaimable_bytes = reclaimable_raw.as_u64().ok_or_else(|| {
        ValidationError::InvalidReclaimableBytes(reclaimable_raw.to_string())
    })?;

    let usage_value_assessment = obj
        .get("usage_value_assessment")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(ValidationError::EmptyUsageValueAssessment)?
        .to_string();

    let recommended_action_raw = obj
        .get("recommended_action")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(ValidationError::EmptyRecommendedAction)?;

    let recommended_action =
        validate_and_enforce_quarantine(verdict, recommended_action_raw)?;

    let sources_arr = obj
        .get("cross_checked_sources")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ValidationError::InvalidSchema(
                "`cross_checked_sources` must be an array of source objects".to_string(),
            )
        })?;

    if sources_arr.len() < MIN_CROSS_CHECKED_SOURCES {
        return Err(ValidationError::InsufficientSources {
            found: sources_arr.len(),
            required: MIN_CROSS_CHECKED_SOURCES,
        });
    }

    let mut cross_checked_sources = Vec::with_capacity(sources_arr.len());
    let mut seen_urls = HashSet::new();

    for (index, item) in sources_arr.iter().enumerate() {
        let map = item.as_object().ok_or_else(|| ValidationError::InvalidSourceEntry {
            index,
            reason: "source entry must be a JSON object".to_string(),
        })?;

        let title = map
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ValidationError::InvalidSourceEntry {
                index,
                reason: "missing or empty `title`".to_string(),
            })?
            .to_string();

        let finding = map
            .get("finding")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ValidationError::InvalidSourceEntry {
                index,
                reason: "missing or empty `finding`".to_string(),
            })?
            .to_string();

        let url_raw = map
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ValidationError::InvalidSourceUrl {
                index,
                url: String::new(),
                reason: "missing or empty `url`".to_string(),
            })?;

        let normalized = validate_and_normalize_external_url(index, url_raw)?;
        if !seen_urls.insert(normalized.clone()) {
            return Err(ValidationError::DuplicateSourceUrl {
                index,
                normalized_url: normalized,
            });
        }

        let source_type = map
            .get("source_type")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();

        cross_checked_sources.push(CrossCheckedSource {
            title,
            url: url_raw.to_string(),
            source_type,
            finding,
        });
    }

    Ok(AdvisorResponse {
        verdict,
        risk_score,
        reclaimable_bytes,
        usage_value_assessment,
        cross_checked_sources,
        recommended_action,
    })
}

fn validate_and_normalize_external_url(
    index: usize,
    raw_url: &str,
) -> Result<String, ValidationError> {
    let parsed = Url::parse(raw_url).map_err(|e| ValidationError::InvalidSourceUrl {
        index,
        url: raw_url.to_string(),
        reason: e.to_string(),
    })?;

    let scheme = parsed.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(ValidationError::InvalidSourceUrl {
            index,
            url: raw_url.to_string(),
            reason: format!("scheme `{scheme}` is not http or https"),
        });
    }

    if parsed.cannot_be_a_base() {
        return Err(ValidationError::InvalidSourceUrl {
            index,
            url: raw_url.to_string(),
            reason: "URL cannot be a base".to_string(),
        });
    }

    let host = parsed.host().ok_or_else(|| ValidationError::InvalidSourceUrl {
        index,
        url: raw_url.to_string(),
        reason: "URL is missing host".to_string(),
    })?;

    let host_canonical = match host {
        Host::Domain(domain) => {
            let lower = domain.trim_end_matches('.').to_ascii_lowercase();
            if lower.is_empty()
                || !lower.contains('.')
                || lower.starts_with('.')
                || lower == "localhost"
                || lower.ends_with(".localhost")
                || lower.ends_with(".local")
                || lower.ends_with(".internal")
                || lower.ends_with(".arpa")
            {
                return Err(ValidationError::InvalidSourceUrl {
                    index,
                    url: raw_url.to_string(),
                    reason: format!("host `{domain}` is not a public external internet domain"),
                });
            }
            lower
        }
        Host::Ipv4(ip) => {
            if is_non_public_ip(IpAddr::V4(ip)) {
                return Err(ValidationError::InvalidSourceUrl {
                    index,
                    url: raw_url.to_string(),
                    reason: format!("IPv4 address `{ip}` is non-public/local"),
                });
            }
            ip.to_string()
        }
        Host::Ipv6(ip) => {
            if is_non_public_ip(IpAddr::V6(ip)) {
                return Err(ValidationError::InvalidSourceUrl {
                    index,
                    url: raw_url.to_string(),
                    reason: format!("IPv6 address `{ip}` is non-public/local"),
                });
            }
            format!("[{ip}]")
        }
    };

    let port_part = parsed
        .port()
        .map(|p| format!(":{p}"))
        .unwrap_or_default();

    let path = parsed.path();
    let normalized_path = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };

    let query_part = parsed
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    Ok(format!(
        "{scheme}://{host_canonical}{port_part}{normalized_path}{query_part}"
    ))
}

fn is_non_public_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            let is_cgnat = octets[0] == 100 && (64..=127).contains(&octets[1]);
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || is_cgnat
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let is_unique_local = (segments[0] & 0xfe00) == 0xfc00;
            let is_link_local = (segments[0] & 0xffc0) == 0xfe80;
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || is_unique_local
                || is_link_local
        }
    }
}

fn validate_and_enforce_quarantine(
    verdict: AdvisorVerdict,
    recommended_action: &str,
) -> Result<String, ValidationError> {
    let lower = recommended_action.to_ascii_lowercase();

    for phrase in [
        "bypass quarantine",
        "skip quarantine",
        "without quarantine",
        "no quarantine",
    ] {
        if lower.contains(phrase) {
            return Err(ValidationError::QuarantineBypassAdvocated(format!(
                "contains forbidden bypass directive `{phrase}`"
            )));
        }
    }

    const DESTRUCTIVE_TERMS: [&str; 13] = [
        "remove-item",
        "rm -rf",
        "rm -fr",
        "rm -r",
        "rd /s",
        "rd /q",
        "rmdir /s",
        "rmdir /q",
        "del /f",
        "del /s",
        "del /q",
        "shift+delete",
        "permanently delete",
    ];

    for term in DESTRUCTIVE_TERMS {
        if let Some(pos) = lower.find(term) {
            let prefix_start = pos.saturating_sub(48);
            let context_before = &lower[prefix_start..pos];
            let negated = [
                "rather than",
                "instead of",
                "never ",
                "do not ",
                "don't ",
                "avoid ",
                "not ",
            ]
            .iter()
            .any(|neg| context_before.contains(neg));
            if !negated {
                return Err(ValidationError::QuarantineBypassAdvocated(format!(
                    "recommends un-quarantined permanent deletion (`{term}`)"
                )));
            }
        }
    }

    if verdict.allows_quarantine_removal() && !lower.contains("quarantine") {
        Ok(format!("{recommended_action} {QUARANTINE_ANNOTATION}"))
    } else {
        Ok(recommended_action.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json_payload() -> String {
        serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 12,
            "reclaimable_bytes": 3_221_225_472u64,
            "usage_value_assessment": "Stale Cargo compressed crate tarballs; Cargo re-downloads them on demand from crates.io.",
            "cross_checked_sources": [
                {
                    "title": "Cargo Book - Cargo Home & Cache",
                    "url": "https://doc.rust-lang.org/cargo/guide/cargo-home.html",
                    "source_type": "A",
                    "finding": "cache/*.crate files are safe to purge when offline rebuilds are not needed."
                },
                {
                    "title": "crates.io Cargo Registry Documentation",
                    "url": "https://crates.io/policies",
                    "source_type": "B",
                    "finding": "Published crate tarballs are immutable and can always be re-fetched."
                },
                {
                    "title": "Rust Users Forum - Safely cleaning .cargo/registry/cache",
                    "url": "https://users.rust-lang.org/t/safe-to-clean-cargo-registry-cache/51234",
                    "source_type": "C",
                    "finding": "Community confirms deleting cache tarballs leaves src/ and builds intact or regenerable."
                }
            ],
            "recommended_action": "Move .cargo\\registry\\cache to the 7-Day Quarantine Vault using WinDirStat."
        })
        .to_string()
    }

    #[test]
    fn accepts_valid_raw_json_with_three_distinct_sources() {
        let parsed = parse_and_validate_advisor_response(&valid_json_payload()).expect("valid");
        assert_eq!(parsed.verdict, AdvisorVerdict::SafeToDelete);
        assert_eq!(parsed.risk_score, 12);
        assert_eq!(parsed.cross_checked_sources.len(), 3);
    }

    #[test]
    fn accepts_fenced_json_inside_markdown() {
        let markdown = format!(
            "### Storage Analysis\n\nI checked 3 external sources.\n\n```json\n{}\n```\n\nLet me know if you want to quarantine.",
            valid_json_payload()
        );
        let parsed = parse_and_validate_advisor_response(&markdown).expect("fenced json");
        assert_eq!(parsed.verdict, AdvisorVerdict::SafeToDelete);
    }

    #[test]
    fn accepts_claude_stream_json_events() {
        let payload = valid_json_payload();
        let line1 = serde_json::json!({"type": "system", "subtype": "init"}).to_string();
        let line2 = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": format!("```json\n{payload}\n```") }]
            }
        })
        .to_string();
        let line3 =
            serde_json::json!({"type": "result", "subtype": "success", "result": "done"})
                .to_string();
        let stream = format!("{line1}\n{line2}\n{line3}");
        let parsed = parse_and_validate_advisor_response(&stream).expect("claude stream-json");
        assert_eq!(parsed.cross_checked_sources.len(), 3);
    }

    #[test]
    fn accepts_claude_content_block_deltas_split_across_lines() {
        let payload = format!("```json\n{}\n```", valid_json_payload());
        let mid = payload.len() / 2;
        let (part1, part2) = payload.split_at(mid);
        let stream = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"content_block_delta","delta":{"type":"text_delta","text":part1}}),
            serde_json::json!({"type":"content_block_delta","delta":{"type":"text_delta","text":part2}})
        );
        let parsed = parse_and_validate_advisor_response(&stream).expect("stream deltas");
        assert_eq!(parsed.verdict, AdvisorVerdict::SafeToDelete);
    }

    #[test]
    fn accepts_codex_and_opencode_json_events() {
        let payload = valid_json_payload();
        let codex_stream = format!(
            "{}\n{}\n",
            serde_json::json!({"type": "thread.started", "thread_id": "t_1"}),
            serde_json::json!({
                "type": "item.completed",
                "item": { "id": "i_1", "type": "agent_message", "text": payload }
            })
        );
        assert!(parse_and_validate_advisor_response(&codex_stream).is_ok());

        let opencode_stream = format!(
            "{}\n",
            serde_json::json!({
                "type": "text",
                "part": { "text": format!("```json\n{}\n```", valid_json_payload()) }
            })
        );
        assert!(parse_and_validate_advisor_response(&opencode_stream).is_ok());
    }

    #[test]
    fn rejects_fewer_than_three_sources() {
        let two_sources = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 10,
            "reclaimable_bytes": 1024,
            "usage_value_assessment": "Temp cache.",
            "cross_checked_sources": [
                { "title": "Doc A", "url": "https://learn.microsoft.com/en-us/windows", "source_type": "A", "finding": "Safe." },
                { "title": "Doc B", "url": "https://superuser.com/questions/1", "source_type": "C", "finding": "Safe." }
            ],
            "recommended_action": "Move to 7-Day Quarantine Vault."
        })
        .to_string();

        assert_eq!(
            parse_and_validate_advisor_response(&two_sources),
            Err(ValidationError::InsufficientSources {
                found: 2,
                required: 3
            })
        );
    }

    #[test]
    fn rejects_duplicate_urls_even_with_fragment_or_case_differences() {
        let dup = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 15,
            "reclaimable_bytes": 2048,
            "usage_value_assessment": "Temp cache.",
            "cross_checked_sources": [
                { "title": "Doc A", "url": "https://learn.microsoft.com/en-us/windows/storage", "source_type": "A", "finding": "Safe." },
                { "title": "Doc A Dup", "url": "HTTPS://LEARN.MICROSOFT.COM/en-us/windows/storage/#section", "source_type": "B", "finding": "Safe." },
                { "title": "Doc C", "url": "https://superuser.com/questions/42", "source_type": "C", "finding": "Safe." }
            ],
            "recommended_action": "Move to 7-Day Quarantine Vault."
        })
        .to_string();

        assert!(matches!(
            parse_and_validate_advisor_response(&dup),
            Err(ValidationError::DuplicateSourceUrl { index: 1, .. })
        ));
    }

    #[test]
    fn rejects_non_http_and_local_urls() {
        for bad_url in [
            "file:///C:/Windows/win.ini",
            "ftp://example.com/file",
            "not-a-valid-url",
            "https://localhost/docs",
            "http://127.0.0.1/internal",
            "https://192.168.1.50/admin",
        ] {
            let json = serde_json::json!({
                "verdict": "SAFE_TO_DELETE",
                "risk_score": 10,
                "reclaimable_bytes": 100,
                "usage_value_assessment": "Cache.",
                "cross_checked_sources": [
                    { "title": "S1", "url": bad_url, "source_type": "A", "finding": "Ok." },
                    { "title": "S2", "url": "https://learn.microsoft.com/en-us/a", "source_type": "B", "finding": "Ok." },
                    { "title": "S3", "url": "https://superuser.com/questions/1", "source_type": "C", "finding": "Ok." }
                ],
                "recommended_action": "Move to 7-Day Quarantine Vault."
            })
            .to_string();

            assert!(
                matches!(
                    parse_and_validate_advisor_response(&json),
                    Err(ValidationError::InvalidSourceUrl { index: 0, .. })
                ),
                "expected InvalidSourceUrl for {bad_url}"
            );
        }
    }

    #[test]
    fn rejects_missing_verdict_and_invalid_risk_score() {
        let missing_verdict = serde_json::json!({
            "risk_score": 10,
            "reclaimable_bytes": 100,
            "usage_value_assessment": "Cache.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Move to 7-Day Quarantine Vault."
        })
        .to_string();
        assert_eq!(
            parse_and_validate_advisor_response(&missing_verdict),
            Err(ValidationError::MissingVerdict)
        );

        let high_risk = serde_json::json!({
            "verdict": "KEEP_CRITICAL",
            "risk_score": 105,
            "reclaimable_bytes": 0,
            "usage_value_assessment": "Critical.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Keep intact."
        })
        .to_string();
        assert_eq!(
            parse_and_validate_advisor_response(&high_risk),
            Err(ValidationError::RiskScoreOutOfRange(105))
        );
    }

    #[test]
    fn enforces_quarantine_vault_and_rejects_bypass_commands() {
        let bypass = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 5,
            "reclaimable_bytes": 1000,
            "usage_value_assessment": "Cache.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Run Remove-Item -Recurse -Force C:\\Temp to permanently delete it."
        })
        .to_string();
        assert!(matches!(
            parse_and_validate_advisor_response(&bypass),
            Err(ValidationError::QuarantineBypassAdvocated(_))
        ));

        // Negated Remove-Item when recommending Quarantine Vault is permitted.
        let safe_negated = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 5,
            "reclaimable_bytes": 1000,
            "usage_value_assessment": "Cache.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Route files through the 7-Day Quarantine Vault rather than permanent Remove-Item."
        })
        .to_string();
        assert!(parse_and_validate_advisor_response(&safe_negated).is_ok());

        // Deletion verdict without explicit Quarantine Vault mention gets annotated.
        let unannotated = serde_json::json!({
            "verdict": "SAFE_WITH_APP_CLOSED",
            "risk_score": 20,
            "reclaimable_bytes": 1000,
            "usage_value_assessment": "Browser shader cache.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Close Chrome before cleaning the shader cache directory."
        })
        .to_string();
        let parsed = parse_and_validate_advisor_response(&unannotated).expect("annotated");
        assert!(parsed.recommended_action.contains("7-Day Quarantine Vault"));
    }

    #[test]
    fn extracts_json_even_after_unmatched_brace_in_prose_and_crlf() {
        let noisy = format!(
            "Note: syntax like {{ broken won't break parsing.\r\n{}\r\nThanks!",
            valid_json_payload()
        );
        let parsed = parse_and_validate_advisor_response(&noisy).expect("recover after prose brace");
        assert_eq!(parsed.verdict, AdvisorVerdict::SafeToDelete);
    }

    #[test]
    fn rejects_empty_fields_negative_risk_and_private_ips() {
        let neg_risk = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": -1,
            "reclaimable_bytes": 100,
            "usage_value_assessment": "Cache.",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Move to 7-Day Quarantine Vault."
        })
        .to_string();
        assert_eq!(
            parse_and_validate_advisor_response(&neg_risk),
            Err(ValidationError::RiskScoreOutOfRange(-1))
        );

        let empty_usage = serde_json::json!({
            "verdict": "SAFE_TO_DELETE",
            "risk_score": 10,
            "reclaimable_bytes": 100,
            "usage_value_assessment": "   ",
            "cross_checked_sources": [
                { "title": "S1", "url": "https://learn.microsoft.com/a", "source_type": "A", "finding": "Ok." },
                { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
            ],
            "recommended_action": "Move to 7-Day Quarantine Vault."
        })
        .to_string();
        assert_eq!(
            parse_and_validate_advisor_response(&empty_usage),
            Err(ValidationError::EmptyUsageValueAssessment)
        );

        for private_url in [
            "http://10.0.0.5/doc",
            "https://172.16.4.2/doc",
            "https://[::1]/doc",
            "https://intranet/doc",
        ] {
            let bad_ip = serde_json::json!({
                "verdict": "SAFE_TO_DELETE",
                "risk_score": 10,
                "reclaimable_bytes": 100,
                "usage_value_assessment": "Cache.",
                "cross_checked_sources": [
                    { "title": "S1", "url": private_url, "source_type": "A", "finding": "Ok." },
                    { "title": "S2", "url": "https://doc.rust-lang.org/b", "source_type": "B", "finding": "Ok." },
                    { "title": "S3", "url": "https://superuser.com/c", "source_type": "C", "finding": "Ok." }
                ],
                "recommended_action": "Move to 7-Day Quarantine Vault."
            })
            .to_string();
            assert!(matches!(
                parse_and_validate_advisor_response(&bad_ip),
                Err(ValidationError::InvalidSourceUrl { index: 0, .. })
            ));
        }
    }
}

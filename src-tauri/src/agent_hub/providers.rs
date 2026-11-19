use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors related to Agent CLI provider configuration or binary discovery.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("custom CLI executable path must not be empty")]
    EmptyExecutable,
    #[error("custom CLI args_template must not be empty")]
    EmptyArgsTemplate,
    #[error("custom CLI args_template must reference {{PROMPT}} or {{CONTEXT_JSON}}")]
    MissingTemplatePlaceholder,
    #[error("Antigravity provider requires a materialized prompt file path")]
    MissingPromptFile,
}

/// Identifier enum for the 6 supported Agent CLI provider modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProviderKind {
    Antigravity,
    Kiro,
    Claude,
    Codex,
    OpenCode,
    Custom,
}

impl AgentProviderKind {
    pub fn all_builtin() -> [AgentProviderKind; 5] {
        [
            AgentProviderKind::Antigravity,
            AgentProviderKind::Kiro,
            AgentProviderKind::Claude,
            AgentProviderKind::Codex,
            AgentProviderKind::OpenCode,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            AgentProviderKind::Antigravity => "Antigravity CLI",
            AgentProviderKind::Kiro => "Kiro CLI",
            AgentProviderKind::Claude => "Claude Code CLI",
            AgentProviderKind::Codex => "OpenAI Codex CLI",
            AgentProviderKind::OpenCode => "OpenCode CLI",
            AgentProviderKind::Custom => "Custom User CLI",
        }
    }

    pub fn default_command(&self) -> &'static str {
        match self {
            AgentProviderKind::Antigravity => "antigravity",
            AgentProviderKind::Kiro => "kiro",
            AgentProviderKind::Claude => "claude",
            AgentProviderKind::Codex => "codex",
            AgentProviderKind::OpenCode => "opencode",
            AgentProviderKind::Custom => "custom",
        }
    }

    pub fn command_aliases(&self) -> &'static [&'static str] {
        match self {
            AgentProviderKind::Antigravity => &["antigravity", "agy"],
            AgentProviderKind::Kiro => &["kiro-cli", "kiro"],
            AgentProviderKind::Claude => &["claude"],
            AgentProviderKind::Codex => &["codex"],
            AgentProviderKind::OpenCode => &["opencode"],
            AgentProviderKind::Custom => &["custom"],
        }
    }

    pub fn install_hint(&self) -> &'static str {
        match self {
            AgentProviderKind::Antigravity => {
                r"Install Antigravity CLI (`agy` / `antigravity`) or check %LOCALAPPDATA%\agy\bin\agy.EXE"
            }
            AgentProviderKind::Kiro => {
                r"Install Kiro CLI (`kiro-cli`) or check %LOCALAPPDATA%\Kiro-Cli\kiro-cli.exe"
            }
            AgentProviderKind::Claude => "npm install -g @anthropic-ai/claude-code",
            AgentProviderKind::Codex => "npm install -g @openai/codex",
            AgentProviderKind::OpenCode => "npm install -g opencode-ai",
            AgentProviderKind::Custom => "Configure custom executable path and args template",
        }
    }
}

/// Configuration for a user-defined Custom Agent CLI adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomCliConfig {
    pub executable: String,
    pub args_template: Vec<String>,
    #[serde(default)]
    pub health_check_args: Option<Vec<String>>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
}

impl CustomCliConfig {
    pub fn new(executable: impl Into<String>, args_template: Vec<String>) -> Self {
        Self {
            executable: executable.into(),
            args_template,
            health_check_args: None,
            working_dir: None,
        }
    }

    pub fn with_health_check_args(mut self, args: Vec<String>) -> Self {
        self.health_check_args = Some(args);
        self
    }

    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.executable.trim().is_empty() {
            return Err(ProviderError::EmptyExecutable);
        }
        if self.args_template.is_empty() {
            return Err(ProviderError::EmptyArgsTemplate);
        }
        let has_placeholder = self
            .args_template
            .iter()
            .any(|arg| arg.contains("{{PROMPT}}") || arg.contains("{{CONTEXT_JSON}}"));
        if !has_placeholder {
            return Err(ProviderError::MissingTemplatePlaceholder);
        }
        Ok(())
    }

    pub fn render_args(
        &self,
        prompt: &str,
        context_json: &str,
    ) -> Result<Vec<String>, ProviderError> {
        self.validate()?;
        Ok(self
            .args_template
            .iter()
            .map(|token| {
                token
                    .replace("{{PROMPT}}", prompt)
                    .replace("{{CONTEXT_JSON}}", context_json)
            })
            .collect())
    }
}

/// Pluggable Agent CLI provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentProvider {
    Antigravity,
    Kiro,
    Claude,
    Codex,
    OpenCode,
    Custom(CustomCliConfig),
}

/// Inputs required to construct a concrete CLI command invocation.
#[derive(Debug, Clone, Copy)]
pub struct InvocationInput<'a> {
    pub prompt: &'a str,
    pub context_json: &'a str,
    pub prompt_file_path: Option<&'a Path>,
}

/// Concrete command + discrete argv representation produced by a provider adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocationSpec {
    pub kind: AgentProviderKind,
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
}

impl AgentProvider {
    pub fn kind(&self) -> AgentProviderKind {
        match self {
            AgentProvider::Antigravity => AgentProviderKind::Antigravity,
            AgentProvider::Kiro => AgentProviderKind::Kiro,
            AgentProvider::Claude => AgentProviderKind::Claude,
            AgentProvider::Codex => AgentProviderKind::Codex,
            AgentProvider::OpenCode => AgentProviderKind::OpenCode,
            AgentProvider::Custom(_) => AgentProviderKind::Custom,
        }
    }

    pub fn default_binary_name(&self) -> &str {
        match self {
            AgentProvider::Custom(cfg) => &cfg.executable,
            _ => self.kind().default_command(),
        }
    }

    pub fn requires_prompt_file(&self) -> bool {
        matches!(self, AgentProvider::Antigravity)
    }

    /// Builds exact command + argument vector per Section 5.3.1 specifications.
    pub fn build_command_spec(
        &self,
        input: &InvocationInput<'_>,
    ) -> Result<CliInvocationSpec, ProviderError> {
        match self {
            AgentProvider::Antigravity => {
                let prompt_file = input
                    .prompt_file_path
                    .ok_or(ProviderError::MissingPromptFile)?;
                Ok(CliInvocationSpec {
                    kind: AgentProviderKind::Antigravity,
                    program: "antigravity".to_string(),
                    args: vec![
                        "chat".to_string(),
                        "--format".to_string(),
                        "json".to_string(),
                        "--prompt-file".to_string(),
                        prompt_file.to_string_lossy().into_owned(),
                    ],
                    working_dir: None,
                })
            }
            AgentProvider::Kiro => Ok(CliInvocationSpec {
                kind: AgentProviderKind::Kiro,
                program: "kiro".to_string(),
                args: vec![
                    "--no-interactive".to_string(),
                    "--trust-all-tools".to_string(),
                    "--prompt".to_string(),
                    input.prompt.to_string(),
                ],
                working_dir: None,
            }),
            AgentProvider::Claude => Ok(CliInvocationSpec {
                kind: AgentProviderKind::Claude,
                program: "claude".to_string(),
                args: vec![
                    "-p".to_string(),
                    input.prompt.to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--allowedTools".to_string(),
                    "WebSearch,WebFetch,Read".to_string(),
                ],
                working_dir: None,
            }),
            AgentProvider::Codex => Ok(CliInvocationSpec {
                kind: AgentProviderKind::Codex,
                program: "codex".to_string(),
                args: vec![
                    "exec".to_string(),
                    "--json".to_string(),
                    input.prompt.to_string(),
                ],
                working_dir: None,
            }),
            AgentProvider::OpenCode => Ok(CliInvocationSpec {
                kind: AgentProviderKind::OpenCode,
                program: "opencode".to_string(),
                args: vec![
                    "run".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    input.prompt.to_string(),
                ],
                working_dir: None,
            }),
            AgentProvider::Custom(cfg) => {
                let args = cfg.render_args(input.prompt, input.context_json)?;
                Ok(CliInvocationSpec {
                    kind: AgentProviderKind::Custom,
                    program: cfg.executable.clone(),
                    args,
                    working_dir: cfg.working_dir.clone(),
                })
            }
        }
    }

    /// Adapts invocation arguments when the resolved OS binary uses a newer or alternate
    /// CLI entry point (for example `kiro-cli.exe` v3, `agy.EXE`, or `opencode.exe`).
    pub fn adapt_args_for_resolved_binary(
        &self,
        resolved_program: &Path,
        input: &InvocationInput<'_>,
    ) -> Option<Vec<String>> {
        self.adapt_args_for_resolved_binary_with_model(resolved_program, input, None)
    }

    /// Adapts invocation arguments for a resolved binary with an optional provider-specific model override.
    pub fn adapt_args_for_resolved_binary_with_model(
        &self,
        resolved_program: &Path,
        input: &InvocationInput<'_>,
        model_override: Option<&str>,
    ) -> Option<Vec<String>> {
        let stem = resolved_program
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());

        match (self, stem.as_deref()) {
            (AgentProvider::Kiro, Some("kiro-cli")) => Some(vec![
                "chat".to_string(),
                "--no-interactive".to_string(),
                "--trust-workspace".to_string(),
                "--trust-tools=fs_read,web_search,web_fetch".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                input.prompt.to_string(),
            ]),
            (AgentProvider::Antigravity, Some("agy")) => Some(vec![
                "-p".to_string(),
                input.prompt.to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ]),
            (AgentProvider::OpenCode, Some("opencode")) => {
                let mut args = vec![
                    "run".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    "--print-logs".to_string(),
                    "--log-level".to_string(),
                    "ERROR".to_string(),
                ];
                if let Some(model) = model_override.map(str::trim).filter(|m| !m.is_empty()) {
                    args.push("-m".to_string());
                    args.push(model.to_string());
                }
                args.push(input.prompt.to_string());
                Some(args)
            }
            _ => None,
        }
    }
}

/// A locally discovered built-in Agent CLI binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredProvider {
    pub kind: AgentProviderKind,
    pub display_name: String,
    pub command: String,
    pub resolved_path: PathBuf,
}

/// Discovery status across all built-in Agent CLI providers (available or absent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub kind: AgentProviderKind,
    pub display_name: String,
    pub command: String,
    pub available: bool,
    pub resolved_path: Option<PathBuf>,
    pub install_hint: String,
}

/// Discovers all available built-in Agent CLIs on PATH and standard Windows install locations.
pub fn discover_available_providers() -> Vec<DiscoveredProvider> {
    discover_all_provider_statuses()
        .into_iter()
        .filter_map(|status| {
            status.resolved_path.map(|resolved_path| DiscoveredProvider {
                kind: status.kind,
                display_name: status.display_name,
                command: status.command,
                resolved_path,
            })
        })
        .collect()
}

/// Returns discovery status for every built-in provider without failing when some are absent.
pub fn discover_all_provider_statuses() -> Vec<ProviderStatus> {
    let (dirs, exts, extra) = default_search_dirs_and_exts();
    discover_providers_in(&dirs, &extra, &exts)
}

/// Pure discovery helper operating on explicit search directories and extra candidate paths.
pub fn discover_providers_in(
    path_dirs: &[PathBuf],
    extra_candidates: &[(AgentProviderKind, Vec<PathBuf>)],
    path_exts: &[String],
) -> Vec<ProviderStatus> {
    AgentProviderKind::all_builtin()
        .into_iter()
        .map(|kind| {
            let mut resolved = None;
            for alias in kind.command_aliases() {
                resolved = resolve_binary_in_dirs(alias, path_dirs, path_exts);
                if resolved.is_some() {
                    break;
                }
            }
            if resolved.is_none() {
                if let Some((_, candidates)) = extra_candidates.iter().find(|(k, _)| *k == kind) {
                    resolved = candidates
                        .iter()
                        .find(|path| path.is_file())
                        .map(|path| preserve_disk_filename_case(path));
                }
            }

            let command = resolved
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| kind.default_command().to_string());

            ProviderStatus {
                kind,
                display_name: kind.display_name().to_string(),
                command,
                available: resolved.is_some(),
                resolved_path: resolved,
                install_hint: kind.install_hint().to_string(),
            }
        })
        .collect()
}

/// Resolves a command name across search directories using Windows `PATHEXT` rules.
pub fn resolve_binary_in_dirs(
    command: &str,
    dirs: &[PathBuf],
    path_exts: &[String],
) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cmd_path = Path::new(trimmed);
    let has_path_sep =
        trimmed.contains('\\') || trimmed.contains('/') || cmd_path.components().count() > 1;
    let has_extension = cmd_path.extension().is_some();

    if has_path_sep {
        return check_candidate_with_exts(cmd_path, has_extension, path_exts);
    }

    for dir in dirs {
        let base = dir.join(trimmed);
        if let Some(found) = check_candidate_with_exts(&base, has_extension, path_exts) {
            return Some(found);
        }
    }
    None
}

fn check_candidate_with_exts(
    base: &Path,
    has_extension: bool,
    path_exts: &[String],
) -> Option<PathBuf> {
    if has_extension {
        return base.is_file().then(|| preserve_disk_filename_case(base));
    }

    // Check executable extensions first on Windows so `.exe`/`.cmd`/`.ps1` take precedence
    // over extensionless POSIX shell shims placed in npm global bin directories.
    let base_os = base.as_os_str();
    for ext in path_exts {
        let mut candidate = base_os.to_os_string();
        let normalized_ext = if ext.starts_with('.') {
            ext.clone()
        } else {
            format!(".{ext}")
        };
        candidate.push(&normalized_ext);
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(preserve_disk_filename_case(&path));
        }
    }

    base.is_file().then(|| preserve_disk_filename_case(base))
}

fn preserve_disk_filename_case(path: &Path) -> PathBuf {
    let (Some(parent), Some(target_name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let target_lossy = target_name.to_string_lossy();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&target_lossy)
            {
                return entry.path();
            }
        }
    }
    path.to_path_buf()
}

/// Candidate binary locations grouped by built-in provider kind.
pub type ProviderCandidateList = Vec<(AgentProviderKind, Vec<PathBuf>)>;

/// Default PATH directories, PATHEXT extensions, and per-provider candidate install locations.
pub type SearchDirsAndExts = (Vec<PathBuf>, Vec<String>, ProviderCandidateList);

/// Collects default PATH directories, PATHEXT extensions, and standard install locations.
pub fn default_search_dirs_and_exts() -> SearchDirsAndExts {
    let mut dirs = Vec::new();
    let mut seen_dirs = HashSet::new();

    let mut push_dir = |p: PathBuf| {
        let key = p.to_string_lossy().to_ascii_lowercase();
        if seen_dirs.insert(key) {
            dirs.push(p);
        }
    };

    if let Some(path_os) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&path_os) {
            if !p.as_os_str().is_empty() {
                push_dir(p);
            }
        }
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        push_dir(PathBuf::from(&appdata).join("npm"));
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(&profile);
        push_dir(home.join(".local").join("bin"));
        push_dir(home.join(".cargo").join("bin"));
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let local = PathBuf::from(&local_app_data);
        push_dir(local.join("Kiro-Cli"));
        push_dir(local.join("agy").join("bin"));
    }

    let mut exts = Vec::new();
    let mut seen_exts = HashSet::new();
    let raw_exts = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD;.PS1".to_string());
    for ext in raw_exts.split(';') {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            let lower = trimmed.to_ascii_lowercase();
            if seen_exts.insert(lower) {
                exts.push(trimmed.to_string());
            }
        }
    }
    for required in [".EXE", ".CMD", ".BAT", ".PS1"] {
        if seen_exts.insert(required.to_ascii_lowercase()) {
            exts.push(required.to_string());
        }
    }

    let mut extra = Vec::new();
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let local = PathBuf::from(&local_app_data);
        let programs = local.join("Programs");
        extra.push((
            AgentProviderKind::Antigravity,
            vec![
                local.join("agy").join("bin").join("agy.exe"),
                local.join("agy").join("bin").join("agy.EXE"),
                programs.join("Antigravity").join("bin").join("antigravity.exe"),
                programs.join("Antigravity").join("antigravity.exe"),
                programs.join("Google").join("Antigravity").join("antigravity.exe"),
            ],
        ));
        extra.push((
            AgentProviderKind::Kiro,
            vec![
                local.join("Kiro-Cli").join("kiro-cli.exe"),
                programs.join("Kiro").join("bin").join("kiro-cli.exe"),
                programs.join("Kiro").join("bin").join("kiro.exe"),
            ],
        ));
        extra.push((
            AgentProviderKind::Codex,
            vec![programs.join("OpenAI").join("Codex").join("bin").join("codex.exe")],
        ));
        extra.push((
            AgentProviderKind::Claude,
            vec![programs.join("Claude").join("claude.exe")],
        ));
        extra.push((
            AgentProviderKind::OpenCode,
            vec![programs.join("OpenCode").join("opencode.exe")],
        ));
    }

    (dirs, exts, extra)
}

fn strip_jsonc_comments_and_trailing_commas(input: &str) -> String {
    let mut no_comments = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            no_comments.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            no_comments.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek() {
                Some('/') => {
                    let _ = chars.next();
                    for c in chars.by_ref() {
                        if c == '\n' {
                            no_comments.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    let _ = chars.next();
                    let mut prev_star = false;
                    for c in chars.by_ref() {
                        if prev_star && c == '/' {
                            break;
                        }
                        prev_star = c == '*';
                    }
                    continue;
                }
                _ => {}
            }
        }

        no_comments.push(ch);
    }

    // Strip trailing commas before `}` or `]` outside double-quoted strings.
    let mut cleaned = String::with_capacity(no_comments.len());
    let mut c_iter = no_comments.chars().peekable();
    in_string = false;
    escaped = false;

    while let Some(ch) = c_iter.next() {
        if in_string {
            cleaned.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            cleaned.push(ch);
            continue;
        }
        if ch == ',' {
            let mut lookahead = c_iter.clone();
            while let Some(&next_ch) = lookahead.peek() {
                if next_ch.is_whitespace() {
                    let _ = lookahead.next();
                } else {
                    break;
                }
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        cleaned.push(ch);
    }

    cleaned
}

/// Builds a subprocess-only `OPENCODE_CONFIG_CONTENT` JSON override from raw OpenCode config
/// texts, explicitly disabling discovered MCP servers (avoiding >128 tool overflows and startup
/// stalls) and enforcing read-only (`edit: deny`, `bash: deny`) permissions.
pub fn build_opencode_isolated_config_from_contents(config_texts: &[&str]) -> String {
    let mut mcp_names = BTreeSet::new();

    for raw in config_texts {
        let stripped = strip_jsonc_comments_and_trailing_commas(raw);
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&stripped) else {
            continue;
        };
        if let Some(Value::Object(mcp_obj)) = root.get("mcp") {
            for key in mcp_obj.keys() {
                if !key.trim().is_empty() {
                    mcp_names.insert(key.clone());
                }
            }
        }
    }

    let mut root = Map::new();
    if !mcp_names.is_empty() {
        let mut mcp_map = Map::new();
        for name in mcp_names {
            let mut entry = Map::new();
            entry.insert("enabled".to_string(), Value::Bool(false));
            mcp_map.insert(name, Value::Object(entry));
        }
        root.insert("mcp".to_string(), Value::Object(mcp_map));
    }

    let mut permission = Map::new();
    permission.insert("edit".to_string(), Value::String("deny".to_string()));
    permission.insert("bash".to_string(), Value::String("deny".to_string()));
    root.insert("permission".to_string(), Value::Object(permission));

    Value::Object(root).to_string()
}

/// Reads user OpenCode config file(s) from standard locations (extracting only top-level `"mcp"`
/// server names, never reading/logging secrets) and builds a subprocess-only isolated config JSON.
pub fn build_opencode_isolated_config() -> String {
    let mut contents = Vec::new();
    let base_profile = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = base_profile {
        let candidates = [
            home.join(".config").join("opencode").join("opencode.json"),
            home.join(".config").join("opencode").join("opencode.jsonc"),
            home.join(".config").join("opencode").join("config.json"),
            home.join(".opencode").join("opencode.json"),
            home.join(".opencode").join("opencode.jsonc"),
        ];
        for path in candidates {
            if let Ok(text) = std::fs::read_to_string(path) {
                contents.push(text);
            }
        }
    }

    let refs: Vec<&str> = contents.iter().map(String::as_str).collect();
    build_opencode_isolated_config_from_contents(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("wds-agent-hub-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builds_exact_command_specs_for_all_six_providers() {
        let prompt = "Analyze C:\\Temp";
        let context_json = r#"{"path":"C:\\Temp"}"#;
        let prompt_file = Path::new(r"C:\Temp\prompt.txt");

        // 1. Antigravity
        let ag = AgentProvider::Antigravity
            .build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: Some(prompt_file),
            })
            .unwrap();
        assert_eq!(ag.program, "antigravity");
        assert_eq!(
            ag.args,
            vec![
                "chat",
                "--format",
                "json",
                "--prompt-file",
                r"C:\Temp\prompt.txt"
            ]
        );

        // Antigravity requires prompt file
        assert_eq!(
            AgentProvider::Antigravity.build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: None,
            }),
            Err(ProviderError::MissingPromptFile)
        );

        // 2. Kiro
        let kiro = AgentProvider::Kiro
            .build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: None,
            })
            .unwrap();
        assert_eq!(kiro.program, "kiro");
        assert_eq!(
            kiro.args,
            vec!["--no-interactive", "--trust-all-tools", "--prompt", prompt]
        );

        // 3. Claude
        let claude = AgentProvider::Claude
            .build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: None,
            })
            .unwrap();
        assert_eq!(claude.program, "claude");
        assert_eq!(
            claude.args,
            vec![
                "-p",
                prompt,
                "--output-format",
                "stream-json",
                "--allowedTools",
                "WebSearch,WebFetch,Read"
            ]
        );

        // 4. Codex
        let codex = AgentProvider::Codex
            .build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: None,
            })
            .unwrap();
        assert_eq!(codex.program, "codex");
        assert_eq!(codex.args, vec!["exec", "--json", prompt]);

        // 5. OpenCode
        let opencode = AgentProvider::OpenCode
            .build_command_spec(&InvocationInput {
                prompt,
                context_json,
                prompt_file_path: None,
            })
            .unwrap();
        assert_eq!(opencode.program, "opencode");
        assert_eq!(opencode.args, vec!["run", "--format", "json", prompt]);

        // 6. Custom
        let custom = AgentProvider::Custom(CustomCliConfig::new(
            r"C:\Tools\my-agent.exe",
            vec![
                "--ask".to_string(),
                "{{PROMPT}}".to_string(),
                "--ctx={{CONTEXT_JSON}}".to_string(),
            ],
        ))
        .build_command_spec(&InvocationInput {
            prompt,
            context_json,
            prompt_file_path: None,
        })
        .unwrap();
        assert_eq!(custom.program, r"C:\Tools\my-agent.exe");
        assert_eq!(
            custom.args,
            vec![
                "--ask".to_string(),
                prompt.to_string(),
                format!("--ctx={context_json}"),
            ]
        );
    }

    #[test]
    fn custom_cli_rejects_invalid_templates() {
        let empty_exe = CustomCliConfig::new("  ", vec!["{{PROMPT}}".to_string()]);
        assert_eq!(empty_exe.validate(), Err(ProviderError::EmptyExecutable));

        let empty_args = CustomCliConfig::new("my-agent", vec![]);
        assert_eq!(empty_args.validate(), Err(ProviderError::EmptyArgsTemplate));

        let no_placeholder = CustomCliConfig::new("my-agent", vec!["--flag".to_string()]);
        assert_eq!(
            no_placeholder.validate(),
            Err(ProviderError::MissingTemplatePlaceholder)
        );
    }

    #[test]
    fn discovers_available_providers_without_failing_when_some_are_missing() {
        let dir = unique_temp_dir("discover");
        let codex_exe = dir.join("codex.exe");
        let opencode_cmd = dir.join("opencode.cmd");
        let kiro_extra = dir.join("extra").join("kiro.exe");
        fs::create_dir_all(kiro_extra.parent().unwrap()).unwrap();
        fs::write(&codex_exe, b"stub").unwrap();
        fs::write(&opencode_cmd, b"@echo off").unwrap();
        fs::write(&kiro_extra, b"stub").unwrap();

        let exts = vec![".EXE".to_string(), ".CMD".to_string(), ".BAT".to_string()];
        let statuses = discover_providers_in(
            std::slice::from_ref(&dir),
            &[(AgentProviderKind::Kiro, vec![kiro_extra.clone()])],
            &exts,
        );

        assert_eq!(statuses.len(), 5);
        assert!(statuses.iter().any(|s| s.kind == AgentProviderKind::Codex
            && s.available
            && s.resolved_path.as_ref() == Some(&codex_exe)));
        assert!(statuses.iter().any(|s| s.kind == AgentProviderKind::OpenCode
            && s.available
            && s.resolved_path.as_ref() == Some(&opencode_cmd)));
        assert!(statuses.iter().any(|s| s.kind == AgentProviderKind::Kiro
            && s.available
            && s.resolved_path.as_ref() == Some(&kiro_extra)));
        assert!(statuses.iter().any(|s| s.kind == AgentProviderKind::Claude
            && !s.available
            && s.resolved_path.is_none()));
        assert!(statuses
            .iter()
            .any(|s| s.kind == AgentProviderKind::Antigravity
                && !s.available
                && s.resolved_path.is_none()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_discovery_never_panics() {
        let _ = discover_available_providers();
        let all = discover_all_provider_statuses();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn discovers_kiro_cli_and_agy_aliases_and_adapts_args() {
        let dir = unique_temp_dir("aliases");
        let kiro_cli_exe = dir.join("kiro-cli.exe");
        let agy_exe = dir.join("agy.EXE");
        fs::write(&kiro_cli_exe, b"stub").unwrap();
        fs::write(&agy_exe, b"stub").unwrap();

        let exts = vec![".EXE".to_string(), ".CMD".to_string()];
        let statuses = discover_providers_in(std::slice::from_ref(&dir), &[], &exts);

        let kiro_status = statuses
            .iter()
            .find(|s| s.kind == AgentProviderKind::Kiro)
            .expect("kiro status");
        assert!(kiro_status.available);
        assert_eq!(kiro_status.command, "kiro-cli");
        assert_eq!(kiro_status.resolved_path.as_ref(), Some(&kiro_cli_exe));

        let agy_status = statuses
            .iter()
            .find(|s| s.kind == AgentProviderKind::Antigravity)
            .expect("antigravity status");
        assert!(agy_status.available);
        assert_eq!(agy_status.command, "agy");
        assert_eq!(agy_status.resolved_path.as_ref(), Some(&agy_exe));

        let input = InvocationInput {
            prompt: "Should I delete C:\\Windows\\Temp?",
            context_json: "{}",
            prompt_file_path: None,
        };
        let adapted_kiro = AgentProvider::Kiro
            .adapt_args_for_resolved_binary(&kiro_cli_exe, &input)
            .expect("adapted kiro-cli args");
        assert_eq!(
            adapted_kiro,
            vec![
                "chat",
                "--no-interactive",
                "--trust-workspace",
                "--trust-tools=fs_read,web_search,web_fetch",
                "--output-format",
                "stream-json",
                "Should I delete C:\\Windows\\Temp?",
            ]
        );

        let adapted_agy = AgentProvider::Antigravity
            .adapt_args_for_resolved_binary(&agy_exe, &input)
            .expect("adapted agy args");
        assert_eq!(
            adapted_agy,
            vec![
                "-p",
                "Should I delete C:\\Windows\\Temp?",
                "--output-format",
                "json",
            ]
        );

        // Legacy names keep build_command_spec args unchanged.
        assert!(AgentProvider::Kiro
            .adapt_args_for_resolved_binary(Path::new("kiro.exe"), &input)
            .is_none());
        assert!(AgentProvider::Antigravity
            .adapt_args_for_resolved_binary(Path::new("antigravity.exe"), &input)
            .is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_opencode_isolated_config_from_json_and_jsonc_and_adapts_opencode_args() {
        let cfg1 = r#"
        {
          // main user opencode config
          "model": "opencode/muse-spark-1.3-contributor-free",
          "mcp": {
            "browseros-neo": { "type": "remote", "url": "http://localhost:9222/mcp" },
            "storage-agent": { "type": "local", "command": ["node", "agent.js"] },
          },
        }"#;
        let cfg2 = r#"
        {
          /* secondary config */
          "mcp": {
            "rea": { "enabled": true },
            "ghidra-mcp": { "enabled": true }
          }
        }"#;

        let isolated_str = build_opencode_isolated_config_from_contents(&[cfg1, cfg2]);
        let isolated: Value = serde_json::from_str(&isolated_str).expect("valid json");
        assert_eq!(isolated["mcp"]["browseros-neo"]["enabled"], Value::Bool(false));
        assert_eq!(isolated["mcp"]["storage-agent"]["enabled"], Value::Bool(false));
        assert_eq!(isolated["mcp"]["rea"]["enabled"], Value::Bool(false));
        assert_eq!(isolated["mcp"]["ghidra-mcp"]["enabled"], Value::Bool(false));
        assert_eq!(isolated["permission"]["edit"], Value::String("deny".to_string()));
        assert_eq!(isolated["permission"]["bash"], Value::String("deny".to_string()));

        // No MCPs -> omits mcp key, keeps read-only permission.
        let bare: Value =
            serde_json::from_str(&build_opencode_isolated_config_from_contents(&[])).unwrap();
        assert!(bare.get("mcp").is_none());
        assert_eq!(bare["permission"]["bash"], Value::String("deny".to_string()));

        let input = InvocationInput {
            prompt: "Say hello",
            context_json: "{}",
            prompt_file_path: None,
        };
        let opencode_exe = Path::new(r"C:\Users\thinh\AppData\Roaming\npm\node_modules\opencode-ai\bin\opencode.exe");
        assert_eq!(
            AgentProvider::OpenCode
                .adapt_args_for_resolved_binary_with_model(opencode_exe, &input, None)
                .unwrap(),
            vec!["run", "--format", "json", "--print-logs", "--log-level", "ERROR", "Say hello"]
        );
        assert_eq!(
            AgentProvider::OpenCode
                .adapt_args_for_resolved_binary_with_model(
                    opencode_exe,
                    &input,
                    Some("  opencode/big-pickle  ")
                )
                .unwrap(),
            vec![
                "run",
                "--format",
                "json",
                "--print-logs",
                "--log-level",
                "ERROR",
                "-m",
                "opencode/big-pickle",
                "Say hello"
            ]
        );
        // Mock wrapper running under node.exe keeps build_command_spec args untouched.
        assert!(AgentProvider::OpenCode
            .adapt_args_for_resolved_binary_with_model(Path::new("node.exe"), &input, Some("opencode/big-pickle"))
            .is_none());
    }
}

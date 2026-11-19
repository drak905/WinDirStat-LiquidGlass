use crate::agent_hub::context::{ContextError, StorageContextPayload};
use crate::agent_hub::prompt::build_cross_check_system_prompt;
use crate::agent_hub::providers::{
    default_search_dirs_and_exts, resolve_binary_in_dirs, AgentProvider, AgentProviderKind,
    InvocationInput, ProviderError,
};
use crate::agent_hub::validator::{
    parse_and_validate_advisor_response, AdvisorResponse, ValidationError,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

static TEMP_PROMPT_SEQ: AtomicU64 = AtomicU64::new(1);

/// Errors raised during Agent CLI resolution, execution, timeout, or response validation.
#[derive(Debug, Error)]
pub enum AgentHubError {
    #[error(transparent)]
    Context(#[from] ContextError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("agent CLI executable `{program}` was not found on PATH or configured overrides")]
    BinaryNotFound { program: String },
    #[error("failed to create or clean up prompt file: {0}")]
    PromptFileIo(String),
    #[error("failed to spawn agent CLI `{program}`: {reason}")]
    SpawnFailed { program: String, reason: String },
    #[error("agent CLI `{program}` timed out after {timeout_ms} ms")]
    Timeout { program: String, timeout_ms: u128 },
    #[error("agent CLI `{program}` exited with status {exit_code:?}: {stderr}")]
    ProcessFailed {
        program: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("connectivity self-test failed for {provider}: {reason}")]
    SelfTestFailed { provider: String, reason: String },
}

/// Identifies which subprocess stream produced a [`StreamChunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSource {
    Stdout,
    Stderr,
}

/// A real-time line emitted by an Agent CLI subprocess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChunk {
    pub source: StreamSource,
    pub line: String,
}

/// Resolved OS command plus any wrapper prefix arguments (e.g., `node` + script or `powershell` + `-File`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
}

impl ResolvedCommand {
    pub fn direct(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            prefix_args: Vec::new(),
        }
    }

    pub fn with_prefix(program: impl Into<PathBuf>, prefix_args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            prefix_args,
        }
    }
}

/// Pluggable binary resolver allowing tests or custom UI overrides to redirect CLI executables.
pub trait CommandResolver: Send + Sync {
    fn resolve(&self, requested_program: &str) -> Result<ResolvedCommand, AgentHubError>;
}

/// Default OS resolver using PATH, Windows `PATHEXT`, and standard Agent CLI install paths.
#[derive(Debug, Default)]
pub struct DefaultCommandResolver;

impl CommandResolver for DefaultCommandResolver {
    fn resolve(&self, requested_program: &str) -> Result<ResolvedCommand, AgentHubError> {
        let trimmed = requested_program.trim();
        if trimmed.is_empty() {
            return Err(AgentHubError::BinaryNotFound {
                program: requested_program.to_string(),
            });
        }

        let (dirs, exts, extra) = default_search_dirs_and_exts();
        let mut found = resolve_binary_in_dirs(trimmed, &dirs, &exts);

        if found.is_none() {
            if let Some(kind) = AgentProviderKind::all_builtin()
                .into_iter()
                .find(|k| k.default_command().eq_ignore_ascii_case(trimmed))
            {
                for alias in kind.command_aliases() {
                    if !alias.eq_ignore_ascii_case(trimmed) {
                        found = resolve_binary_in_dirs(alias, &dirs, &exts);
                        if found.is_some() {
                            break;
                        }
                    }
                }
                if found.is_none() {
                    if let Some((_, candidates)) = extra.into_iter().find(|(k, _)| *k == kind) {
                        found = candidates.into_iter().find(|p| p.is_file());
                    }
                }
            }
        }

        let resolved_path = found.ok_or_else(|| AgentHubError::BinaryNotFound {
            program: requested_program.to_string(),
        })?;

        Ok(wrap_platform_script(resolved_path))
    }
}

fn wrap_platform_script(resolved_path: PathBuf) -> ResolvedCommand {
    let ext = resolved_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());

    match ext.as_deref() {
        Some("ps1") => ResolvedCommand::with_prefix(
            "powershell.exe",
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                resolved_path.to_string_lossy().into_owned(),
            ],
        ),
        Some("cmd") | Some("bat") => {
            if let Some(unwrapped) = try_unwrap_npm_cmd_shim(&resolved_path) {
                return unwrapped;
            }
            ResolvedCommand::with_prefix(
                "cmd.exe",
                vec![
                    "/C".to_string(),
                    resolved_path.to_string_lossy().into_owned(),
                ],
            )
        }
        _ => ResolvedCommand::direct(resolved_path),
    }
}

/// Unwraps npm-generated `.cmd` shims (`%dp0%\node_modules\...\bin\tool.exe` or
/// `node_modules\...\cli.js`) so multi-line prompts pass directly via `CreateProcessW`
/// without `cmd.exe` metacharacter/newline splitting.
fn try_unwrap_npm_cmd_shim(cmd_path: &Path) -> Option<ResolvedCommand> {
    let parent = cmd_path.parent()?;
    let metadata = std::fs::metadata(cmd_path).ok()?;
    if metadata.len() > 8192 {
        return None;
    }
    let content = std::fs::read_to_string(cmd_path).ok()?;

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("%*") {
            continue;
        }
        for token in trimmed.split('"') {
            let rel = token
                .strip_prefix(r"%dp0%\")
                .or_else(|| token.strip_prefix(r"%~dp0\"))
                .or_else(|| token.strip_prefix("%dp0%/"))
                .or_else(|| token.strip_prefix("%~dp0/"));
            let Some(rel_path) = rel else {
                continue;
            };
            let target = parent.join(rel_path.replace('/', "\\"));
            if !target.is_file() {
                continue;
            }
            let target_ext = target
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase());
            match target_ext.as_deref() {
                Some("exe") => return Some(ResolvedCommand::direct(target)),
                Some("js") | Some("mjs") | Some("cjs") => {
                    let (dirs, exts, _) = default_search_dirs_and_exts();
                    let node_exe = resolve_binary_in_dirs("node", &dirs, &exts)?;
                    return Some(ResolvedCommand::with_prefix(
                        node_exe,
                        vec![target.to_string_lossy().into_owned()],
                    ));
                }
                _ => {}
            }
        }
    }
    None
}

/// Test/override resolver mapping logical command names to explicit executables or runner scripts.
#[derive(Debug, Default)]
pub struct OverrideCommandResolver {
    overrides: HashMap<String, ResolvedCommand>,
    fallback_to_default: bool,
}

impl OverrideCommandResolver {
    pub fn new(fallback_to_default: bool) -> Self {
        Self {
            overrides: HashMap::new(),
            fallback_to_default,
        }
    }

    pub fn insert(&mut self, command: impl Into<String>, resolved: ResolvedCommand) {
        self.overrides.insert(command.into(), resolved);
    }
}

impl CommandResolver for OverrideCommandResolver {
    fn resolve(&self, requested_program: &str) -> Result<ResolvedCommand, AgentHubError> {
        if let Some(resolved) = self.overrides.get(requested_program) {
            return Ok(resolved.clone());
        }
        if self.fallback_to_default {
            return DefaultCommandResolver.resolve(requested_program);
        }
        Err(AgentHubError::BinaryNotFound {
            program: requested_program.to_string(),
        })
    }
}

/// Timeout and runtime configuration for [`AgentHubRunner`].
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub timeout: Duration,
    pub self_test_timeout: Duration,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            self_test_timeout: Duration::from_secs(15),
        }
    }
}

/// Result returned from a validated Agent CLI execution.
#[derive(Debug, Clone)]
pub struct ExecutionOutput {
    pub provider_kind: AgentProviderKind,
    pub invoked_program: PathBuf,
    pub invoked_args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub response: AdvisorResponse,
}

/// Raw subprocess execution output before strict advisor schema validation.
#[derive(Debug, Clone)]
pub struct RawChatExecutionOutput {
    pub provider_kind: AgentProviderKind,
    pub invoked_program: PathBuf,
    pub invoked_args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
}

/// Result returned by [`AgentHubRunner::self_test_provider`].
#[derive(Debug, Clone)]
pub struct ConnectivityReport {
    pub provider_kind: AgentProviderKind,
    pub resolved_program: PathBuf,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub ok: bool,
}

/// RAII guard ensuring temporary prompt files created for Antigravity CLI are always removed.
struct TempPromptFileGuard {
    path: PathBuf,
}

impl TempPromptFileGuard {
    async fn create(content: &str) -> Result<Self, AgentHubError> {
        let seq = TEMP_PROMPT_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("wds-antigravity-prompt-{}-{seq}.txt", std::process::id()));
        tokio::fs::write(&path, content.as_bytes())
            .await
            .map_err(|e| AgentHubError::PromptFileIo(format!("{}: {e}", path.display())))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPromptFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct RawProcessOutput {
    program: PathBuf,
    args: Vec<String>,
    exit_code: Option<i32>,
    success: bool,
    stdout: String,
    stderr: String,
    duration_ms: u128,
}

/// Asynchronous subprocess router executing Agent CLIs with streaming and strict validation.
pub struct AgentHubRunner {
    resolver: Arc<dyn CommandResolver>,
    config: RunnerConfig,
}

impl Default for AgentHubRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentHubRunner {
    pub fn new() -> Self {
        Self {
            resolver: Arc::new(DefaultCommandResolver),
            config: RunnerConfig::default(),
        }
    }

    pub fn with_resolver(resolver: Arc<dyn CommandResolver>, config: RunnerConfig) -> Self {
        Self { resolver, config }
    }

    /// Executes an Agent CLI with a prompt and optional context JSON, adapting binary-specific
    /// flags when resolved to alternate aliases (`kiro-cli`, `agy`) and returning raw stdout/stderr.
    pub async fn run_raw_prompt(
        &self,
        provider: &AgentProvider,
        prompt: &str,
        context_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<RawChatExecutionOutput, AgentHubError> {
        let _prompt_guard = if provider.requires_prompt_file() {
            Some(TempPromptFileGuard::create(prompt).await?)
        } else {
            None
        };

        let inv_input = InvocationInput {
            prompt,
            context_json,
            prompt_file_path: _prompt_guard.as_ref().map(|g| g.path()),
        };

        let spec = provider.build_command_spec(&inv_input)?;
        let resolved = self.resolver.resolve(&spec.program)?;
        let effective_args = provider
            .adapt_args_for_resolved_binary(&resolved.program, &inv_input)
            .unwrap_or(spec.args);

        let raw = execute_subprocess(
            &resolved,
            &effective_args,
            spec.working_dir.as_deref(),
            self.config.timeout,
            stream_tx,
        )
        .await?;

        if !raw.success {
            return Err(AgentHubError::ProcessFailed {
                program: spec.program,
                exit_code: raw.exit_code,
                stdout: raw.stdout,
                stderr: raw.stderr,
            });
        }

        Ok(RawChatExecutionOutput {
            provider_kind: spec.kind,
            invoked_program: raw.program,
            invoked_args: raw.args,
            exit_code: raw.exit_code,
            stdout: raw.stdout,
            stderr: raw.stderr,
            duration_ms: raw.duration_ms,
        })
    }

    /// Executes the selected Agent CLI with injected Storage Context Payload and mandatory
    /// multi-source cross-check system prompt, returning only strictly validated responses.
    pub async fn run_advisor(
        &self,
        provider: &AgentProvider,
        payload: &StorageContextPayload,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<ExecutionOutput, AgentHubError> {
        payload.validate()?;
        let context_json = payload.to_compact_json()?;
        let prompt = build_cross_check_system_prompt(payload);

        let raw = self
            .run_raw_prompt(provider, &prompt, &context_json, stream_tx)
            .await?;

        let response = parse_and_validate_advisor_response(&raw.stdout)?;
        Ok(ExecutionOutput {
            provider_kind: raw.provider_kind,
            invoked_program: raw.invoked_program,
            invoked_args: raw.invoked_args,
            exit_code: raw.exit_code,
            stdout: raw.stdout,
            stderr: raw.stderr,
            duration_ms: raw.duration_ms,
            response,
        })
    }

    /// Runs a lightweight connectivity self-test against the resolved provider binary.
    pub async fn self_test_provider(
        &self,
        provider: &AgentProvider,
    ) -> Result<ConnectivityReport, AgentHubError> {
        let (probe_args, working_dir) = match provider {
            AgentProvider::Custom(cfg) => {
                cfg.validate()?;
                let args = cfg
                    .health_check_args
                    .clone()
                    .unwrap_or_else(|| vec!["--version".to_string()]);
                (args, cfg.working_dir.clone())
            }
            _ => (vec!["--version".to_string()], None),
        };

        let resolved = self.resolver.resolve(provider.default_binary_name())?;
        let raw = execute_subprocess(
            &resolved,
            &probe_args,
            working_dir.as_deref(),
            self.config.self_test_timeout,
            None,
        )
        .await
        .map_err(|e| AgentHubError::SelfTestFailed {
            provider: provider.kind().display_name().to_string(),
            reason: e.to_string(),
        })?;

        if !raw.success {
            return Err(AgentHubError::SelfTestFailed {
                provider: provider.kind().display_name().to_string(),
                reason: format!(
                    "exited with status {:?}: {}",
                    raw.exit_code,
                    raw.stderr.trim()
                ),
            });
        }

        Ok(ConnectivityReport {
            provider_kind: provider.kind(),
            resolved_program: raw.program,
            exit_code: raw.exit_code,
            stdout: raw.stdout,
            stderr: raw.stderr,
            duration_ms: raw.duration_ms,
            ok: true,
        })
    }
}

async fn execute_subprocess(
    resolved: &ResolvedCommand,
    spec_args: &[String],
    working_dir: Option<&Path>,
    timeout: Duration,
    stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
) -> Result<RawProcessOutput, AgentHubError> {
    let started = Instant::now();
    let mut full_args = resolved.prefix_args.clone();
    full_args.extend(spec_args.iter().cloned());

    let mut cmd = Command::new(&resolved.program);
    cmd.args(&full_args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn().map_err(|e| AgentHubError::SpawnFailed {
        program: resolved.program.display().to_string(),
        reason: e.to_string(),
    })?;

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| AgentHubError::SpawnFailed {
            program: resolved.program.display().to_string(),
            reason: "failed to capture stdout".to_string(),
        })?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| AgentHubError::SpawnFailed {
            program: resolved.program.display().to_string(),
            reason: "failed to capture stderr".to_string(),
        })?;

    let stdout_task = tokio::spawn(drain_stream(stdout_pipe, StreamSource::Stdout, stream_tx.clone()));
    let stderr_task = tokio::spawn(drain_stream(stderr_pipe, StreamSource::Stderr, stream_tx));

    let wait_res = tokio::time::timeout(timeout, child.wait()).await;
    let status = match wait_res {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            stdout_task.abort();
            stderr_task.abort();
            return Err(AgentHubError::SpawnFailed {
                program: resolved.program.display().to_string(),
                reason: e.to_string(),
            });
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(AgentHubError::Timeout {
                program: resolved.program.display().to_string(),
                timeout_ms: timeout.as_millis(),
            });
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();

    Ok(RawProcessOutput {
        program: resolved.program.clone(),
        args: full_args,
        exit_code: status.code(),
        success: status.success(),
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
    })
}

async fn drain_stream<R: AsyncRead + Unpin>(
    reader: R,
    source: StreamSource,
    stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut accumulated = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(tx) = &stream_tx {
            let _ = tx.send(StreamChunk {
                source,
                line: line.clone(),
            });
        }
        accumulated.push_str(&line);
        accumulated.push('\n');
    }
    accumulated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_resolver_returns_registered_command() {
        let mut resolver = OverrideCommandResolver::new(false);
        resolver.insert(
            "codex",
            ResolvedCommand::with_prefix("node", vec!["mock.js".to_string()]),
        );
        let resolved = resolver.resolve("codex").expect("registered");
        assert_eq!(resolved.program, PathBuf::from("node"));
        assert_eq!(resolved.prefix_args, vec!["mock.js".to_string()]);
        assert!(matches!(
            resolver.resolve("missing"),
            Err(AgentHubError::BinaryNotFound { .. })
        ));
    }

    #[test]
    fn runner_config_has_sensible_defaults() {
        let cfg = RunnerConfig::default();
        assert_eq!(cfg.timeout, Duration::from_secs(120));
        assert_eq!(cfg.self_test_timeout, Duration::from_secs(15));
    }

    #[test]
    fn unwraps_npm_exe_cmd_shim_and_falls_back_for_generic_cmd() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wds-shim-test-{}-{nanos}", std::process::id()));
        let bin_dir = dir.join("node_modules").join("opencode-ai").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let target_exe = bin_dir.join("opencode.exe");
        std::fs::write(&target_exe, b"stub").unwrap();

        let shim_cmd = dir.join("opencode.cmd");
        std::fs::write(
            &shim_cmd,
            "@ECHO off\r\n\"%dp0%\\node_modules\\opencode-ai\\bin\\opencode.exe\"   %*\r\n",
        )
        .unwrap();

        let wrapped = wrap_platform_script(shim_cmd);
        assert_eq!(wrapped.program, target_exe);
        assert!(wrapped.prefix_args.is_empty());

        let generic_cmd = dir.join("generic.cmd");
        std::fs::write(&generic_cmd, "@ECHO off\r\necho hello\r\n").unwrap();
        let fallback = wrap_platform_script(generic_cmd.clone());
        assert_eq!(fallback.program, PathBuf::from("cmd.exe"));
        assert_eq!(
            fallback.prefix_args,
            vec!["/C".to_string(), generic_cmd.to_string_lossy().into_owned()]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

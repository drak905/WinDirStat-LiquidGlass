use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use windirstat_liquid_glass::agent_hub::{
    default_search_dirs_and_exts, resolve_binary_in_dirs, AdvisorVerdict, AgentHubError,
    AgentHubRunner, AgentProvider, AgentProviderKind, CustomCliConfig, ExtensionStat,
    LockedHandleStatus, OverrideCommandResolver, ResolvedCommand, RunnerConfig,
    StorageContextPayload, ValidationError,
};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wds-tier2-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn find_node_exe() -> PathBuf {
    let (dirs, exts, _) = default_search_dirs_and_exts();
    if let Some(p) = resolve_binary_in_dirs("node", &dirs, &exts) {
        return p;
    }
    let fallback = PathBuf::from(r"C:\Program Files\nodejs\node.exe");
    assert!(
        fallback.is_file(),
        "node.exe is required for Tier 2 mock CLI integration tests"
    );
    fallback
}

fn sample_payload() -> StorageContextPayload {
    StorageContextPayload {
        path: r"C:\Users\dev\AppData\Local\Temp\wds-cache".to_string(),
        logical_size_bytes: 4_294_967_296,
        physical_size_bytes: 4_300_000_000,
        file_count: 1520,
        oldest_modified_utc: Some("2025-01-01T00:00:00Z".to_string()),
        newest_modified_utc: Some("2026-10-01T12:00:00Z".to_string()),
        publisher_signature: None,
        top_extensions: vec![
            ExtensionStat::new(".tmp", 1200, 3_500_000_000),
            ExtensionStat::new(".log", 320, 794_967_296),
        ],
        locked_handle_status: LockedHandleStatus::Unlocked,
        user_question: "Can I clean this Temp cache folder safely?".to_string(),
    }
}

fn write_mock_node_script(dir: &TempDirGuard, name: &str) -> PathBuf {
    let script_path = dir.path.join(name);
    let script = r####"
const fs = require('fs');
const args = process.argv.slice(2);
const mode = args[0];

if (args.includes('--version') || args.includes('--ping')) {
  console.log(`mock-cli ${mode} ok`);
  process.exit(0);
}

const validPayload = {
  verdict: "SAFE_TO_DELETE",
  risk_score: 8,
  reclaimable_bytes: 4294967296,
  usage_value_assessment: "Transient application temp cache files safe to purge when unused.",
  cross_checked_sources: [
    {
      title: "Microsoft Learn - Windows Temp Folder Guidance",
      url: "https://learn.microsoft.com/en-us/windows/win32/fileio/creating-and-using-a-temporary-file",
      source_type: "A",
      finding: "Files inside user Temp directories are temporary by contract."
    },
    {
      title: "Windows Directory Encyclopedia - AppData Local Temp",
      url: "https://ss64.com/nt/syntax-folders.html",
      source_type: "B",
      finding: "AppData\\Local\\Temp holds ephemeral installer and scratch files."
    },
    {
      title: "SuperUser - Cleaning AppData Local Temp safely",
      url: "https://superuser.com/questions/426195/is-it-safe-to-delete-appdata-local-temp",
      source_type: "C",
      finding: "Deleting unlocked files in Local\\Temp does not harm installed software."
    }
  ],
  recommended_action: "Route unlocked temporary files through the 7-Day Quarantine Vault."
};

function assertContains(text, needle, label) {
  if (!text || !text.includes(needle)) {
    console.error(`Missing expected ${label}: ${needle}`);
    process.exit(2);
  }
}

if (mode === 'antigravity') {
  if (args[1] !== 'chat' || args[2] !== '--format' || args[3] !== 'json' || args[4] !== '--prompt-file') {
    console.error('Unexpected antigravity args: ' + JSON.stringify(args));
    process.exit(2);
  }
  const promptFile = args[5];
  const promptText = fs.readFileSync(promptFile, 'utf8');
  assertContains(promptText, 'AT LEAST THREE (3)', 'cross-check directive');
  assertContains(promptText, 'wds-cache', 'payload path');
  console.log(JSON.stringify({ type: 'assistant', content: JSON.stringify(validPayload) }));
  process.exit(0);
}

if (mode === 'kiro') {
  if (args[1] !== '--no-interactive' || args[2] !== '--trust-all-tools' || args[3] !== '--prompt') {
    console.error('Unexpected kiro args: ' + JSON.stringify(args));
    process.exit(2);
  }
  assertContains(args[4], 'Source Type A', 'prompt');
  console.log("### Kiro Analysis\n```json\n" + JSON.stringify(validPayload, null, 2) + "\n```");
  process.exit(0);
}

if (mode === 'kiro-cli') {
  if (
    args[1] !== 'chat' ||
    args[2] !== '--no-interactive' ||
    args[3] !== '--trust-workspace' ||
    args[4] !== '--effort' ||
    args[5] !== 'medium' ||
    args[6] !== '--trust-tools=fs_read,web_search,web_fetch' ||
    args[7] !== '--output-format' ||
    args[8] !== 'stream-json'
  ) {
    console.error('Unexpected kiro-cli args: ' + JSON.stringify(args));
    process.exit(2);
  }
  assertContains(args[9], 'Source Type A', 'prompt');
  const fenced = "```json\n" + JSON.stringify(validPayload) + "\n```";
  console.log(JSON.stringify({ type: 'runStarted', data: { payloadSchema: 'acp', engine: 'v3' } }));
  console.log(JSON.stringify({ type: 'sessionUpdate', data: { sessionId: 'sess_mock', update: { sessionUpdate: 'session_info_update' } } }));
  console.log(JSON.stringify({ type: 'runFinished', data: { sessionId: 'sess_mock', status: 'success', finalText: fenced } }));
  process.exit(0);
}

if (mode === 'claude') {
  if (args[1] !== '-p' || args[3] !== '--output-format' || args[4] !== 'stream-json' || args[5] !== '--allowedTools' || args[6] !== 'WebSearch,WebFetch,Read') {
    console.error('Unexpected claude args: ' + JSON.stringify(args));
    process.exit(2);
  }
  assertContains(args[2], '7-Day Quarantine Vault', 'prompt');
  const fullText = "```json\n" + JSON.stringify(validPayload) + "\n```";
  const mid = Math.floor(fullText.length / 2);
  console.log(JSON.stringify({ type: 'system', subtype: 'init' }));
  console.log(JSON.stringify({ type: 'content_block_delta', delta: { type: 'text_delta', text: fullText.slice(0, mid) } }));
  console.log(JSON.stringify({ type: 'content_block_delta', delta: { type: 'text_delta', text: fullText.slice(mid) } }));
  console.log(JSON.stringify({ type: 'result', subtype: 'success', result: 'ok' }));
  process.exit(0);
}

if (mode === 'codex') {
  if (args[1] !== 'exec' || args[2] !== '--json') {
    console.error('Unexpected codex args: ' + JSON.stringify(args));
    process.exit(2);
  }
  assertContains(args[3], 'wds-cache', 'prompt');
  console.log(JSON.stringify({ type: 'thread.started', thread_id: 'thr_test' }));
  console.log(JSON.stringify({
    type: 'item.completed',
    item: { id: 'msg_1', type: 'agent_message', text: JSON.stringify(validPayload) }
  }));
  process.exit(0);
}

if (mode === 'opencode') {
  if (args[1] !== 'run' || args[2] !== '--format' || args[3] !== 'json') {
    console.error('Unexpected opencode args: ' + JSON.stringify(args));
    process.exit(2);
  }
  assertContains(args[4], 'wds-cache', 'prompt');
  console.log(JSON.stringify({ type: 'text', part: { text: JSON.stringify(validPayload) } }));
  process.exit(0);
}

if (mode === 'custom') {
  if (args[1] !== '--prompt' || args[3] !== '--context') {
    console.error('Unexpected custom args: ' + JSON.stringify(args));
    process.exit(2);
  }
  const ctx = JSON.parse(args[4]);
  if (!ctx.path.includes('wds-cache')) {
    console.error('Missing path in context JSON');
    process.exit(2);
  }
  console.log(JSON.stringify(validPayload));
  process.exit(0);
}

if (mode === 'sleep') {
  setTimeout(() => {
    console.log(JSON.stringify(validPayload));
  }, 5000);
} else if (mode === 'fail_exit') {
  console.error('simulated cli failure');
  process.exit(7);
} else if (mode === 'bad_lt3_urls') {
  const bad = { ...validPayload, cross_checked_sources: validPayload.cross_checked_sources.slice(0, 2) };
  console.log(JSON.stringify(bad));
} else if (mode === 'bad_dup_urls') {
  const bad = {
    ...validPayload,
    cross_checked_sources: [
      validPayload.cross_checked_sources[0],
      { ...validPayload.cross_checked_sources[1], url: validPayload.cross_checked_sources[0].url + "#frag" },
      validPayload.cross_checked_sources[2]
    ]
  };
  console.log(JSON.stringify(bad));
} else if (mode === 'bad_invalid_url') {
  const bad = {
    ...validPayload,
    cross_checked_sources: [
      { ...validPayload.cross_checked_sources[0], url: "file:///C:/Windows/System32" },
      validPayload.cross_checked_sources[1],
      validPayload.cross_checked_sources[2]
    ]
  };
  console.log(JSON.stringify(bad));
} else if (mode === 'bad_missing_verdict') {
  const { verdict, ...bad } = validPayload;
  console.log(JSON.stringify(bad));
} else if (mode === 'bad_missing_risk') {
  const { risk_score, ...bad } = validPayload;
  console.log(JSON.stringify(bad));
} else if (mode === 'bad_range_risk') {
  console.log(JSON.stringify({ ...validPayload, risk_score: 142 }));
} else {
  console.error('Unknown mode: ' + mode);
  process.exit(3);
}
"####;
    fs::write(&script_path, script).expect("write mock node script");
    script_path
}

fn runner_with_mock_modes(node_exe: &Path, script_path: &Path) -> AgentHubRunner {
    let mut resolver = OverrideCommandResolver::new(false);
    for name in ["antigravity", "kiro", "claude", "codex", "opencode"] {
        resolver.insert(
            name,
            ResolvedCommand::with_prefix(
                node_exe,
                vec![script_path.to_string_lossy().into_owned(), name.to_string()],
            ),
        );
    }
    AgentHubRunner::with_resolver(Arc::new(resolver), RunnerConfig::default())
}

#[tokio::test]
async fn executes_all_six_providers_end_to_end_and_streams_output() {
    let dir = TempDirGuard::new("all-six");
    let node_exe = find_node_exe();
    let script_path = write_mock_node_script(&dir, "mock-agent.js");
    let runner = runner_with_mock_modes(&node_exe, &script_path);
    let payload = sample_payload();

    let builtin_providers = [
        AgentProvider::Antigravity,
        AgentProvider::Kiro,
        AgentProvider::Claude,
        AgentProvider::Codex,
        AgentProvider::OpenCode,
    ];

    for provider in builtin_providers {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let output = runner
            .run_advisor(&provider, &payload, Some(tx))
            .await
            .unwrap_or_else(|e| panic!("provider {:?} failed: {e}", provider.kind()));

        assert_eq!(output.provider_kind, provider.kind());
        assert_eq!(output.response.verdict, AdvisorVerdict::SafeToDelete);
        assert_eq!(output.response.risk_score, 8);
        assert_eq!(output.response.cross_checked_sources.len(), 3);

        // Verify streaming channel received at least one line.
        let mut streamed_lines = 0usize;
        while rx.try_recv().is_ok() {
            streamed_lines += 1;
        }
        assert!(streamed_lines >= 1, "expected streamed lines for {:?}", provider.kind());

        // For Antigravity, assert temp prompt file was cleaned up after execution.
        if provider.kind() == AgentProviderKind::Antigravity {
            let prompt_file_arg = output.invoked_args.last().expect("prompt file arg");
            assert!(
                !std::path::Path::new(prompt_file_arg).exists(),
                "Antigravity temp prompt file should be deleted after run: {prompt_file_arg}"
            );
        }
    }

    // Custom provider with {{PROMPT}}, {{CONTEXT_JSON}}, and connectivity self-test.
    let custom_cfg = CustomCliConfig::new(
        node_exe.to_string_lossy().into_owned(),
        vec![
            script_path.to_string_lossy().into_owned(),
            "custom".to_string(),
            "--prompt".to_string(),
            "{{PROMPT}}".to_string(),
            "--context".to_string(),
            "{{CONTEXT_JSON}}".to_string(),
        ],
    )
    .with_health_check_args(vec![
        script_path.to_string_lossy().into_owned(),
        "custom".to_string(),
        "--ping".to_string(),
    ]);

    let custom_provider = AgentProvider::Custom(custom_cfg);
    let default_runner = AgentHubRunner::new();
    let connectivity = default_runner
        .self_test_provider(&custom_provider)
        .await
        .expect("custom self-test");
    assert!(connectivity.ok);
    assert!(connectivity.stdout.contains("mock-cli custom ok"));

    let custom_out = default_runner
        .run_advisor(&custom_provider, &payload, None)
        .await
        .expect("custom provider run");
    assert_eq!(custom_out.provider_kind, AgentProviderKind::Custom);
    assert_eq!(custom_out.response.cross_checked_sources.len(), 3);
}

#[tokio::test]
async fn rejects_invalid_agent_responses_end_to_end() {
    let dir = TempDirGuard::new("rejections");
    let node_exe = find_node_exe();
    let script_path = write_mock_node_script(&dir, "mock-agent.js");
    let payload = sample_payload();

    let cases = [
        ("bad_lt3_urls", "insufficient"),
        ("bad_dup_urls", "duplicate"),
        ("bad_invalid_url", "invalid_url"),
        ("bad_missing_verdict", "missing_verdict"),
        ("bad_missing_risk", "missing_risk"),
        ("bad_range_risk", "range_risk"),
    ];

    for (mode, label) in cases {
        let mut resolver = OverrideCommandResolver::new(false);
        resolver.insert(
            "codex",
            ResolvedCommand::with_prefix(
                &node_exe,
                vec![script_path.to_string_lossy().into_owned(), mode.to_string()],
            ),
        );
        let runner = AgentHubRunner::with_resolver(Arc::new(resolver), RunnerConfig::default());
        let err = runner
            .run_advisor(&AgentProvider::Codex, &payload, None)
            .await
            .expect_err(&format!("expected validation rejection for {label}"));

        match (mode, err) {
            (
                "bad_lt3_urls",
                AgentHubError::Validation(ValidationError::InsufficientSources {
                    found: 2,
                    required: 3,
                }),
            ) => {}
            (
                "bad_dup_urls",
                AgentHubError::Validation(ValidationError::DuplicateSourceUrl { .. }),
            ) => {}
            (
                "bad_invalid_url",
                AgentHubError::Validation(ValidationError::InvalidSourceUrl { .. }),
            ) => {}
            (
                "bad_missing_verdict",
                AgentHubError::Validation(ValidationError::MissingVerdict),
            ) => {}
            (
                "bad_missing_risk",
                AgentHubError::Validation(ValidationError::MissingRiskScore),
            ) => {}
            (
                "bad_range_risk",
                AgentHubError::Validation(ValidationError::RiskScoreOutOfRange(142)),
            ) => {}
            (m, other) => panic!("unexpected error for mode {m}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn handles_subprocess_timeout_and_nonzero_exit_and_cleans_temp_file() {
    let dir = TempDirGuard::new("timeout-and-exit");
    let node_exe = find_node_exe();
    let script_path = write_mock_node_script(&dir, "mock-agent.js");
    let payload = sample_payload();

    // Timeout case with Antigravity (must kill child and clean temp prompt file).
    let mut timeout_resolver = OverrideCommandResolver::new(false);
    timeout_resolver.insert(
        "antigravity",
        ResolvedCommand::with_prefix(
            &node_exe,
            vec![
                script_path.to_string_lossy().into_owned(),
                "sleep".to_string(),
            ],
        ),
    );
    let timeout_runner = AgentHubRunner::with_resolver(
        Arc::new(timeout_resolver),
        RunnerConfig {
            timeout: Duration::from_millis(200),
            self_test_timeout: Duration::from_secs(5),
        },
    );
    let err = timeout_runner
        .run_advisor(&AgentProvider::Antigravity, &payload, None)
        .await
        .expect_err("expected timeout");
    assert!(matches!(err, AgentHubError::Timeout { .. }));

    // Non-zero exit case.
    let mut exit_resolver = OverrideCommandResolver::new(false);
    exit_resolver.insert(
        "codex",
        ResolvedCommand::with_prefix(
            &node_exe,
            vec![
                script_path.to_string_lossy().into_owned(),
                "fail_exit".to_string(),
            ],
        ),
    );
    let exit_runner = AgentHubRunner::with_resolver(Arc::new(exit_resolver), RunnerConfig::default());
    let err = exit_runner
        .run_advisor(&AgentProvider::Codex, &payload, None)
        .await
        .expect_err("expected process failure");
    assert!(matches!(
        err,
        AgentHubError::ProcessFailed {
            exit_code: Some(7),
            ..
        }
    ));
}

#[tokio::test]
async fn executes_windows_cmd_batch_shim_via_runner() {
    let dir = TempDirGuard::new("cmd-shim");
    let cmd_path = dir.path.join("mock-opencode.cmd");
    let json_path = dir.path.join("response.json");
    let valid_json = serde_json::json!({
        "verdict": "CLEAN_VIA_OFFICIAL_TOOL",
        "risk_score": 18,
        "reclaimable_bytes": 1048576,
        "usage_value_assessment": "Component store superseded updates cleanable via DISM.",
        "cross_checked_sources": [
            { "title": "Microsoft Learn DISM", "url": "https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder", "source_type": "A", "finding": "Use DISM /StartComponentCleanup." },
            { "title": "WinSxS Overview", "url": "https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/manage-the-component-store", "source_type": "B", "finding": "Direct deletion breaks servicing stack." },
            { "title": "SuperUser WinSxS", "url": "https://superuser.com/questions/725314/cleanup-winsxs", "source_type": "C", "finding": "DISM is safe; manual delete corrupts updates." }
        ],
        "recommended_action": "Run DISM cleanup or move orphaned logs via 7-Day Quarantine Vault."
    })
    .to_string();
    fs::write(&json_path, valid_json).expect("write json");
    let cmd_script = format!("@echo off\r\ntype \"{}\"\r\n", json_path.display());
    fs::write(&cmd_path, cmd_script).expect("write cmd");

    let custom = AgentProvider::Custom(CustomCliConfig::new(
        cmd_path.to_string_lossy().into_owned(),
        vec!["{{PROMPT}}".to_string()],
    ));
    let runner = AgentHubRunner::new();
    let result = runner
        .run_advisor(&custom, &sample_payload(), None)
        .await
        .expect("cmd shim run");
    assert_eq!(result.response.verdict, AdvisorVerdict::CleanViaOfficialTool);
}

#[tokio::test]
async fn self_tests_discovered_host_providers_when_installed() {
    use windirstat_liquid_glass::agent_hub::discover_available_providers;

    let discovered = discover_available_providers();
    let runner = AgentHubRunner::new();

    for item in discovered {
        if matches!(
            item.kind,
            AgentProviderKind::Kiro
                | AgentProviderKind::Antigravity
                | AgentProviderKind::Codex
                | AgentProviderKind::OpenCode
        ) {
            let provider = match item.kind {
                AgentProviderKind::Kiro => AgentProvider::Kiro,
                AgentProviderKind::Antigravity => AgentProvider::Antigravity,
                AgentProviderKind::Codex => AgentProvider::Codex,
                AgentProviderKind::OpenCode => AgentProvider::OpenCode,
                _ => continue,
            };
            let report = runner
                .self_test_provider(&provider)
                .await
                .unwrap_or_else(|e| panic!("host self-test failed for {:?}: {e}", item.kind));
            assert!(report.ok);
            assert!(!report.stdout.trim().is_empty());
        }
    }
}

#[tokio::test]
async fn executes_kiro_cli_acp_stream_json_via_runner() {
    use windirstat_liquid_glass::agent_hub::InvocationInput;

    let dir = TempDirGuard::new("kiro-cli-acp");
    let node_exe = find_node_exe();
    let script_path = write_mock_node_script(&dir, "mock-agent.js");
    let payload = sample_payload();

    let adapted_args = AgentProvider::Kiro
        .adapt_args_for_resolved_binary(
            Path::new("kiro-cli.exe"),
            &InvocationInput {
                prompt: "{{PROMPT}}",
                context_json: "{{CONTEXT_JSON}}",
                prompt_file_path: None,
            },
        )
        .expect("kiro-cli adapted args");

    let mut template_args = vec![
        script_path.to_string_lossy().into_owned(),
        "kiro-cli".to_string(),
    ];
    template_args.extend(adapted_args);

    let kiro_acp = AgentProvider::Custom(CustomCliConfig::new(
        node_exe.to_string_lossy().into_owned(),
        template_args,
    ));
    let runner = AgentHubRunner::new();
    let output = runner
        .run_advisor(&kiro_acp, &payload, None)
        .await
        .expect("kiro-cli ACP run");
    assert_eq!(output.response.verdict, AdvisorVerdict::SafeToDelete);
    assert_eq!(output.response.cross_checked_sources.len(), 3);
}

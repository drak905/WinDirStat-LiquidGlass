# WinDirStat Liquid Glass QA Fixtures

## 1. Synthetic 100k-Node Disk Tree (`generate_synthetic_tree.mjs`)

Deterministic Windows filesystem tree generator compatible with WinDirStat `/SaveTo <file.json>` (`windirstat/CsvLoader.cpp`) and nested frontend WebGL treemap benchmarks.

```powershell
# Verify determinism, parent-before-child order, cumulative rollups, and 100k node accounting
node tests/fixtures/generate_synthetic_tree.mjs --verify

# Materialize the full 100,000-node /SaveTo JSON fixture (gitignored)
node tests/fixtures/generate_synthetic_tree.mjs --count 100000 --out tests/fixtures/synthetic-tree-100k.json

# Generate nested tree JSON for frontend benchmarks
node tests/fixtures/generate_synthetic_tree.mjs --count 100000 --format nested --out tests/fixtures/synthetic-tree-100k-nested.json
```

A compact 250-node sample (`synthetic-tree-sample.json`) is committed for lightweight unit tests.

## 2. Mock Agent CLIs (`mock_agent_clis/`)

Cross-platform `.cmd`, `.ps1`, and POSIX shell wrappers in `tests/fixtures/mock_agent_clis/bin/` emulate:
- `antigravity`
- `kiro`
- `claude`
- `codex`
- `opencode`

Environment & CLI mode flags:
- `WDS_MOCK_AGENT_MODE=valid` (default): emits structured evaluation JSON with `>= 3` external source URLs.
- `WDS_MOCK_AGENT_MODE=insufficient_sources`: emits `< 3` URLs (`2` URLs) to test strict rejection gates.
- `WDS_MOCK_AGENT_MODE=zero_sources`: emits `0` URLs (`cross_checked_sources: []`).
- `WDS_MOCK_AGENT_MODE=missing_fields`: omits required keys and sets out-of-range `risk_score`.
- `WDS_MOCK_AGENT_MODE=malformed_json`: emits truncated invalid JSON.
- `WDS_MOCK_AGENT_MODE=stream` or `--stream`: emits NDJSON chunks + final payload.

Run self-verification:
```powershell
node tests/fixtures/mock_agent_clis/mock_agent_responder.mjs --self-test
```

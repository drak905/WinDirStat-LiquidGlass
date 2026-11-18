$worktrees = @(
    @{ Branch = "feat/core-ipc-bridge";    Path = "../wds-wt-core";  Role = "Core C++23 & Rust IPC Bridge Subagent" },
    @{ Branch = "feat/liquid-glass-ui";    Path = "../wds-wt-ui";    Role = "Apple Liquid Glass UI & WebGL Treemap Subagent" },
    @{ Branch = "feat/agent-cli-router";   Path = "../wds-wt-agent"; Role = "Pluggable Agent CLI & Internet Cross-Check Subagent" },
    @{ Branch = "feat/7day-rollback-vault";Path = "../wds-wt-vault"; Role = "7-Day Quarantine Backup & Rollback Subagent" },
    @{ Branch = "feat/tdd-ci-suite";       Path = "../wds-wt-qa";    Role = "4-Tier TDD & E2E Verification Subagent" }
)

foreach ($wt in $worktrees) {
    git worktree add -b $wt.Branch $wt.Path main
    Write-Host "[Worktree Ready] $($wt.Role) -> $($wt.Path)" -ForegroundColor Cyan
}

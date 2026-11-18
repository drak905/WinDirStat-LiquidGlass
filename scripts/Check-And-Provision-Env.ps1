#Requires -Version 5.1
$ErrorActionPreference = "Stop"

Write-Host "=== [Phase 0] Auditing Windows Build & Agent CLI Environment ===" -ForegroundColor Cyan

function Refresh-SessionPath {
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" +
                [System.Environment]::GetEnvironmentVariable("Path","User")
}

# 1. Verify or Bootstrap WinGet
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Host "[!] WinGet missing. Bootstrapping Microsoft.DesktopAppInstaller..." -ForegroundColor Yellow
    $bundleUrl = "https://aka.ms/getwinget"
    $bundlePath = "$env:TEMP\Microsoft.DesktopAppInstaller.msixbundle"
    Invoke-WebRequest -Uri $bundleUrl -OutFile $bundlePath
    Add-AppxPackage -Path $bundlePath
    Refresh-SessionPath
}

# 2. Check and Auto-Install Core Development Tools
$packages = @(
    @{ Cmd = "git";      Id = "Git.Git";                          Name = "Git for Windows" },
    @{ Cmd = "gh";       Id = "GitHub.cli";                       Name = "GitHub CLI" },
    @{ Cmd = "rustup";   Id = "Rustlang.Rustup";                  Name = "Rust Toolchain" },
    @{ Cmd = "node";     Id = "OpenJS.NodeJS.LTS";                Name = "Node.js LTS" },
    @{ Cmd = "sqlite3";  Id = "SQLite.SQLite";                    Name = "SQLite3" },
    @{ Cmd = "gitleaks"; Id = "Gitleaks.Gitleaks";                Name = "Gitleaks Secret Scanner" }
)

foreach ($pkg in $packages) {
    if (-not (Get-Command $pkg.Cmd -ErrorAction SilentlyContinue)) {
        Write-Host "[+] Installing missing tool: $($pkg.Name) ($($pkg.Id))..." -ForegroundColor Yellow
        winget install --id $pkg.Id -e --accept-source-agreements --accept-package-agreements --silent
        Refresh-SessionPath
    } else {
        Write-Host "[OK] $($pkg.Name) detected." -ForegroundColor Green
    }
}

# 3. Ensure Rust stable MSVC target is active
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    rustup default stable-x86_64-pc-windows-msvc
}

# 4. Check and Auto-Install Visual Studio C++ Build Tools (C++23, ATL, Windows SDK for WinDirStat)
$vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasCppTools = $false
if (Test-Path $vsWhere) {
    $vcInstall = & $vsWhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vcInstall) { $hasCppTools = $true }
}

if (-not $hasCppTools) {
    Write-Host "[+] Installing Visual Studio C++ Build Tools (MSVC, ATL, Windows 11 SDK)..." -ForegroundColor Yellow
    winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements `
        --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.ATL --includeRecommended"
    Refresh-SessionPath
} else {
    Write-Host "[OK] MSVC C++ Build Tools & ATL detected." -ForegroundColor Green
}

# 5. Ensure pnpm, Vitest, and Playwright CLI are installed
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    npm install -g pnpm @playwright/cli
    Refresh-SessionPath
}

# 6. Audit Installed Local Agent CLIs (Pluggable Backend Discovery)
$supportedAgentClis = @(
    @{ Name = "Antigravity CLI"; Cmd = "antigravity";InstallHint = "Check Google Antigravity CLI PATH registration" },
    @{ Name = "Kiro CLI";        Cmd = "kiro";       InstallHint = "Check Kiro CLI binary in %LOCALAPPDATA%\Programs\Kiro" },
    @{ Name = "Claude Code CLI"; Cmd = "claude";     InstallHint = "npm install -g @anthropic-ai/claude-code" },
    @{ Name = "OpenAI Codex CLI";Cmd = "codex";      InstallHint = "npm install -g @openai/codex" },
    @{ Name = "OpenCode CLI";    Cmd = "opencode";   InstallHint = "npm install -g opencode-ai" }
)

$detectedAgents = @()
foreach ($agent in $supportedAgentClis) {
    if (Get-Command $agent.Cmd -ErrorAction SilentlyContinue) {
        $binPath = (Get-Command $agent.Cmd).Source
        Write-Host "[OK] Agent CLI available: $($agent.Name) -> $binPath" -ForegroundColor Green
        $detectedAgents += @{ name = $agent.Name; command = $agent.Cmd; path = $binPath }
    } else {
        Write-Host "[i] Optional Agent CLI not currently on PATH: $($agent.Name) (Hint: $($agent.InstallHint))" -ForegroundColor DarkGray
    }
}

if ($detectedAgents.Count -eq 0) {
    Write-Host "[+] No Agent CLI found on PATH. Auto-installing OpenCode CLI & Codex CLI as baseline adapters..." -ForegroundColor Yellow
    npm install -g @openai/codex opencode-ai
    Refresh-SessionPath
}

$detectedAgents | ConvertTo-Json -Depth 3 | Set-Content -Path "$PSScriptRoot\detected-agent-clis.json" -Encoding UTF8
Write-Host "=== [Phase 0 Complete] Environment verified and ready ===" -ForegroundColor Cyan

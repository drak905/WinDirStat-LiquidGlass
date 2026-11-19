#Requires -Version 5.1
param(
    [string] $Version = "1.0.0",
    [string] $OutputDir = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ReleaseDir = Join-Path $RepoRoot "src-tauri\target\release"
$StageDir = Join-Path $RepoRoot "build\stage-desktop"
$IconFile = Join-Path $RepoRoot "src-tauri\icons\icon.ico"
$IssFile = Join-Path $RepoRoot "setup\inno\WinDirStat-LiquidGlass.iss"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path (Split-Path $RepoRoot -Parent) "release"
}

$mainExe = Join-Path $ReleaseDir "WinDirStat-LiquidGlass.exe"
$vaultExe = Join-Path $ReleaseDir "wds-vault.exe"
if (-not (Test-Path -LiteralPath $mainExe)) {
    throw "Missing native desktop binary: $mainExe"
}
if (-not (Test-Path -LiteralPath $vaultExe)) {
    throw "Missing vault CLI binary: $vaultExe"
}

$isccCandidates = @(
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
    (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
)
$iscc = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $iscc) {
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { $iscc = $cmd.Source }
}
if (-not $iscc) {
    throw "Inno Setup 6 (ISCC.exe) not found."
}

if (Test-Path -LiteralPath $StageDir) {
    Remove-Item -LiteralPath $StageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $StageDir -Force | Out-Null
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null

Copy-Item -LiteralPath $mainExe -Destination $StageDir -Force
Copy-Item -LiteralPath $vaultExe -Destination $StageDir -Force
Get-ChildItem -Path $ReleaseDir -Filter "*.dll" -File -ErrorAction SilentlyContinue | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $StageDir -Force
}

Write-Host "[Stage] Contents of $StageDir" -ForegroundColor Cyan
Get-ChildItem -Path $StageDir | Format-Table Name, Length

Write-Host "[Inno] Compiling setup installer via $iscc..." -ForegroundColor Cyan
& $iscc `
    "/DAppVersion=$Version" `
    "/DSourceDir=$StageDir" `
    "/DOutputDir=$OutputDir" `
    "/DIconFile=$IconFile" `
    $IssFile

if ($LASTEXITCODE -ne 0) {
    throw "ISCC.exe failed with exit code $LASTEXITCODE"
}

$installerPath = Join-Path $OutputDir "WinDirStat-LiquidGlass-$Version-setup.exe"
if (-not (Test-Path -LiteralPath $installerPath)) {
    throw "Expected installer not found at $installerPath"
}

Write-Host "[OK] Installer built: $installerPath" -ForegroundColor Green
Get-Item -LiteralPath $installerPath | Select-Object FullName, Length

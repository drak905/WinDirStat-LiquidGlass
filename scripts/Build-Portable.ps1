#Requires -Version 5.1
param(
    [string] $Version = "1.0.0",
    [string] $OutputDir = "",
    [switch] $BuildFrontend
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$WorkspaceRoot = Split-Path $RepoRoot -Parent
$SrcTauriDir = Join-Path $RepoRoot "src-tauri"
$SrcUiDir = Join-Path $RepoRoot "src-ui"
$DistIndex = Join-Path $SrcUiDir "dist\index.html"
$ReleaseDir = Join-Path $SrcTauriDir "target\release"

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $WorkspaceRoot "release"
}

# Ensure Cargo, WinLibs/GCC, and npm/pnpm bins are visible cleanly.
$machinePath = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
$userPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
$mergedPaths = @($machinePath, $userPath, $env:PATH) -join ";" -split ";" |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
    Select-Object -Unique
$env:PATH = $mergedPaths -join ";"

$extraPaths = @(
    (Join-Path $env:USERPROFILE ".cargo\bin"),
    (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links"),
    (Join-Path $env:APPDATA "npm")
)
foreach ($candidate in $extraPaths) {
    if ((Test-Path -LiteralPath $candidate) -and (($env:PATH -split ";") -notcontains $candidate)) {
        $env:PATH = "$candidate;$env:PATH"
    }
}

function Get-PeImportedDlls {
    param([string] $ExePath)

    $bytes = [System.IO.File]::ReadAllBytes($ExePath)
    $peOffset = [System.BitConverter]::ToInt32($bytes, 0x3C)
    $sig = [System.BitConverter]::ToUInt32($bytes, $peOffset)
    if ($sig -ne 0x00004550) {
        throw "Invalid PE header signature in $ExePath"
    }

    $coffOffset = $peOffset + 4
    $numSections = [System.BitConverter]::ToUInt16($bytes, $coffOffset + 2)
    $optHeaderSize = [System.BitConverter]::ToUInt16($bytes, $coffOffset + 16)
    $optHeaderOffset = $coffOffset + 20
    $magic = [System.BitConverter]::ToUInt16($bytes, $optHeaderOffset)

    # DataDirectory[1] (Import Table) RVA offset differs between PE32 (0x10b) and PE32+ (0x20b)
    if ($magic -eq 0x20B) {
        $importRva = [System.BitConverter]::ToUInt32($bytes, $optHeaderOffset + 120)
    }
    elseif ($magic -eq 0x10B) {
        $importRva = [System.BitConverter]::ToUInt32($bytes, $optHeaderOffset + 104)
    }
    else {
        throw "Unsupported PE OptionalHeader magic 0x$($magic.ToString('X')) in $ExePath"
    }

    if ($importRva -eq 0) {
        return @()
    }

    $sections = @()
    $sectionOffset = $optHeaderOffset + $optHeaderSize
    for ($i = 0; $i -lt $numSections; $i++) {
        $o = $sectionOffset + ($i * 40)
        $virtualSize = [System.BitConverter]::ToUInt32($bytes, $o + 8)
        $virtualAddr = [System.BitConverter]::ToUInt32($bytes, $o + 12)
        $rawSize = [System.BitConverter]::ToUInt32($bytes, $o + 16)
        $rawPtr = [System.BitConverter]::ToUInt32($bytes, $o + 20)
        $span = [Math]::Max($virtualSize, $rawSize)
        $sections += [PSCustomObject]@{
            VirtualAddress = $virtualAddr
            Span           = $span
            RawPointer     = $rawPtr
        }
    }

    $resolveRva = {
        param([uint32] $Rva)
        foreach ($sec in $sections) {
            if ($Rva -ge $sec.VirtualAddress -and $Rva -lt ($sec.VirtualAddress + $sec.Span)) {
                return [int]($sec.RawPointer + ($Rva - $sec.VirtualAddress))
            }
        }
        return -1
    }

    $impOff = & $resolveRva $importRva
    if ($impOff -lt 0) {
        throw "Failed to map PE Import Table RVA 0x$($importRva.ToString('X'))"
    }

    $dlls = New-Object System.Collections.Generic.List[string]
    while ($true) {
        $origFirstThunk = [System.BitConverter]::ToUInt32($bytes, $impOff)
        $nameRva = [System.BitConverter]::ToUInt32($bytes, $impOff + 12)
        $firstThunk = [System.BitConverter]::ToUInt32($bytes, $impOff + 16)
        if ($origFirstThunk -eq 0 -and $nameRva -eq 0 -and $firstThunk -eq 0) {
            break
        }
        $nameOff = & $resolveRva $nameRva
        if ($nameOff -ge 0) {
            $end = $nameOff
            while ($bytes[$end] -ne 0) { $end++ }
            $dllName = [System.Text.Encoding]::ASCII.GetString($bytes, $nameOff, $end - $nameOff)
            $dlls.Add($dllName)
        }
        $impOff += 20
    }

    return $dlls.ToArray()
}

if ($BuildFrontend -or -not (Test-Path -LiteralPath $DistIndex)) {
    Write-Host "[Frontend] Building Vite bundle..." -ForegroundColor Cyan
    Push-Location $SrcUiDir
    try {
        $pnpm = Get-Command pnpm -ErrorAction SilentlyContinue
        if ($pnpm) {
            & pnpm build
        }
        else {
            & npm run build
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Frontend build failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

Write-Host "[Cargo] Building standalone release binary WinDirStat-LiquidGlass..." -ForegroundColor Cyan
Push-Location $SrcTauriDir
try {
    & cargo build --release --bin WinDirStat-LiquidGlass
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release --bin WinDirStat-LiquidGlass failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$builtExe = Join-Path $ReleaseDir "WinDirStat-LiquidGlass.exe"
if (-not (Test-Path -LiteralPath $builtExe)) {
    throw "Expected compiled binary not found at $builtExe"
}

Write-Host "[PE] Inspecting Import Table for zero external/sidecar DLLs..." -ForegroundColor Cyan
$importedDlls = Get-PeImportedDlls -ExePath $builtExe
Write-Host ("  Imported DLLs ({0}): {1}" -f $importedDlls.Count, ($importedDlls -join ", ")) -ForegroundColor DarkGray

$forbiddenPatterns = @(
    "^WebView2Loader\.dll$",
    "^libgcc",
    "^libstdc\+\+",
    "^libwinpthread",
    "^vcruntime",
    "^msvcp"
)
foreach ($dll in $importedDlls) {
    foreach ($pattern in $forbiddenPatterns) {
        if ($dll -imatch $pattern) {
            throw "Forbidden external DLL dependency found in PE Import Table: $dll (matched $pattern)"
        }
    }
}
Write-Host "[PE] Verified zero WebView2Loader.dll or third-party runtime DLL imports." -ForegroundColor Green

Write-Host "[Smoke] Testing binary from an isolated empty directory (zero sidecar DLLs)..." -ForegroundColor Cyan
$isolatedDir = Join-Path $env:TEMP ("wds-portable-verify-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $isolatedDir -Force | Out-Null
try {
    $isolatedExe = Join-Path $isolatedDir "WinDirStat-LiquidGlass.exe"
    Copy-Item -LiteralPath $builtExe -Destination $isolatedExe -Force

    $selfCheckOut = (& $isolatedExe --self-check 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "--self-check failed (exit $LASTEXITCODE): $selfCheckOut"
    }
    Write-Host "  $selfCheckOut" -ForegroundColor DarkGray
    if ($selfCheckOut -notmatch "custom-protocol=true" -or $selfCheckOut -notmatch "embedded-webview2-loader=true") {
        throw "--self-check output missing expected flags: $selfCheckOut"
    }

    $wv2CheckOut = (& $isolatedExe --portable --self-check-webview2 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "--portable --self-check-webview2 failed (exit $LASTEXITCODE): $wv2CheckOut"
    }
    Write-Host "  $wv2CheckOut" -ForegroundColor DarkGray
    if ($wv2CheckOut -notmatch "WebView2 self-check OK") {
        throw "--portable --self-check-webview2 output missing success marker: $wv2CheckOut"
    }

    $materializedRuntimeDir = Join-Path $isolatedDir "WinDirStat-LiquidGlass-Data\runtime"
    $materializedLoaders = @(Get-ChildItem -Path $materializedRuntimeDir -Filter "WebView2Loader-*.dll" -File -ErrorAction SilentlyContinue)
    if ($materializedLoaders.Count -lt 1) {
        throw "Expected materialized embedded loader in $materializedRuntimeDir, found none."
    }
    Write-Host ("  Materialized loader: {0} ({1} bytes)" -f $materializedLoaders[0].FullName, $materializedLoaders[0].Length) -ForegroundColor Green
}
finally {
    if (Test-Path -LiteralPath $isolatedDir) {
        Remove-Item -LiteralPath $isolatedDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$portableFileName = "WinDirStat-LiquidGlass-$Version-portable.exe"
$releasePortablePath = Join-Path $OutputDir $portableFileName
$rootPortablePath = Join-Path $WorkspaceRoot $portableFileName

Copy-Item -LiteralPath $builtExe -Destination $releasePortablePath -Force
Copy-Item -LiteralPath $builtExe -Destination $rootPortablePath -Force

$sha256 = (Get-FileHash -LiteralPath $releasePortablePath -Algorithm SHA256).Hash
$item = Get-Item -LiteralPath $releasePortablePath

Write-Host ""
Write-Host "[OK] Portable single-file executable built and verified:" -ForegroundColor Green
Write-Host "  Release Path   : $releasePortablePath"
Write-Host "  Workspace Copy : $rootPortablePath"
Write-Host ("  Size (bytes)   : {0:N0}" -f $item.Length)
Write-Host "  SHA-256        : $sha256"

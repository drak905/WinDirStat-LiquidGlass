@ECHO OFF
SETLOCAL
TITLE WinDirStat Liquid Glass
SET "ROOT=%~dp0"
SET "ROOT=%ROOT:~0,-1%"

ECHO ============================================================
ECHO  WinDirStat Liquid Glass - Interactive Web UI & Vault CLI
ECHO ============================================================
ECHO.

IF EXIST "%ROOT%\src-tauri\target\release\wds-vault.exe" (
    ECHO [Vault CLI] "%ROOT%\src-tauri\target\release\wds-vault.exe"
) ELSE (
    ECHO [Vault CLI] Not yet built. Build with:
    ECHO   cd /d "%ROOT%\src-tauri" ^&^& cargo build --release --bin wds-vault
)
ECHO.

WHERE pnpm >NUL 2>&1
IF %ERRORLEVEL% NEQ 0 (
    ECHO [ERROR] pnpm was not found on PATH. Run scripts\Check-And-Provision-Env.ps1 first.
    PAUSE
    EXIT /B 1
)

IF NOT EXIST "%ROOT%\src-ui\node_modules" (
    ECHO [Setup] Installing UI dependencies via pnpm...
    CALL pnpm --dir "%ROOT%\src-ui" install
    IF %ERRORLEVEL% NEQ 0 EXIT /B %ERRORLEVEL%
)

ECHO [Launch] Starting WinDirStat Liquid Glass UI at http://localhost:1420 ...
CALL pnpm --dir "%ROOT%\src-ui" dev --open

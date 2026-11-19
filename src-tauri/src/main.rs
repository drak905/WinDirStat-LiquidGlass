#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use windirstat_liquid_glass::desktop_api::{
    default_app_data_dir, detect_ui_providers, find_real_duplicates, list_real_vault_items,
    quarantine_real_item, query_windows_drives, rollback_real_vault_item, scan_directory_snapshot,
    try_run_real_advisor, AppContext, UiAgentProviderStatus, UiDriveInfo, UiDuplicateGroup,
    UiScanSnapshot, UiVaultItem,
};
use windirstat_liquid_glass::vault::service::start_expiration_worker;

struct SharedDesktopState {
    ctx: Arc<AppContext>,
}

#[cfg(windows)]
struct WebView2IsolationGuard {
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
    temp_data_dir: Option<PathBuf>,
}

#[cfg(windows)]
impl Drop for WebView2IsolationGuard {
    fn drop(&mut self) {
        if !self.mutex_handle.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.mutex_handle);
            }
        }
        if let Some(dir) = &self.temp_data_dir {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

#[cfg(windows)]
fn is_default_webview2_locked() -> bool {
    use std::os::windows::fs::OpenOptionsExt;

    let Some(local) = env::var_os("LOCALAPPDATA") else {
        return false;
    };
    let lockfile = PathBuf::from(local)
        .join("org.windirstat.liquidglass")
        .join("EBWebView")
        .join("lockfile");
    if !lockfile.exists() {
        return false;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&lockfile)
        .is_err()
}

#[cfg(windows)]
fn prepare_webview2_isolation() -> WebView2IsolationGuard {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let mutex_name: Vec<u16> = "Local\\WinDirStatLiquidGlass_PrimaryWebView2\0"
        .encode_utf16()
        .collect();
    let mutex_handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    let mutex_taken =
        !mutex_handle.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    let mut temp_data_dir = None;
    if env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none()
        && (mutex_taken || is_default_webview2_locked())
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let session_dir = PathBuf::from(local)
                .join("org.windirstat.liquidglass")
                .join(format!("session-{}", std::process::id()));
            let _ = fs::create_dir_all(&session_dir);
            env::set_var("WEBVIEW2_USER_DATA_FOLDER", &session_dir);
            temp_data_dir = Some(session_dir);
        }
    }

    WebView2IsolationGuard {
        mutex_handle,
        temp_data_dir,
    }
}

fn run_embedded_self_check() -> Result<(), String> {
    if !cfg!(feature = "custom-protocol") {
        return Err("custom-protocol feature is disabled; offline UI assets are not embedded".into());
    }

    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let html_bytes = context
        .assets()
        .get(&"index.html".into())
        .ok_or_else(|| "embedded index.html asset missing from tauri context".to_string())?;
    let html = String::from_utf8_lossy(&html_bytes);

    if !html.contains("WinDirStat Liquid Glass") {
        return Err("embedded index.html missing expected title".into());
    }
    if !html.contains("/assets/index-") {
        return Err("embedded index.html missing compiled Vite asset bundle reference".into());
    }

    println!(
        "WinDirStat-LiquidGlass self-check OK: custom-protocol=true, index.html={} bytes",
        html_bytes.len()
    );
    Ok(())
}

#[tauri::command]
async fn list_drives() -> Vec<UiDriveInfo> {
    tauri::async_runtime::spawn_blocking(query_windows_drives)
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn scan_directory(target: String) -> Option<UiScanSnapshot> {
    tauri::async_runtime::spawn_blocking(move || scan_directory_snapshot(&target))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
async fn find_duplicates(target: String) -> Option<Vec<UiDuplicateGroup>> {
    tauri::async_runtime::spawn_blocking(move || find_real_duplicates(&target))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
async fn detect_agent_clis() -> Vec<UiAgentProviderStatus> {
    tauri::async_runtime::spawn_blocking(detect_ui_providers)
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn ask_agent_advisor(
    provider: String,
    prompt: String,
    target_path: Option<String>,
    target_name: Option<String>,
    target_size_bytes: Option<u64>,
    custom_command: Option<String>,
) -> Option<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        try_run_real_advisor(
            &provider,
            &prompt,
            target_path.as_deref(),
            target_name.as_deref(),
            target_size_bytes,
            custom_command.as_deref(),
        )
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
async fn quarantine_item(
    state: State<'_, SharedDesktopState>,
    path: String,
    _name: Option<String>,
    _size_bytes: Option<u64>,
    _batch_id: Option<String>,
) -> Result<Option<UiVaultItem>, String> {
    let ctx = Arc::clone(&state.ctx);
    Ok(tauri::async_runtime::spawn_blocking(move || quarantine_real_item(&ctx, &path))
        .await
        .ok()
        .flatten())
}

#[tauri::command]
async fn list_vault_items(
    state: State<'_, SharedDesktopState>,
) -> Result<Option<Vec<UiVaultItem>>, String> {
    let ctx = Arc::clone(&state.ctx);
    Ok(tauri::async_runtime::spawn_blocking(move || list_real_vault_items(&ctx))
        .await
        .ok()
        .flatten())
}

#[tauri::command]
async fn rollback_vault_item(
    state: State<'_, SharedDesktopState>,
    id: String,
) -> Result<Option<UiVaultItem>, String> {
    let ctx = Arc::clone(&state.ctx);
    Ok(tauri::async_runtime::spawn_blocking(move || rollback_real_vault_item(&ctx, &id))
        .await
        .ok()
        .flatten())
}

#[tauri::command]
async fn purge_vault_item(_id: String) -> Option<()> {
    None
}

fn main() {
    if env::args().any(|arg| arg == "--self-check") {
        if let Err(err) = run_embedded_self_check() {
            eprintln!("Self-check FAILED: {err}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(windows)]
    let _wv2_guard = prepare_webview2_isolation();

    let app_dir = default_app_data_dir();
    let _ = fs::create_dir_all(&app_dir);

    let ctx = Arc::new(AppContext {
        vault_db_path: app_dir.join("vault_manifest.db"),
    });

    let _expiration_worker = ctx
        .open_vault()
        .ok()
        .map(|svc| start_expiration_worker(Arc::new(svc)));

    tauri::Builder::default()
        .manage(SharedDesktopState {
            ctx: Arc::clone(&ctx),
        })
        .invoke_handler(tauri::generate_handler![
            list_drives,
            scan_directory,
            find_duplicates,
            detect_agent_clis,
            ask_agent_advisor,
            quarantine_item,
            list_vault_items,
            rollback_vault_item,
            purge_vault_item,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WinDirStat Liquid Glass");
}

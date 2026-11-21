#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use windirstat_liquid_glass::desktop_api::{
    detect_ui_providers, find_real_duplicates, list_opencode_models_async, list_real_vault_items,
    quarantine_real_item, query_windows_drives, rollback_real_vault_item, run_real_advisor_async,
    scan_directory_snapshot, test_agent_cli_async, AppContext, UiAgentProviderStatus, UiDriveInfo,
    UiDuplicateGroup, UiScanSnapshot, UiVaultItem,
};
use windirstat_liquid_glass::portable::{
    resolve_runtime_paths, webview2_mutex_name_for_paths, RuntimePaths,
};
use windirstat_liquid_glass::vault::service::start_expiration_worker;

#[cfg(all(windows, not(target_env = "msvc")))]
mod webview2_embedded;

struct SharedDesktopState {
    ctx: Arc<AppContext>,
}

#[cfg(windows)]
struct WebView2IsolationGuard {
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
    temp_data_dir: Option<PathBuf>,
    active_data_dir: PathBuf,
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
fn is_webview2_dir_locked(webview2_data_dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;

    for lockfile in [
        webview2_data_dir.join("EBWebView").join("lockfile"),
        webview2_data_dir.join("lockfile"),
    ] {
        if lockfile.exists()
            && OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&lockfile)
                .is_err()
        {
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn prepare_webview2_isolation(paths: &RuntimePaths) -> WebView2IsolationGuard {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let mutex_str = webview2_mutex_name_for_paths(paths);
    let mutex_name: Vec<u16> = mutex_str
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mutex_handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    let mutex_taken =
        !mutex_handle.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    let mut temp_data_dir = None;
    let active_data_dir = if let Some(existing) =
        env::var_os("WEBVIEW2_USER_DATA_FOLDER").filter(|v| !v.is_empty())
    {
        PathBuf::from(existing)
    } else if mutex_taken || is_webview2_dir_locked(&paths.webview2_data_dir) {
        let session_dir = paths
            .webview2_data_dir
            .join(format!("session-{}", std::process::id()));
        let _ = fs::create_dir_all(&session_dir);
        env::set_var("WEBVIEW2_USER_DATA_FOLDER", &session_dir);
        temp_data_dir = Some(session_dir.clone());
        session_dir
    } else {
        let _ = fs::create_dir_all(&paths.webview2_data_dir);
        env::set_var("WEBVIEW2_USER_DATA_FOLDER", &paths.webview2_data_dir);
        paths.webview2_data_dir.clone()
    };

    WebView2IsolationGuard {
        mutex_handle,
        temp_data_dir,
        active_data_dir,
    }
}

fn run_embedded_self_check(paths: &RuntimePaths) -> Result<(), String> {
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

    #[cfg(all(windows, not(target_env = "msvc")))]
    let loader_info = format!(
        "true ({} bytes, tag={})",
        webview2_embedded::embedded_loader_size(),
        webview2_embedded::embedded_loader_tag()
    );
    #[cfg(any(not(windows), target_env = "msvc"))]
    let loader_info = "static-msvc".to_string();

    println!(
        "WinDirStat-LiquidGlass self-check OK: custom-protocol=true, index.html={} bytes, embedded-webview2-loader={}, portable-mode={} ({}), data-dir={}",
        html_bytes.len(),
        loader_info,
        paths.is_portable,
        paths.mode_source.as_str(),
        paths.app_data_dir.display()
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
    active_drive: Option<String>,
    model: Option<String>,
    scan_summary: Option<String>,
) -> Value {
    run_real_advisor_async(
        &provider,
        &prompt,
        target_path.as_deref(),
        target_name.as_deref(),
        target_size_bytes,
        custom_command.as_deref(),
        active_drive.as_deref(),
        model.as_deref(),
        scan_summary.as_deref(),
    )
    .await
}

#[tauri::command]
async fn list_opencode_models() -> Vec<String> {
    list_opencode_models_async().await
}

#[tauri::command]
async fn test_agent_cli(provider: String, custom_command: Option<String>) -> Value {
    test_agent_cli_async(&provider, custom_command.as_deref()).await
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
    Ok(
        tauri::async_runtime::spawn_blocking(move || quarantine_real_item(&ctx, &path))
            .await
            .ok()
            .flatten(),
    )
}

#[tauri::command]
async fn list_vault_items(
    state: State<'_, SharedDesktopState>,
) -> Result<Option<Vec<UiVaultItem>>, String> {
    let ctx = Arc::clone(&state.ctx);
    Ok(
        tauri::async_runtime::spawn_blocking(move || list_real_vault_items(&ctx))
            .await
            .ok()
            .flatten(),
    )
}

#[tauri::command]
async fn rollback_vault_item(
    state: State<'_, SharedDesktopState>,
    id: String,
) -> Result<Option<UiVaultItem>, String> {
    let ctx = Arc::clone(&state.ctx);
    Ok(
        tauri::async_runtime::spawn_blocking(move || rollback_real_vault_item(&ctx, &id))
            .await
            .ok()
            .flatten(),
    )
}

#[tauri::command]
async fn purge_vault_item(_id: String) -> Option<()> {
    None
}

fn main() {
    let paths = resolve_runtime_paths();

    #[cfg(all(windows, not(target_env = "msvc")))]
    webview2_embedded::configure_runtime_cache_dir(paths.runtime_cache_dir.clone());

    let self_check_wv2 = env::args().any(|arg| arg == "--self-check-webview2");
    if self_check_wv2 || env::args().any(|arg| arg == "--self-check") {
        if let Err(err) = run_embedded_self_check(&paths) {
            eprintln!("Self-check FAILED: {err}");
            std::process::exit(1);
        }
        if self_check_wv2 {
            #[cfg(all(windows, not(target_env = "msvc")))]
            match webview2_embedded::probe_available_webview2_version() {
                Ok((dll_path, version)) => {
                    println!(
                        "WinDirStat-LiquidGlass WebView2 self-check OK: runtime={}, loader={}, portable-mode={}",
                        version,
                        dll_path.display(),
                        paths.is_portable
                    );
                }
                Err(err) => {
                    eprintln!("WebView2 self-check FAILED: {err}");
                    std::process::exit(1);
                }
            }
            #[cfg(any(not(windows), target_env = "msvc"))]
            println!(
                "WinDirStat-LiquidGlass WebView2 self-check OK: runtime=static-msvc, portable-mode={}",
                paths.is_portable
            );
        }
        return;
    }

    #[cfg(windows)]
    let wv2_guard = prepare_webview2_isolation(&paths);

    #[cfg(windows)]
    let setup_webview_dir = wv2_guard.active_data_dir.clone();
    #[cfg(not(windows))]
    let setup_webview_dir = paths.webview2_data_dir.clone();

    let _ = fs::create_dir_all(&paths.app_data_dir);

    let ctx = Arc::new(AppContext {
        vault_db_path: paths.vault_db_path.clone(),
    });

    let _expiration_worker = ctx
        .open_vault()
        .ok()
        .map(|svc| start_expiration_worker(Arc::new(svc)));

    tauri::Builder::default()
        .manage(SharedDesktopState {
            ctx: Arc::clone(&ctx),
        })
        .setup(move |app| {
            let window_cfg = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "missing main window config in tauri.conf.json",
                    )
                })?;

            tauri::WebviewWindowBuilder::from_config(app.handle(), &window_cfg)?
                .data_directory(setup_webview_dir)
                .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_drives,
            scan_directory,
            find_duplicates,
            detect_agent_clis,
            ask_agent_advisor,
            list_opencode_models,
            test_agent_cli,
            quarantine_item,
            list_vault_items,
            rollback_vault_item,
            purge_vault_item,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WinDirStat Liquid Glass");
}

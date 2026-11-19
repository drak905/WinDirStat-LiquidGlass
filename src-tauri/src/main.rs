#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
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

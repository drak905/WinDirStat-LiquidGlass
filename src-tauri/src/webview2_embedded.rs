use std::env;
use std::ffi::c_void;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const E_FAIL: i32 = 0x8000_4005u32 as i32;

static EMBEDDED_DLL_BYTES: &[u8] = include_bytes!(env!("EMBEDDED_WEBVIEW2_LOADER_PATH"));
const EMBEDDED_DLL_TAG: &str = env!("EMBEDDED_WEBVIEW2_LOADER_TAG");

static RUNTIME_CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static LOADED_LOADER: OnceLock<Result<(PathBuf, isize), String>> = OnceLock::new();

pub fn configure_runtime_cache_dir(dir: PathBuf) {
    let _ = RUNTIME_CACHE_DIR.set(dir);
}

pub fn embedded_loader_size() -> usize {
    EMBEDDED_DLL_BYTES.len()
}

pub fn embedded_loader_tag() -> &'static str {
    EMBEDDED_DLL_TAG
}

fn cache_dir_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(configured) = RUNTIME_CACHE_DIR.get() {
        dirs.push(configured.clone());
    }
    dirs.push(env::temp_dir().join("WinDirStatLiquidGlass").join("runtime"));
    dirs
}

fn file_matches_embedded(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && m.len() == EMBEDDED_DLL_BYTES.len() as u64)
        .unwrap_or(false)
        && fs::read(path)
            .map(|bytes| bytes == EMBEDDED_DLL_BYTES)
            .unwrap_or(false)
}

fn materialize_in_dir(dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir)
        .map_err(|err| format!("failed creating runtime cache dir {}: {err}", dir.display()))?;

    let target = dir.join(format!("WebView2Loader-{EMBEDDED_DLL_TAG}.dll"));
    if file_matches_embedded(&target) {
        return Ok(target);
    }

    let tmp = dir.join(format!(
        "WebView2Loader-{EMBEDDED_DLL_TAG}.{}.tmp",
        std::process::id()
    ));
    fs::write(&tmp, EMBEDDED_DLL_BYTES)
        .map_err(|err| format!("failed writing temp loader {}: {err}", tmp.display()))?;

    if target.exists() && file_matches_embedded(&target) {
        let _ = fs::remove_file(&tmp);
        return Ok(target);
    }

    match fs::rename(&tmp, &target) {
        Ok(()) => Ok(target),
        Err(rename_err) => {
            let _ = fs::remove_file(&tmp);
            if file_matches_embedded(&target) {
                Ok(target)
            } else {
                Err(format!(
                    "failed finalizing loader {}: {rename_err}",
                    target.display()
                ))
            }
        }
    }
}

fn materialize_embedded_dll() -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    for dir in cache_dir_candidates() {
        match materialize_in_dir(&dir) {
            Ok(path) => return Ok(path),
            Err(err) => errors.push(err),
        }
    }
    Err(errors.join("; "))
}

fn load_webview2_module() -> Result<(PathBuf, *mut c_void), String> {
    let cached = LOADED_LOADER.get_or_init(|| {
        let path = materialize_embedded_dll()?;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hmodule = unsafe { LoadLibraryW(wide.as_ptr()) };
        if hmodule.is_null() {
            let os_err = std::io::Error::last_os_error();
            return Err(format!(
                "LoadLibraryW failed for {}: {os_err}",
                path.display()
            ));
        }
        Ok((path, hmodule as isize))
    });

    match cached {
        Ok((path, raw_handle)) => Ok((path.clone(), (*raw_handle) as *mut c_void)),
        Err(err) => Err(err.clone()),
    }
}

unsafe fn resolve_proc<F: Copy>(proc_name_with_nul: &[u8]) -> Result<F, i32> {
    let (_path, hmodule) = load_webview2_module().map_err(|_| E_FAIL)?;
    let proc = GetProcAddress(hmodule, proc_name_with_nul.as_ptr());
    match proc {
        Some(func) => Ok(std::mem::transmute_copy(&func)),
        None => Err(E_FAIL),
    }
}

#[no_mangle]
pub unsafe extern "system" fn CompareBrowserVersions(
    version1: *const u16,
    version2: *const u16,
    result: *mut i32,
) -> i32 {
    type FnType = unsafe extern "system" fn(*const u16, *const u16, *mut i32) -> i32;
    match resolve_proc::<FnType>(b"CompareBrowserVersions\0") {
        Ok(func) => func(version1, version2, result),
        Err(hr) => hr,
    }
}

#[no_mangle]
pub unsafe extern "system" fn CreateCoreWebView2Environment(
    environment_created_handler: *mut c_void,
) -> i32 {
    type FnType = unsafe extern "system" fn(*mut c_void) -> i32;
    match resolve_proc::<FnType>(b"CreateCoreWebView2Environment\0") {
        Ok(func) => func(environment_created_handler),
        Err(hr) => hr,
    }
}

#[no_mangle]
pub unsafe extern "system" fn CreateCoreWebView2EnvironmentWithOptions(
    browser_executable_folder: *const u16,
    user_data_folder: *const u16,
    environment_options: *mut c_void,
    environment_created_handler: *mut c_void,
) -> i32 {
    type FnType =
        unsafe extern "system" fn(*const u16, *const u16, *mut c_void, *mut c_void) -> i32;
    match resolve_proc::<FnType>(b"CreateCoreWebView2EnvironmentWithOptions\0") {
        Ok(func) => func(
            browser_executable_folder,
            user_data_folder,
            environment_options,
            environment_created_handler,
        ),
        Err(hr) => hr,
    }
}

#[no_mangle]
pub unsafe extern "system" fn GetAvailableCoreWebView2BrowserVersionString(
    browser_executable_folder: *const u16,
    version_info: *mut *mut u16,
) -> i32 {
    type FnType = unsafe extern "system" fn(*const u16, *mut *mut u16) -> i32;
    match resolve_proc::<FnType>(b"GetAvailableCoreWebView2BrowserVersionString\0") {
        Ok(func) => func(browser_executable_folder, version_info),
        Err(hr) => hr,
    }
}

#[no_mangle]
pub unsafe extern "system" fn GetAvailableCoreWebView2BrowserVersionStringWithOptions(
    browser_executable_folder: *const u16,
    environment_options: *mut c_void,
    version_info: *mut *mut u16,
) -> i32 {
    type FnType = unsafe extern "system" fn(*const u16, *mut c_void, *mut *mut u16) -> i32;
    match resolve_proc::<FnType>(b"GetAvailableCoreWebView2BrowserVersionStringWithOptions\0") {
        Ok(func) => func(browser_executable_folder, environment_options, version_info),
        Err(hr) => hr,
    }
}

pub fn probe_available_webview2_version() -> Result<(PathBuf, String), String> {
    let (dll_path, _) = load_webview2_module()?;
    let mut version_ptr: *mut u16 = std::ptr::null_mut();
    let hr = unsafe {
        GetAvailableCoreWebView2BrowserVersionString(std::ptr::null(), &mut version_ptr)
    };
    if hr < 0 {
        if !version_ptr.is_null() {
            unsafe { CoTaskMemFree(version_ptr.cast()) };
        }
        return Err(format!(
            "GetAvailableCoreWebView2BrowserVersionString failed with HRESULT 0x{:08X}",
            hr as u32
        ));
    }
    if version_ptr.is_null() {
        return Err(
            "Edge WebView2 Runtime not detected (GetAvailableCoreWebView2BrowserVersionString returned null)"
                .to_string(),
        );
    }

    let mut len = 0usize;
    unsafe {
        while *version_ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(version_ptr, len);
        let version = String::from_utf16_lossy(slice);
        CoTaskMemFree(version_ptr.cast());
        Ok((dll_path, version))
    }
}

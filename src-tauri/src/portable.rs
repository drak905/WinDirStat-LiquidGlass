use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PORTABLE_DATA_DIR_NAME: &str = "WinDirStat-LiquidGlass-Data";

static PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortableModeSource {
    CliFlag,
    EnvVar,
    MarkerFile,
    ExistingDataDir,
    RemovableDrive,
    Standard,
}

impl PortableModeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CliFlag => "cli-flag",
            Self::EnvVar => "env-var",
            Self::MarkerFile => "marker-file",
            Self::ExistingDataDir => "existing-data-dir",
            Self::RemovableDrive => "removable-drive",
            Self::Standard => "standard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub is_portable: bool,
    pub mode_source: PortableModeSource,
    pub app_data_dir: PathBuf,
    pub vault_db_path: PathBuf,
    pub webview2_data_dir: PathBuf,
    pub runtime_cache_dir: PathBuf,
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn ensure_dir_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() || !dir.is_dir() {
        return false;
    }
    let seq = PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let probe = dir.join(format!(".wds-write-probe-{}-{seq}", std::process::id()));
    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn is_truthy_env(value: Option<&str>) -> bool {
    value
        .map(|raw| matches!(raw.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn resolve_runtime_paths_with(
    exe_path: Option<&Path>,
    args: &[String],
    portable_env: Option<&str>,
    local_app_data: Option<&Path>,
    temp_dir: &Path,
    is_removable_drive: bool,
) -> RuntimePaths {
    let exe_dir = exe_path
        .and_then(|p| p.parent())
        .filter(|d| !d.as_os_str().is_empty());

    let mode_source = if args.iter().any(|arg| arg == "--portable") {
        PortableModeSource::CliFlag
    } else if is_truthy_env(portable_env) {
        PortableModeSource::EnvVar
    } else if exe_dir
        .map(|dir| dir.join("portable.txt").is_file() || dir.join(".portable").is_file())
        .unwrap_or(false)
    {
        PortableModeSource::MarkerFile
    } else if exe_dir
        .map(|dir| dir.join(PORTABLE_DATA_DIR_NAME).is_dir())
        .unwrap_or(false)
    {
        PortableModeSource::ExistingDataDir
    } else if is_removable_drive {
        PortableModeSource::RemovableDrive
    } else {
        PortableModeSource::Standard
    };

    if mode_source != PortableModeSource::Standard {
        let portable_root = exe_dir
            .map(|dir| dir.join(PORTABLE_DATA_DIR_NAME))
            .filter(|candidate| ensure_dir_writable(candidate))
            .unwrap_or_else(|| temp_dir.join("WinDirStatLiquidGlass-Portable"));

        return RuntimePaths {
            is_portable: true,
            mode_source,
            vault_db_path: portable_root.join("vault_manifest.db"),
            webview2_data_dir: portable_root.join("WebView2"),
            runtime_cache_dir: portable_root.join("runtime"),
            app_data_dir: portable_root,
        };
    }

    let base_local = local_app_data
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| temp_dir.to_path_buf());
    let app_data_dir = base_local.join("WinDirStatLiquidGlass");

    RuntimePaths {
        is_portable: false,
        mode_source: PortableModeSource::Standard,
        vault_db_path: app_data_dir.join("vault_manifest.db"),
        webview2_data_dir: base_local.join("org.windirstat.liquidglass"),
        runtime_cache_dir: app_data_dir.join("runtime"),
        app_data_dir,
    }
}

#[cfg(windows)]
fn detect_exe_on_removable_drive(exe_path: Option<&Path>) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    const DRIVE_REMOVABLE: u32 = 2;

    let Some(path) = exe_path else {
        return false;
    };
    let drive_letter = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter as char),
            _ => None,
        },
        _ => None,
    };
    let Some(letter) = drive_letter else {
        return false;
    };

    let root = format!("{letter}:\\");
    let wide: Vec<u16> = std::ffi::OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe { GetDriveTypeW(wide.as_ptr()) == DRIVE_REMOVABLE }
}

#[cfg(not(windows))]
fn detect_exe_on_removable_drive(_exe_path: Option<&Path>) -> bool {
    false
}

pub fn resolve_runtime_paths() -> RuntimePaths {
    let exe_path = env::current_exe().ok();
    let args: Vec<String> = env::args().collect();
    let portable_env = env::var("WDS_PORTABLE").ok();
    let local_app_data = env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let temp_dir = env::temp_dir();
    let is_removable = detect_exe_on_removable_drive(exe_path.as_deref());

    resolve_runtime_paths_with(
        exe_path.as_deref(),
        &args,
        portable_env.as_deref(),
        local_app_data.as_deref(),
        &temp_dir,
        is_removable,
    )
}

pub fn webview2_mutex_name_for_paths(paths: &RuntimePaths) -> String {
    if !paths.is_portable {
        return "Local\\WinDirStatLiquidGlass_PrimaryWebView2".to_string();
    }
    let normalized = paths.webview2_data_dir.to_string_lossy().to_ascii_lowercase();
    let hash = fnv1a_64(normalized.as_bytes());
    format!("Local\\WinDirStatLiquidGlass_WV2_{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn standard_mode_uses_local_app_data_and_creates_nothing_next_to_exe() {
        let exe_dir = TempDir::new().unwrap();
        let local_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &[],
            None,
            Some(local_dir.path()),
            temp_dir.path(),
            false,
        );

        assert!(!paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::Standard);
        assert_eq!(
            paths.app_data_dir,
            local_dir.path().join("WinDirStatLiquidGlass")
        );
        assert_eq!(
            paths.vault_db_path,
            local_dir
                .path()
                .join("WinDirStatLiquidGlass")
                .join("vault_manifest.db")
        );
        assert_eq!(
            paths.webview2_data_dir,
            local_dir.path().join("org.windirstat.liquidglass")
        );
        assert_eq!(
            paths.runtime_cache_dir,
            local_dir.path().join("WinDirStatLiquidGlass").join("runtime")
        );
        assert!(!exe_dir.path().join(PORTABLE_DATA_DIR_NAME).exists());
        assert_eq!(
            webview2_mutex_name_for_paths(&paths),
            "Local\\WinDirStatLiquidGlass_PrimaryWebView2"
        );
    }

    #[test]
    fn cli_flag_activates_portable_mode() {
        let exe_dir = TempDir::new().unwrap();
        let local_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");
        let args = vec![
            "WinDirStat-LiquidGlass.exe".to_string(),
            "--portable".to_string(),
        ];

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &args,
            None,
            Some(local_dir.path()),
            temp_dir.path(),
            false,
        );

        let expected_root = exe_dir.path().join(PORTABLE_DATA_DIR_NAME);
        assert!(paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::CliFlag);
        assert_eq!(paths.app_data_dir, expected_root);
        assert_eq!(paths.vault_db_path, expected_root.join("vault_manifest.db"));
        assert_eq!(paths.webview2_data_dir, expected_root.join("WebView2"));
        assert_eq!(paths.runtime_cache_dir, expected_root.join("runtime"));
        assert!(webview2_mutex_name_for_paths(&paths).starts_with("Local\\WinDirStatLiquidGlass_WV2_"));
    }

    #[test]
    fn env_var_activates_portable_mode() {
        let exe_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &[],
            Some("1"),
            None,
            temp_dir.path(),
            false,
        );

        assert!(paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::EnvVar);
        assert_eq!(
            paths.app_data_dir,
            exe_dir.path().join(PORTABLE_DATA_DIR_NAME)
        );
    }

    #[test]
    fn marker_files_activate_portable_mode() {
        for marker_name in ["portable.txt", ".portable"] {
            let exe_dir = TempDir::new().unwrap();
            let temp_dir = TempDir::new().unwrap();
            fs::write(exe_dir.path().join(marker_name), "").unwrap();
            let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");

            let paths = resolve_runtime_paths_with(
                Some(&exe_path),
                &[],
                None,
                None,
                temp_dir.path(),
                false,
            );

            assert!(paths.is_portable, "marker {marker_name} should activate portable mode");
            assert_eq!(paths.mode_source, PortableModeSource::MarkerFile);
            assert_eq!(
                paths.app_data_dir,
                exe_dir.path().join(PORTABLE_DATA_DIR_NAME)
            );
        }
    }

    #[test]
    fn existing_data_dir_activates_portable_mode() {
        let exe_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let existing_data = exe_dir.path().join(PORTABLE_DATA_DIR_NAME);
        fs::create_dir_all(&existing_data).unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &[],
            None,
            None,
            temp_dir.path(),
            false,
        );

        assert!(paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::ExistingDataDir);
        assert_eq!(paths.app_data_dir, existing_data);
    }

    #[test]
    fn removable_drive_activates_portable_mode() {
        let exe_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &[],
            None,
            None,
            temp_dir.path(),
            true,
        );

        assert!(paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::RemovableDrive);
        assert_eq!(
            paths.app_data_dir,
            exe_dir.path().join(PORTABLE_DATA_DIR_NAME)
        );
    }

    #[test]
    fn unwritable_portable_parent_falls_back_to_temp_dir() {
        let exe_dir = TempDir::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        // Blocking regular file with the portable data folder name makes directory creation fail cleanly.
        fs::write(exe_dir.path().join(PORTABLE_DATA_DIR_NAME), b"blocker").unwrap();
        let exe_path = exe_dir.path().join("WinDirStat-LiquidGlass.exe");
        let args = vec!["--portable".to_string()];

        let paths = resolve_runtime_paths_with(
            Some(&exe_path),
            &args,
            None,
            None,
            temp_dir.path(),
            false,
        );

        let expected_fallback = temp_dir.path().join("WinDirStatLiquidGlass-Portable");
        assert!(paths.is_portable);
        assert_eq!(paths.mode_source, PortableModeSource::CliFlag);
        assert_eq!(paths.app_data_dir, expected_fallback);
        assert_eq!(paths.vault_db_path, expected_fallback.join("vault_manifest.db"));
        assert_eq!(paths.webview2_data_dir, expected_fallback.join("WebView2"));
        assert_eq!(paths.runtime_cache_dir, expected_fallback.join("runtime"));
    }
}

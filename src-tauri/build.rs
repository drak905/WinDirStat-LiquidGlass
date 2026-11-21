use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn target_arch_dir(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x64"),
        "x86" => Some("x86"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

fn parse_webview2_com_sys_version(manifest_dir: &Path) -> Option<String> {
    let lock_path = manifest_dir.join("Cargo.lock");
    let contents = fs::read_to_string(lock_path).ok()?;
    let mut in_pkg = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line == "[[package]]" {
            in_pkg = false;
        } else if line == r#"name = "webview2-com-sys""# {
            in_pkg = true;
        } else if in_pkg {
            if let Some(rest) = line.strip_prefix(r#"version = ""#) {
                if let Some(ver) = rest.strip_suffix('"') {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

fn cargo_home_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("CARGO_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(home));
    }
    if let Some(profile) = env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(profile).join(".cargo");
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(home).join(".cargo");
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

fn is_valid_pe_dll(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    fs::read(path)
        .map(|bytes| !bytes.is_empty() && bytes.starts_with(b"MZ"))
        .unwrap_or(false)
}

fn find_webview2_loader_dll(arch_dir: &str) -> Result<PathBuf, String> {
    let mut searched = Vec::new();

    if let Some(env_path) = env::var_os("WINDIRSTAT_WEBVIEW2_LOADER_DLL").filter(|v| !v.is_empty())
    {
        let candidate = PathBuf::from(env_path);
        searched.push(format!("WINDIRSTAT_WEBVIEW2_LOADER_DLL={}", candidate.display()));
        if is_valid_pe_dll(&candidate) {
            return Ok(candidate);
        }
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let exact_pkg_name = parse_webview2_com_sys_version(&manifest_dir)
        .map(|ver| format!("webview2-com-sys-{ver}"));

    for cargo_home in cargo_home_candidates() {
        let reg_src = cargo_home.join("registry").join("src");
        searched.push(reg_src.display().to_string());
        let Ok(index_entries) = fs::read_dir(&reg_src) else {
            continue;
        };
        let mut index_dirs: Vec<PathBuf> = index_entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        index_dirs.sort();

        if let Some(ref pkg_dir_name) = exact_pkg_name {
            for idx_dir in &index_dirs {
                let candidate = idx_dir
                    .join(pkg_dir_name)
                    .join(arch_dir)
                    .join("WebView2Loader.dll");
                searched.push(candidate.display().to_string());
                if is_valid_pe_dll(&candidate) {
                    return Ok(candidate);
                }
            }
        }

        let mut wildcard_matches = Vec::new();
        for idx_dir in &index_dirs {
            if let Ok(pkg_entries) = fs::read_dir(idx_dir) {
                for entry in pkg_entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("webview2-com-sys-") {
                        let candidate = entry.path().join(arch_dir).join("WebView2Loader.dll");
                        if is_valid_pe_dll(&candidate) {
                            wildcard_matches.push(candidate);
                        }
                    }
                }
            }
        }
        wildcard_matches.sort();
        if let Some(latest) = wildcard_matches.pop() {
            return Ok(latest);
        }
    }

    if let Ok(out_dir) = env::var("OUT_DIR") {
        let mut cur = Some(Path::new(&out_dir));
        while let Some(dir) = cur {
            if dir.file_name().and_then(|n| n.to_str()) == Some("build") {
                searched.push(dir.display().to_string());
                if let Ok(entries) = fs::read_dir(dir) {
                    let mut out_matches = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        if !name.to_string_lossy().starts_with("webview2-com-sys") {
                            continue;
                        }
                        if let Ok(hash_entries) = fs::read_dir(entry.path()) {
                            for h in hash_entries.flatten() {
                                let candidate = h
                                    .path()
                                    .join("out")
                                    .join(arch_dir)
                                    .join("WebView2Loader.dll");
                                if is_valid_pe_dll(&candidate) {
                                    out_matches.push(candidate);
                                }
                            }
                        }
                    }
                    out_matches.sort();
                    if let Some(matched) = out_matches.pop() {
                        return Ok(matched);
                    }
                }
                break;
            }
            cur = dir.parent();
        }
    }

    Err(format!(
        "Unable to locate Microsoft WebView2Loader.dll for arch '{arch_dir}'. Searched: {}",
        searched.join("; ")
    ))
}

fn prepare_embedded_webview2_loader() {
    println!("cargo:rerun-if-env-changed=WINDIRSTAT_WEBVIEW2_LOADER_DLL");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env == "msvc" {
        return;
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let Some(arch_dir) = target_arch_dir(&target_arch) else {
        panic!("Unsupported Windows target architecture for embedded WebView2Loader.dll: {target_arch}");
    };

    let src = match find_webview2_loader_dll(arch_dir) {
        Ok(path) => path,
        Err(msg) => panic!("{msg}"),
    };

    let bytes = fs::read(&src).unwrap_or_else(|e| {
        panic!(
            "Failed to read WebView2Loader.dll from {}: {e}",
            src.display()
        )
    });
    if bytes.is_empty() || !bytes.starts_with(b"MZ") {
        panic!(
            "Invalid WebView2Loader.dll payload at {} ({} bytes)",
            src.display(),
            bytes.len()
        );
    }

    let hash = fnv1a_64(&bytes);
    let tag = format!("{hash:016x}_{}", bytes.len());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"));
    let dest = out_dir.join("WebView2Loader.dll");
    fs::write(&dest, &bytes).unwrap_or_else(|e| {
        panic!(
            "Failed to stage embedded WebView2Loader.dll into {}: {e}",
            dest.display()
        )
    });

    println!(
        "cargo:rustc-env=EMBEDDED_WEBVIEW2_LOADER_PATH={}",
        dest.display()
    );
    println!("cargo:rustc-env=EMBEDDED_WEBVIEW2_LOADER_TAG={tag}");
    println!("cargo:rerun-if-changed={}", src.display());
}

fn main() {
    prepare_embedded_webview2_loader();
    tauri_build::build();
}

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|entry| entry.path());

    for entry in sorted {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                let web_path = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
                let abs = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                out.push((web_path, abs));
            }
        }
    }
}

fn guess_mime(web_path: &str) -> &'static str {
    if web_path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if web_path.ends_with(".js") || web_path.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if web_path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if web_path.ends_with(".svg") {
        "image/svg+xml"
    } else if web_path.ends_with(".png") {
        "image/png"
    } else if web_path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if web_path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist_dir = manifest_dir.join("..").join("src-ui").join("dist");
    println!("cargo:rerun-if-changed={}", dist_dir.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let dest = out_dir.join("embedded_ui_assets.rs");
    let mut file = fs::File::create(&dest).expect("create embedded_ui_assets.rs");

    let mut assets = Vec::new();
    if dist_dir.is_dir() {
        collect_files(&dist_dir, &dist_dir, &mut assets);
    }

    writeln!(
        file,
        "pub fn get_embedded_asset(path: &str) -> Option<(&'static [u8], &'static str)> {{"
    )
    .unwrap();
    writeln!(file, "    match path {{").unwrap();

    for (web_path, abs_path) in &assets {
        println!("cargo:rerun-if-changed={}", abs_path.display());
        let mime = guess_mime(web_path);
        let lit = abs_path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(
            file,
            "        \"{}\" => Some((include_bytes!(\"{}\"), \"{}\")),",
            web_path, lit, mime
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}

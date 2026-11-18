use super::{DuplicateGroup, IpcBridgeError, ScanNode};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use xxhash_rust::xxh64::Xxh64;

const IT_MYCOMPUTER: u64 = 1 << 0;
const IT_DRIVE: u64 = 1 << 1;
const IT_DIRECTORY: u64 = 1 << 2;
const IT_FILE: u64 = 1 << 3;
const IT_CONTAINER_BITS: u64 = IT_MYCOMPUTER | IT_DRIVE | IT_DIRECTORY;

struct RawRecord {
    explicit_id: Option<u64>,
    explicit_parent_id: Option<u64>,
    explicit_name: Option<String>,
    full_path: String,
    is_dir: bool,
    logical_size: u64,
    physical_size: u64,
    modified_utc: String,
    owner: String,
    xxhash64: Option<u64>,
    explicit_extension: Option<String>,
}

fn normalize_path_key(path: &str) -> String {
    let normalized = path.trim().replace('/', "\\");
    if is_drive_root(&normalized) {
        return normalized.to_ascii_lowercase();
    }
    normalized.trim_end_matches('\\').to_ascii_lowercase()
}

fn is_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

fn parent_path_key(path: &str) -> Option<String> {
    let normalized = path.trim().replace('/', "\\");
    if is_drive_root(&normalized) || normalized == "\\" {
        return None;
    }
    let trimmed = normalized.trim_end_matches('\\');
    if let Some(pos) = trimmed.rfind('\\') {
        if pos == 0 {
            return Some("\\".to_string());
        }
        let prefix = &trimmed[..pos];
        if prefix.len() == 2 && prefix.as_bytes()[1] == b':' {
            return Some(format!("{prefix}\\").to_ascii_lowercase());
        }
        if prefix == "\\" {
            return None;
        }
        return Some(prefix.to_ascii_lowercase());
    }
    None
}

fn extract_name(full_path: &str) -> String {
    let trimmed = full_path.trim();
    if is_drive_root(trimmed) {
        return trimmed.to_string();
    }
    let no_trail = trimmed.trim_end_matches(['\\', '/']);
    if let Some(pos) = no_trail.rfind(['\\', '/']) {
        let leaf = &no_trail[pos + 1..];
        if !leaf.is_empty() {
            return leaf.to_string();
        }
    }
    trimmed.to_string()
}

fn extract_extension(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(pos) => name[pos..].to_ascii_lowercase(),
        None => String::new(),
    }
}

fn parse_u64_hex_or_dec(text: &str) -> Option<u64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

fn value_as_u64(val: Option<&Value>) -> u64 {
    match val {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => parse_u64_hex_or_dec(s).unwrap_or(0),
        _ => 0,
    }
}

fn value_as_opt_u64(val: Option<&Value>) -> Option<u64> {
    match val {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.len() == 16 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
                u64::from_str_radix(trimmed, 16).ok()
            } else {
                parse_u64_hex_or_dec(trimmed)
            }
        }
        _ => None,
    }
}

fn value_as_str(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn infer_is_dir(wds_attrs: Option<u64>, attrs_str: &str, folders: u64, files: u64, path: &str) -> bool {
    if let Some(mask) = wds_attrs {
        let base_type = mask & 0x1FFF;
        if (base_type & IT_CONTAINER_BITS) != 0 {
            return true;
        }
        if (base_type & IT_FILE) != 0 {
            return false;
        }
    }
    if attrs_str.contains('D') || folders > 0 || files > 0 {
        return true;
    }
    path.ends_with('\\') || path.ends_with('/')
}

fn build_nodes_from_records(records: Vec<RawRecord>) -> Vec<ScanNode> {
    let mut nodes = Vec::with_capacity(records.len());
    let mut path_to_id: HashMap<String, u64> = HashMap::with_capacity(records.len());
    let mut next_id = 1u64;

    for rec in records {
        let id = rec.explicit_id.unwrap_or(next_id);
        next_id = next_id.max(id + 1);

        let key = normalize_path_key(&rec.full_path);
        let parent_id = rec
            .explicit_parent_id
            .or_else(|| parent_path_key(&rec.full_path).and_then(|pk| path_to_id.get(&pk).copied()));

        path_to_id.entry(key).or_insert(id);

        let name = rec
            .explicit_name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| extract_name(&rec.full_path));
        let extension = rec
            .explicit_extension
            .unwrap_or_else(|| extract_extension(&name, rec.is_dir));

        nodes.push(ScanNode {
            id,
            parent_id,
            name,
            full_path: rec.full_path,
            is_dir: rec.is_dir,
            logical_size: rec.logical_size,
            physical_size: rec.physical_size,
            modified_utc: rec.modified_utc,
            owner: rec.owner,
            xxhash64: rec.xxhash64,
            extension,
        });
    }
    nodes
}

pub fn parse_saveto_json(json_input: &str) -> Result<Vec<ScanNode>, IpcBridgeError> {
    let cleaned = json_input.trim_start_matches('\u{FEFF}').trim();
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }

    let root: Value = serde_json::from_str(cleaned)?;
    let items = match &root {
        Value::Array(arr) => arr,
        Value::Object(map) => match map.get("nodes").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                return Err(IpcBridgeError::Snapshot(
                    "expected JSON array or { nodes: [...] }".into(),
                ))
            }
        },
        _ => {
            return Err(IpcBridgeError::Snapshot(
                "expected top-level JSON array for /SaveTo snapshot".into(),
            ))
        }
    };

    let mut records = Vec::with_capacity(items.len());
    for entry in items {
        let obj = entry.as_object().ok_or_else(|| {
            IpcBridgeError::Snapshot("snapshot array element is not a JSON object".into())
        })?;

        let full_path = value_as_str(obj.get("full_path").or_else(|| obj.get("Name")));
        if full_path.is_empty() {
            return Err(IpcBridgeError::Snapshot(
                "snapshot object missing required Name/full_path field".into(),
            ));
        }

        let explicit_id = value_as_opt_u64(obj.get("id"));
        let explicit_parent_id = value_as_opt_u64(obj.get("parent_id"));
        let explicit_name = obj.get("name").and_then(|v| v.as_str()).map(str::to_string);
        let explicit_extension = obj
            .get("extension")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let wds_attrs = obj
            .get("WinDirStat Attributes")
            .and_then(|v| match v {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => parse_u64_hex_or_dec(s),
                _ => None,
            });
        let attrs_str = value_as_str(obj.get("Attributes"));
        let folders = value_as_u64(obj.get("Folders"));
        let files = value_as_u64(obj.get("Files"));
        let is_dir = obj
            .get("is_dir")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| infer_is_dir(wds_attrs, &attrs_str, folders, files, &full_path));

        let logical_size = value_as_u64(obj.get("logical_size").or_else(|| obj.get("Logical Size")));
        let physical_size =
            value_as_u64(obj.get("physical_size").or_else(|| obj.get("Physical Size")));
        let modified_utc = value_as_str(obj.get("modified_utc").or_else(|| obj.get("Last Change")));
        let owner = value_as_str(obj.get("owner").or_else(|| obj.get("Owner")));
        let xxhash64 = value_as_opt_u64(obj.get("xxhash64").or_else(|| obj.get("Hash")));

        records.push(RawRecord {
            explicit_id,
            explicit_parent_id,
            explicit_name,
            full_path,
            is_dir,
            logical_size,
            physical_size,
            modified_utc,
            owner,
            xxhash64,
            explicit_extension,
        });
    }

    Ok(build_nodes_from_records(records))
}

fn parse_csv_row(line: &str) -> Result<Vec<String>, IpcBridgeError> {
    let mut fields = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i <= bytes.len() {
        if i == bytes.len() {
            if line.ends_with(',') {
                fields.push(String::new());
            }
            break;
        }
        if bytes[i] == b'"' {
            i += 1;
            let mut seg_start = i;
            let mut val = String::new();
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                        val.push_str(&line[seg_start..i]);
                        val.push('"');
                        i += 2;
                        seg_start = i;
                        continue;
                    }
                    val.push_str(&line[seg_start..i]);
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(IpcBridgeError::Snapshot(
                    "unterminated quoted field in CSV snapshot".into(),
                ));
            }
            fields.push(val);
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            fields.push(line[start..i].trim().to_string());
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
            } else {
                break;
            }
        }
    }
    Ok(fields)
}

pub fn parse_saveto_csv(csv_input: &str) -> Result<Vec<ScanNode>, IpcBridgeError> {
    let cleaned = csv_input.trim_start_matches('\u{FEFF}');
    let mut lines = cleaned.lines().filter(|l| !l.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let headers = parse_csv_row(header_line.trim_end_matches('\r'))?;
    let find_col = |candidates: &[&str]| -> Option<usize> {
        headers.iter().position(|h| {
            candidates
                .iter()
                .any(|c| h.trim().eq_ignore_ascii_case(c))
        })
    };

    let idx_name = find_col(&["Name", "full_path"])
        .ok_or_else(|| IpcBridgeError::Snapshot("CSV snapshot missing Name header".into()))?;
    let idx_files = find_col(&["Files"]);
    let idx_folders = find_col(&["Folders"]);
    let idx_logical = find_col(&["Logical Size", "logical_size"]);
    let idx_physical = find_col(&["Physical Size", "physical_size"]);
    let idx_attrs = find_col(&["Attributes"]);
    let idx_last_change = find_col(&["Last Change", "modified_utc"]);
    let idx_wds_attrs = find_col(&["WinDirStat Attributes"]);
    let idx_owner = find_col(&["Owner", "owner"]);
    let idx_hash = find_col(&["xxhash64", "Hash"]);

    let mut records = Vec::new();
    for line in lines {
        let cols = parse_csv_row(line.trim_end_matches('\r'))?;
        let get = |idx: Option<usize>| -> &str {
            idx.and_then(|i| cols.get(i))
                .map(String::as_str)
                .unwrap_or("")
        };

        let full_path = get(Some(idx_name)).to_string();
        if full_path.is_empty() {
            return Err(IpcBridgeError::Snapshot(
                "CSV row has empty Name/path".into(),
            ));
        }
        let wds_attrs = parse_u64_hex_or_dec(get(idx_wds_attrs));
        let attrs_str = get(idx_attrs);
        let folders = get(idx_folders).parse::<u64>().unwrap_or(0);
        let files = get(idx_files).parse::<u64>().unwrap_or(0);
        let is_dir = infer_is_dir(wds_attrs, attrs_str, folders, files, &full_path);

        let logical_size = get(idx_logical).parse::<u64>().unwrap_or(0);
        let physical_size = get(idx_physical).parse::<u64>().unwrap_or(0);
        let modified_utc = get(idx_last_change).to_string();
        let owner = get(idx_owner).to_string();
        let xxhash64 = {
            let raw = get(idx_hash).trim();
            if raw.len() == 16 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
                u64::from_str_radix(raw, 16).ok()
            } else {
                parse_u64_hex_or_dec(raw)
            }
        };

        records.push(RawRecord {
            explicit_id: None,
            explicit_parent_id: None,
            explicit_name: None,
            full_path,
            is_dir,
            logical_size,
            physical_size,
            modified_utc,
            owner,
            xxhash64,
            explicit_extension: None,
        });
    }

    Ok(build_nodes_from_records(records))
}

pub fn parse_saveto_snapshot(input: &str) -> Result<Vec<ScanNode>, IpcBridgeError> {
    let trimmed = input.trim_start_matches('\u{FEFF}').trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        parse_saveto_json(input)
    } else {
        parse_saveto_csv(input)
    }
}

pub fn compute_file_xxhash64(path: &Path) -> Result<u64, IpcBridgeError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Xxh64::new(0);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.digest())
}

pub fn group_duplicates_by_xxhash64(
    nodes: &[ScanNode],
) -> Result<Vec<DuplicateGroup>, IpcBridgeError> {
    let mut by_size: HashMap<u64, Vec<&ScanNode>> = HashMap::new();
    for node in nodes {
        if !node.is_dir && node.logical_size > 0 {
            by_size.entry(node.logical_size).or_default().push(node);
        }
    }

    let mut by_hash: HashMap<(u64, u64), Vec<ScanNode>> = HashMap::new();
    for (size, candidates) in by_size {
        if candidates.len() < 2 {
            continue;
        }
        for node in candidates {
            let hash_opt = match node.xxhash64 {
                Some(h) => Some(h),
                None => match compute_file_xxhash64(Path::new(&node.full_path)) {
                    Ok(h) => Some(h),
                    Err(IpcBridgeError::Io(e))
                        if e.kind() == std::io::ErrorKind::NotFound
                            || e.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        None
                    }
                    Err(e) => return Err(e),
                },
            };

            if let Some(hash) = hash_opt {
                let mut cloned = node.clone();
                cloned.xxhash64 = Some(hash);
                by_hash.entry((size, hash)).or_default().push(cloned);
            }
        }
    }

    let mut groups = Vec::new();
    for ((file_size, xxhash64), mut files) in by_hash {
        if files.len() < 2 {
            continue;
        }
        files.sort_by(|a, b| a.full_path.cmp(&b.full_path));
        let count = files.len();
        let wasted_bytes = (count as u64 - 1).saturating_mul(file_size);
        groups.push(DuplicateGroup {
            xxhash64,
            hash_hex: format!("{xxhash64:016x}"),
            file_size,
            count,
            wasted_bytes,
            files,
        });
    }

    groups.sort_by(|a, b| {
        b.wasted_bytes
            .cmp(&a.wasted_bytes)
            .then_with(|| a.xxhash64.cmp(&b.xxhash64))
    });
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_official_windirstat_saveto_json_snapshot() {
        let json = "\u{FEFF}[\r\n\
{\r\n\
  \"Name\": \"C:\\\\Project\",\r\n\
  \"Files\": 2,\r\n\
  \"Folders\": 1,\r\n\
  \"Logical Size\": 3072,\r\n\
  \"Physical Size\": 8192,\r\n\
  \"Attributes\": \"D\",\r\n\
  \"Last Change\": \"2026-11-18T12:00:00Z\",\r\n\
  \"WinDirStat Attributes\": \"0x10000004\",\r\n\
  \"Index\": \"0x0000000000000001\",\r\n\
  \"Owner\": \"BUILTIN\\\\Administrators\",\r\n\
  \"ScanOutcome\": \"complete\",\r\n\
  \"ScanError\": \"\"\r\n\
},\r\n\
{\r\n\
  \"Name\": \"C:\\\\Project\\\\src\",\r\n\
  \"Files\": 1,\r\n\
  \"Folders\": 0,\r\n\
  \"Logical Size\": 1024,\r\n\
  \"Physical Size\": null,\r\n\
  \"Attributes\": \"D\",\r\n\
  \"Last Change\": \"2026-11-18T12:01:00Z\",\r\n\
  \"WinDirStat Attributes\": \"0x00000004\",\r\n\
  \"Index\": \"0x0000000000000002\",\r\n\
  \"ScanOutcome\": \"complete\",\r\n\
  \"ScanError\": \"\"\r\n\
},\r\n\
{\r\n\
  \"Name\": \"C:\\\\Project\\\\src\\\\main.rs\",\r\n\
  \"Files\": 0,\r\n\
  \"Folders\": 0,\r\n\
  \"Logical Size\": 1024,\r\n\
  \"Physical Size\": 4096,\r\n\
  \"Attributes\": \"A\",\r\n\
  \"Last Change\": \"2026-11-18T12:01:00Z\",\r\n\
  \"WinDirStat Attributes\": \"0x00000008\",\r\n\
  \"Index\": \"0x0000000000000003\",\r\n\
  \"ScanOutcome\": \"complete\",\r\n\
  \"ScanError\": \"\"\r\n\
},\r\n\
{\r\n\
  \"Name\": \"C:\\\\Project\\\\README.md\",\r\n\
  \"Files\": 0,\r\n\
  \"Folders\": 0,\r\n\
  \"Logical Size\": 2048,\r\n\
  \"Physical Size\": 4096,\r\n\
  \"Attributes\": \"A\",\r\n\
  \"Last Change\": \"2026-11-18T12:02:00Z\",\r\n\
  \"WinDirStat Attributes\": \"0x00000008\",\r\n\
  \"Index\": \"0x0000000000000004\",\r\n\
  \"ScanOutcome\": \"complete\",\r\n\
  \"ScanError\": \"\"\r\n\
}\r\n]";

        let nodes = parse_saveto_json(json).unwrap();
        assert_eq!(nodes.len(), 4);

        assert_eq!(nodes[0].id, 1);
        assert_eq!(nodes[0].parent_id, None);
        assert!(nodes[0].is_dir);
        assert_eq!(nodes[0].name, "Project");
        assert_eq!(nodes[0].owner, "BUILTIN\\Administrators");

        assert_eq!(nodes[1].id, 2);
        assert_eq!(nodes[1].parent_id, Some(1));
        assert!(nodes[1].is_dir);
        assert_eq!(nodes[1].physical_size, 0);

        assert_eq!(nodes[2].id, 3);
        assert_eq!(nodes[2].parent_id, Some(2));
        assert!(!nodes[2].is_dir);
        assert_eq!(nodes[2].name, "main.rs");
        assert_eq!(nodes[2].extension, ".rs");
        assert_eq!(nodes[2].logical_size, 1024);

        assert_eq!(nodes[3].id, 4);
        assert_eq!(nodes[3].parent_id, Some(1));
        assert!(!nodes[3].is_dir);
        assert_eq!(nodes[3].extension, ".md");
    }

    #[test]
    fn parses_saveto_csv_fallback_snapshot() {
        let csv = "\"Name\",\"Files\",\"Folders\",\"Logical Size\",\"Physical Size\",\"Attributes\",\"Last Change\",\"WinDirStat Attributes\",\"Index\",\"ScanOutcome\",\"ScanError\"\r\n\
\"C:\\Root\",1,0,500,4096,\"D\",2026-11-18T10:00:00Z,0x10000004,0x0000000000000001,\"complete\",\"\"\r\n\
\"C:\\Root\\app.EXE\",0,0,500,4096,\"A\",2026-11-18T10:00:00Z,0x00000008,0x0000000000000002,\"complete\",\"\"\r\n";

        let nodes = parse_saveto_snapshot(csv).unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].is_dir);
        assert_eq!(nodes[1].parent_id, Some(1));
        assert!(!nodes[1].is_dir);
        assert_eq!(nodes[1].extension, ".exe");
    }

    #[test]
    fn xxhash64_groups_real_duplicate_files_and_computes_wasted_bytes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wds_xxhash_test_{}_{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();

        let p1 = dir.join("copy1.bin");
        let p2 = dir.join("copy2.bin");
        let p3 = dir.join("copy3.bin");
        let p_diff = dir.join("same_size_diff_content.bin");
        let p_unique = dir.join("unique_size.bin");

        let payload_dup = vec![0x5Au8; 4096];
        let mut payload_diff = vec![0x5Au8; 4096];
        payload_diff[2048] = 0xA5;
        let payload_unique = vec![0x11u8; 1024];

        fs::write(&p1, &payload_dup).unwrap();
        fs::write(&p2, &payload_dup).unwrap();
        fs::write(&p3, &payload_dup).unwrap();
        fs::write(&p_diff, &payload_diff).unwrap();
        fs::write(&p_unique, &payload_unique).unwrap();

        let make_file_node = |id: u64, path: &Path, size: u64| ScanNode {
            id,
            parent_id: Some(1),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            full_path: path.to_string_lossy().into_owned(),
            is_dir: false,
            logical_size: size,
            physical_size: size,
            modified_utc: String::new(),
            owner: String::new(),
            xxhash64: None,
            extension: ".bin".into(),
        };

        let nodes = vec![
            make_file_node(2, &p1, 4096),
            make_file_node(3, &p2, 4096),
            make_file_node(4, &p3, 4096),
            make_file_node(5, &p_diff, 4096),
            make_file_node(6, &p_unique, 1024),
        ];

        let groups = group_duplicates_by_xxhash64(&nodes).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 3);
        assert_eq!(groups[0].file_size, 4096);
        assert_eq!(groups[0].wasted_bytes, 2 * 4096);
        assert_eq!(groups[0].hash_hex.len(), 16);
        assert_eq!(groups[0].files.len(), 3);
    }

    #[test]
    fn xxhash64_groups_prehashed_nodes_and_sorts_by_waste_descending() {
        let make = |id: u64, path: &str, size: u64, hash: u64| ScanNode {
            id,
            parent_id: Some(1),
            name: path.into(),
            full_path: path.into(),
            is_dir: false,
            logical_size: size,
            physical_size: size,
            modified_utc: String::new(),
            owner: String::new(),
            xxhash64: Some(hash),
            extension: ".iso".into(),
        };

        let nodes = vec![
            make(1, r"C:\a1.iso", 100, 0x1111),
            make(2, r"C:\a2.iso", 100, 0x1111),
            make(3, r"C:\b1.iso", 500, 0x2222),
            make(4, r"C:\b2.iso", 500, 0x2222),
            make(5, r"C:\b3.iso", 500, 0x2222),
        ];

        let groups = group_duplicates_by_xxhash64(&nodes).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].wasted_bytes, 1000);
        assert_eq!(groups[0].hash_hex, "0000000000002222");
        assert_eq!(groups[1].wasted_bytes, 100);
        assert_eq!(groups[1].hash_hex, "0000000000001111");
    }

    #[test]
    fn csv_snapshot_handles_escaped_quotes_and_drive_roots() {
        let csv = "\"Name\",\"Files\",\"Folders\",\"Logical Size\",\"Physical Size\",\"Attributes\",\"Last Change\",\"WinDirStat Attributes\",\"Index\"\r\n\
\"C:\\\",1,0,128,4096,\"D\",2026-11-18T10:00:00Z,0x10000002,0x0000000000000001\r\n\
\"C:\\my \"\"quoted\"\" file.txt\",0,0,128,4096,\"A\",2026-11-18T10:00:00Z,0x00000008,0x0000000000000002\r\n";

        let nodes = parse_saveto_csv(csv).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].full_path, "C:\\");
        assert_eq!(nodes[0].parent_id, None);
        assert_eq!(nodes[1].parent_id, Some(1));
        assert_eq!(nodes[1].name, "my \"quoted\" file.txt");
        assert_eq!(nodes[1].extension, ".txt");
    }
}

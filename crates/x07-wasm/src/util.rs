use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::report::meta::FileDigest;

pub fn file_digest(path: &Path) -> Result<FileDigest> {
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    Ok(FileDigest {
        path: path.display().to_string(),
        sha256: sha256_hex(&bytes),
        bytes_len: bytes.len() as u64,
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex_lower(&h.finalize())
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(nibble_to_hex((b >> 4) & 0xF));
        out.push(nibble_to_hex(b & 0xF));
    }
    out
}

fn nibble_to_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

pub fn canon_value_jcs(v: &mut Value) {
    match v {
        Value::Array(arr) => {
            for x in arr {
                canon_value_jcs(x);
            }
        }
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, val) in entries.iter_mut() {
                canon_value_jcs(val);
            }
            map.clear();
            for (k, val) in entries {
                map.insert(k, val);
            }
        }
        _ => {}
    }
}

pub fn wasmtime_version() -> Option<String> {
    option_env!("X07_WASM_WASMTIME_VERSION").map(|s| s.to_string())
}

pub fn truncate_bytes_lossy(bytes: &[u8], max_bytes: usize) -> String {
    if bytes.len() <= max_bytes {
        return String::from_utf8_lossy(bytes).to_string();
    }
    let head = &bytes[..max_bytes];
    let mut s = String::from_utf8_lossy(head).to_string();
    s.push_str("...(truncated)");
    s
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    let mut entries = std::fs::read_dir(src)
        .with_context(|| format!("read dir: {}", src.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let ty = e.file_type()?;
        let name = e.file_name();
        let src_path = e.path();
        let dst_path = dst.join(name);
        if ty.is_dir() {
            std::fs::create_dir_all(&dst_path)
                .with_context(|| format!("create dir: {}", dst_path.display()))?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            std::fs::copy(&src_path, &dst_path).with_context(|| {
                format!("copy {} -> {}", src_path.display(), dst_path.display())
            })?;
        }
    }
    Ok(())
}

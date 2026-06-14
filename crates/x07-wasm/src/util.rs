use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::report::meta::FileDigest;

pub fn file_digest(path: &Path) -> Result<FileDigest> {
    let (sha256, bytes_len) = sha256_file_hex(path)?;
    Ok(FileDigest {
        path: path.display().to_string(),
        sha256,
        bytes_len,
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

pub fn sha256_file_hex(path: &Path) -> Result<(String, u64)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open: {}", path.display()))?;
    let mut h = Sha256::new();
    let mut bytes_len: u64 = 0;

    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("read: {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        bytes_len += n as u64;
    }

    Ok((hex_lower(&h.finalize()), bytes_len))
}

pub fn out_tmp_path(out: &Path) -> PathBuf {
    let Some(file_name) = out.file_name() else {
        return PathBuf::from(format!("{}.tmp", out.display()));
    };
    let mut file_name_tmp: OsString = OsString::from(file_name);
    file_name_tmp.push(OsStr::new(".tmp"));

    let mut out_tmp = out.to_path_buf();
    out_tmp.set_file_name(file_name_tmp);
    out_tmp
}

// Fail-closed invariant: remove any stale output (and temp output) before
// commands begin producing new bytes.
pub fn preunlink_out(out: &Path) -> PathBuf {
    let out_tmp = out_tmp_path(out);
    let _ = std::fs::remove_file(out);
    let _ = std::fs::remove_file(&out_tmp);
    out_tmp
}

pub fn write_file_atomic(out: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let out_tmp = out_tmp_path(out);
    let w = std::fs::write(&out_tmp, bytes);
    if let Err(err) = w {
        let _ = std::fs::remove_file(&out_tmp);
        return Err(err);
    }
    let r = std::fs::rename(&out_tmp, out);
    if let Err(err) = r {
        let _ = std::fs::remove_file(&out_tmp);
        return Err(err);
    }
    Ok(())
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

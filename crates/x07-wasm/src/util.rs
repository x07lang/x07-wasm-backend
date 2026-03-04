use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

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

#[derive(Debug, Clone)]
pub struct FileReadCappedError {
    pub kind: &'static str,
    pub path: String,
    pub bytes_len: u64,
    pub max_bytes: u64,
    pub detail: String,
}

fn file_len_checked(path: &Path, max_bytes: u64) -> std::result::Result<u64, FileReadCappedError> {
    let md = std::fs::metadata(path).map_err(|err| FileReadCappedError {
        kind: "metadata_failed",
        path: path.display().to_string(),
        bytes_len: 0,
        max_bytes,
        detail: format!("metadata failed: {err}"),
    })?;
    if !md.is_file() {
        return Err(FileReadCappedError {
            kind: "not_file",
            path: path.display().to_string(),
            bytes_len: md.len(),
            max_bytes,
            detail: "not a regular file".to_string(),
        });
    }
    let len = md.len();
    if len > max_bytes {
        return Err(FileReadCappedError {
            kind: "too_large",
            path: path.display().to_string(),
            bytes_len: len,
            max_bytes,
            detail: format!("file size {len} exceeds cap {max_bytes}"),
        });
    }
    if len > (usize::MAX as u64) {
        return Err(FileReadCappedError {
            kind: "too_large",
            path: path.display().to_string(),
            bytes_len: len,
            max_bytes,
            detail: "file too large for this platform".to_string(),
        });
    }
    Ok(len)
}

pub fn read_file_capped(
    path: &Path,
    max_bytes: u64,
) -> std::result::Result<Vec<u8>, FileReadCappedError> {
    let len = file_len_checked(path, max_bytes)?;
    let mut f = std::fs::File::open(path).map_err(|err| FileReadCappedError {
        kind: "open_failed",
        path: path.display().to_string(),
        bytes_len: len,
        max_bytes,
        detail: format!("open failed: {err}"),
    })?;
    let mut buf: Vec<u8> = Vec::with_capacity(len as usize);
    f.read_to_end(&mut buf).map_err(|err| FileReadCappedError {
        kind: "read_failed",
        path: path.display().to_string(),
        bytes_len: len,
        max_bytes,
        detail: format!("read failed: {err}"),
    })?;
    if (buf.len() as u64) > max_bytes {
        return Err(FileReadCappedError {
            kind: "too_large",
            path: path.display().to_string(),
            bytes_len: buf.len() as u64,
            max_bytes,
            detail: "read exceeded cap".to_string(),
        });
    }
    Ok(buf)
}

pub fn sha256_file_hex_capped(
    path: &Path,
    max_bytes: u64,
) -> std::result::Result<(String, u64), FileReadCappedError> {
    let len = file_len_checked(path, max_bytes)?;
    let mut f = std::fs::File::open(path).map_err(|err| FileReadCappedError {
        kind: "open_failed",
        path: path.display().to_string(),
        bytes_len: len,
        max_bytes,
        detail: format!("open failed: {err}"),
    })?;

    let mut h = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf).map_err(|err| FileReadCappedError {
            kind: "read_failed",
            path: path.display().to_string(),
            bytes_len: len,
            max_bytes,
            detail: format!("read failed: {err}"),
        })?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_bytes {
            return Err(FileReadCappedError {
                kind: "too_large",
                path: path.display().to_string(),
                bytes_len: total,
                max_bytes,
                detail: "read exceeded cap".to_string(),
            });
        }
        h.update(&buf[..n]);
    }
    Ok((hex_lower(&h.finalize()), total))
}

pub fn file_digest_capped(
    path: &Path,
    max_bytes: u64,
) -> std::result::Result<FileDigest, FileReadCappedError> {
    let (sha256, bytes_len) = sha256_file_hex_capped(path, max_bytes)?;
    Ok(FileDigest {
        path: path.display().to_string(),
        sha256,
        bytes_len,
    })
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

#[derive(Debug, Clone)]
pub struct UnsafePathError {
    pub kind: &'static str,
    pub rel: String,
    pub detail: String,
}

pub fn safe_join_under_dir(
    base_dir: &Path,
    rel: &str,
) -> std::result::Result<PathBuf, UnsafePathError> {
    if rel.is_empty() {
        return Err(UnsafePathError {
            kind: "empty",
            rel: rel.to_string(),
            detail: "path is empty".to_string(),
        });
    }

    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(UnsafePathError {
            kind: "absolute",
            rel: rel.to_string(),
            detail: "absolute paths are not allowed".to_string(),
        });
    }

    let mut cleaned = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(part) => cleaned.push(part),
            _ => {
                return Err(UnsafePathError {
                    kind: "non_normal_component",
                    rel: rel.to_string(),
                    detail: "path must not contain ., .., or prefix/root components".to_string(),
                })
            }
        }
    }

    // Reject symlinks in any existing path component under base_dir.
    // Missing paths are handled by callers (e.g. verify commands emit "missing file" diagnostics).
    let mut cur = base_dir.to_path_buf();
    for c in cleaned.components() {
        let Component::Normal(part) = c else {
            continue;
        };
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(md) => {
                if md.file_type().is_symlink() {
                    return Err(UnsafePathError {
                        kind: "symlink",
                        rel: rel.to_string(),
                        detail: format!("symlink path component: {}", cur.display()),
                    });
                }
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    break;
                }
                return Err(UnsafePathError {
                    kind: "metadata_failed",
                    rel: rel.to_string(),
                    detail: format!("symlink_metadata failed for {}: {err}", cur.display()),
                });
            }
        }
    }

    Ok(base_dir.join(cleaned))
}

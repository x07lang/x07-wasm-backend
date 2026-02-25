use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

use crate::report;
use crate::util;

#[derive(Debug, Clone)]
pub struct LoadedBytes {
    pub bytes: Vec<u8>,
    pub blob_ref: Value,
}

pub fn load_file(path: &Path, meta: &mut report::meta::ReportMeta) -> Result<LoadedBytes> {
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let len = bytes.len();
    let sha = util::sha256_hex(&bytes);
    meta.inputs.push(report::meta::FileDigest {
        path: path.display().to_string(),
        sha256: sha.clone(),
        bytes_len: len as u64,
    });
    Ok(LoadedBytes {
        bytes,
        blob_ref: json!({ "bytes_len": len as u64, "sha256": sha, "path": path.display().to_string() }),
    })
}

pub fn load_hex(hex_s: &str) -> Result<LoadedBytes> {
    let bytes = hex::decode(hex_s.trim()).context("hex decode")?;
    let len = bytes.len();
    let sha = util::sha256_hex(&bytes);
    Ok(LoadedBytes {
        bytes,
        blob_ref: json!({ "bytes_len": len as u64, "sha256": sha }),
    })
}

pub fn load_base64(b64: &str) -> Result<LoadedBytes> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .context("base64 decode")?;
    let len = bytes.len();
    let sha = util::sha256_hex(&bytes);
    let canon_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(LoadedBytes {
        bytes,
        blob_ref: json!({ "bytes_len": len as u64, "sha256": sha, "base64": canon_b64 }),
    })
}

pub fn load_bytes_spec(spec: &str, meta: &mut report::meta::ReportMeta) -> Result<LoadedBytes> {
    let s = spec.trim();
    if s.is_empty() {
        return Ok(LoadedBytes {
            bytes: Vec::new(),
            blob_ref: json!({ "bytes_len": 0, "sha256": util::sha256_hex(&[]) }),
        });
    }

    if let Some(rest) = s.strip_prefix("hex:") {
        return load_hex(rest);
    }
    if let Some(rest) = s.strip_prefix("b64:") {
        return load_base64(rest);
    }
    if let Some(rest) = s.strip_prefix('@') {
        return load_file(Path::new(rest), meta);
    }

    anyhow::bail!("unsupported bytes spec (expected hex:, b64:, or @path)")
}

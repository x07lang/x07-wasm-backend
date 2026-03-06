use std::path::Path;

use anyhow::{Context, Result};

use crate::cmdutil;
use crate::diag::{Diagnostic, Stage};
use crate::report::meta::FileDigest;
use crate::util;

pub const ENV_ADAPTERS_FROM_SOURCE: &str = "X07WASM_ADAPTERS_FROM_SOURCE";

pub const EMBEDDED_HTTP_ADAPTER_COMPONENT_WASM: &[u8] =
    include_bytes!("support/adapters/http-adapter.component.wasm");
pub const EMBEDDED_HTTP_STATE_DOC_ADAPTER_COMPONENT_WASM: &[u8] =
    include_bytes!("support/adapters/http-state-doc-adapter.component.wasm");
pub const EMBEDDED_CLI_ADAPTER_COMPONENT_WASM: &[u8] =
    include_bytes!("support/adapters/cli-adapter.component.wasm");
pub const EMBEDDED_WEB_UI_ADAPTER_COMPONENT_WASM: &[u8] =
    include_bytes!("support/adapters/web-ui-adapter.component.wasm");

pub fn adapters_from_source_enabled() -> bool {
    let Some(v) = std::env::var_os(ENV_ADAPTERS_FROM_SOURCE) else {
        return false;
    };
    let s = v.to_string_lossy();
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    !matches!(s.to_ascii_lowercase().as_str(), "0" | "false" | "no")
}

pub fn embedded_digest(path: &str, bytes: &[u8]) -> FileDigest {
    FileDigest {
        path: path.to_string(),
        sha256: util::sha256_hex(bytes),
        bytes_len: bytes.len() as u64,
    }
}

pub fn write_bytes(out_path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))?;
    }

    let out_tmp = util::preunlink_out(out_path);
    std::fs::write(&out_tmp, bytes).with_context(|| format!("write: {}", out_tmp.display()))?;
    std::fs::rename(&out_tmp, out_path)
        .with_context(|| format!("rename {} -> {}", out_tmp.display(), out_path.display()))?;
    Ok(())
}

pub fn build_wasm32_wasip2_release_bytes(
    manifest_path: &Path,
    built_wasm_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
    label: &str,
) -> Option<Vec<u8>> {
    let cargo_args: Vec<String> = vec![
        "build".to_string(),
        "--release".to_string(),
        "--locked".to_string(),
        "--target".to_string(),
        "wasm32-wasip2".to_string(),
        "--manifest-path".to_string(),
        manifest_path.display().to_string(),
    ];

    let cargo_out = match cmdutil::run_cmd_capture("cargo", &cargo_args) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_CARGO_BUILD_SPAWN_FAILED",
                Stage::Run,
                label,
                &err,
            ));
            return None;
        }
    };
    if !cargo_out.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_CARGO_BUILD_FAILED",
            Stage::Run,
            label,
            cargo_out.code,
            &cargo_out.stderr,
        ));
        return None;
    }

    let bytes = match std::fs::read(built_wasm_path) {
        Ok(v) => v,
        Err(_) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_ADAPTER_BUILD_OUTPUT_MISSING",
                crate::diag::Severity::Error,
                Stage::Run,
                format!(
                    "adapter build output missing: {}",
                    built_wasm_path.display()
                ),
            ));
            return None;
        }
    };
    Some(bytes)
}

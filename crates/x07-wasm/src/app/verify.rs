use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{AppVerifyArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_app_verify(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppVerifyArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let pack_digest = match util::file_digest(&args.pack_manifest) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_VERIFY_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to digest pack manifest {}: {err:#}",
                    args.pack_manifest.display()
                ),
            ));
            report::meta::FileDigest {
                path: args.pack_manifest.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let bytes = match std::fs::read(&args.pack_manifest) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_VERIFY_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read pack manifest {}: {err}",
                    args.pack_manifest.display()
                ),
            ));
            Vec::new()
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_VERIFY_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("pack manifest is not JSON: {err}"),
            ));
            Value::Null
        }
    };

    let mut schema_valid = false;
    if doc_json != Value::Null {
        match store.validate("https://x07.io/spec/x07-app.pack.schema.json", &doc_json) {
            Ok(diags) => {
                if diags.is_empty() {
                    schema_valid = true;
                } else {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_VERIFY_SCHEMA_INVALID",
                        Severity::Error,
                        Stage::Parse,
                        "pack manifest schema invalid".to_string(),
                    ));
                    diagnostics.extend(diags);
                }
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_VERIFY_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
            }
        }
    }

    let pack_dir = args
        .pack_manifest
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let assets = doc_json.get("assets").and_then(Value::as_array).cloned();
    let assets = assets.unwrap_or_default();
    let assets_count = assets.len() as u64;

    let mut assets_checked: u64 = 0;
    let mut missing_assets: u64 = 0;
    let mut digest_mismatches: u64 = 0;
    let mut headers_invalid: u64 = 0;

    if schema_valid {
        for asset in assets {
            let file = asset.get("file").cloned().unwrap_or(Value::Null);
            let rel = file
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let want_sha = file.get("sha256").and_then(Value::as_str).unwrap_or("");
            let want_len = file.get("bytes_len").and_then(Value::as_u64).unwrap_or(0);

            let serve_path = asset
                .get("serve_path")
                .and_then(Value::as_str)
                .unwrap_or("");
            let full = pack_dir.join(&rel);
            if !full.is_file() {
                missing_assets += 1;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_VERIFY_MISSING_ASSET",
                    Severity::Error,
                    Stage::Run,
                    format!("missing asset file: {}", full.display()),
                ));
                continue;
            }

            let bytes = match std::fs::read(&full) {
                Ok(v) => v,
                Err(err) => {
                    missing_assets += 1;
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_VERIFY_MISSING_ASSET",
                        Severity::Error,
                        Stage::Run,
                        format!("failed to read asset file {}: {err}", full.display()),
                    ));
                    continue;
                }
            };
            assets_checked += 1;
            let got_sha = util::sha256_hex(&bytes);
            let got_len = bytes.len() as u64;
            if got_sha != want_sha || got_len != want_len {
                digest_mismatches += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_APP_VERIFY_DIGEST_MISMATCH",
                    Severity::Error,
                    Stage::Run,
                    "asset digest mismatch".to_string(),
                );
                d.data.insert("asset_path".to_string(), json!(rel));
                d.data.insert("want_sha256".to_string(), json!(want_sha));
                d.data.insert("got_sha256".to_string(), json!(got_sha));
                d.data.insert("want_bytes_len".to_string(), json!(want_len));
                d.data.insert("got_bytes_len".to_string(), json!(got_len));
                diagnostics.push(d);
            }

            if (serve_path.ends_with(".wasm") || rel.ends_with(".wasm"))
                && !has_wasm_content_type_header(&asset)
            {
                headers_invalid += 1;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_VERIFY_HEADERS_INVALID",
                    Severity::Error,
                    Stage::Run,
                    "wasm asset missing required content-type application/wasm".to_string(),
                ));
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.app.verify.report@0.1.0",
      "command": "x07-wasm.app.verify",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "stdout": { "bytes_len": 0 },
        "stderr": { "bytes_len": 0 },
        "stdout_json": {
          "pack_manifest": pack_digest,
          "assets_count": assets_count,
          "assets_checked": assets_checked,
          "missing_assets": missing_assets,
          "digest_mismatches": digest_mismatches,
          "headers_invalid": headers_invalid,
        }
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn has_wasm_content_type_header(asset: &Value) -> bool {
    let hdrs = asset
        .get("headers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for h in hdrs {
        let k = h
            .get("k")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if k != "content-type" {
            continue;
        }
        let v = h.get("v").and_then(Value::as_str).unwrap_or("");
        if v.trim() == "application/wasm" {
            return true;
        }
    }
    false
}

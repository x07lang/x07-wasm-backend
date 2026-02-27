use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{CapsValidateArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_caps_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: CapsValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let (cap_status, compatibility_hash) =
        validate_caps_profile(&store, &args.profile, &mut meta, &mut diagnostics)?;

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.caps.validate.report@0.1.0",
      "command": "x07-wasm.caps.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "capabilities": cap_status,
        "compatibility_hash": compatibility_hash,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn validate_caps_profile(
    store: &SchemaStore,
    path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Value, String)> {
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_CAPS_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read capabilities profile {}: {err}",
                    path.display()
                ),
            ));
            let status = json!({
              "path": path.display().to_string(),
              "ok": false,
              "schema_valid": false,
              "sha256": null,
            });
            return Ok((status, "0".repeat(64)));
        }
    };

    let digest = report::meta::FileDigest {
        path: path.display().to_string(),
        sha256: util::sha256_hex(&bytes),
        bytes_len: bytes.len() as u64,
    };
    meta.inputs.push(digest.clone());

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_CAPS_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("capabilities JSON invalid: {err}"),
            ));
            let status = json!({
              "path": digest.path,
              "ok": false,
              "schema_valid": false,
              "sha256": digest.sha256,
            });
            return Ok((status, "0".repeat(64)));
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-app.capabilities.schema.json",
        &doc_json,
    )?;
    let schema_valid = schema_diags.is_empty();
    if !schema_valid {
        for dd in schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_CAPS_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
    }

    let status = json!({
      "path": digest.path,
      "ok": schema_valid,
      "schema_valid": schema_valid,
      "sha256": digest.sha256,
    });

    let compat_doc = json!({
      "capabilities_sha256": digest.sha256,
    });
    let bytes = report::canon::canonical_json_bytes(&compat_doc)?;
    let compatibility_hash = util::sha256_hex(&bytes);

    Ok((status, compatibility_hash))
}

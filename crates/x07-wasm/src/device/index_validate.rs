use std::ffi::OsString;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{DeviceIndexValidateArgs, MachineArgs, Scope};
use crate::device::contracts::DeviceIndexDoc;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_DEVICE_PROFILE_ID: &str = "device_dev";

pub fn cmd_device_index_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceIndexValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let index_path = args.index;
    let index_digest = match util::file_digest(&index_path) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            Some(d)
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read device index {}: {err:#}",
                    index_path.display()
                ),
            ));
            None
        }
    };

    let index_bytes = match std::fs::read(&index_path) {
        Ok(b) => b,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read device index {}: {err}",
                    index_path.display()
                ),
            ));
            Vec::new()
        }
    };

    let index_doc_json: Value = match serde_json::from_slice(&index_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_INDEX_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse device index JSON: {err}"),
            ));
            json!(null)
        }
    };

    let mut index_ok = diagnostics.iter().all(|d| d.severity != Severity::Error);

    let mut index_parsed: Option<DeviceIndexDoc> = None;
    if index_doc_json != Value::Null {
        let schema_diags = store.validate(
            "https://x07.io/spec/x07-arch.device.index.schema.json",
            &index_doc_json,
        )?;
        if schema_diags.iter().any(|d| d.severity == Severity::Error) {
            index_ok = false;
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "device index schema invalid".to_string(),
            );
            d.data.insert("errors".to_string(), json!(schema_diags));
            diagnostics.push(d);
        } else {
            match serde_json::from_value(index_doc_json.clone()) {
                Ok(v) => index_parsed = Some(v),
                Err(err) => {
                    index_ok = false;
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_INDEX_PARSE_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!("failed to parse device index doc: {err}"),
                    ));
                }
            }
        }
    } else {
        index_ok = false;
    }

    if let Some(idx) = index_parsed.as_ref() {
        let mut seen = std::collections::BTreeSet::new();
        for p in &idx.profiles {
            if !seen.insert(p.id.clone()) {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_INDEX_DUPLICATE_PROFILE_ID",
                    Severity::Error,
                    Stage::Parse,
                    format!("duplicate profile id in device index: {:?}", p.id),
                ));
            }
        }
    }

    let mut default_profile_id: Option<String> = None;
    let mut default_profile_found = false;
    let mut profiles_count: u64 = 0;

    if let Some(idx) = index_parsed.as_ref() {
        profiles_count = idx.profiles.len() as u64;
        default_profile_id = idx
            .defaults
            .as_ref()
            .and_then(|d| d.default_profile_id.clone())
            .or_else(|| Some(DEFAULT_DEVICE_PROFILE_ID.to_string()));
        if let Some(def) = default_profile_id.as_deref() {
            default_profile_found = idx.profiles.iter().any(|p| p.id == def);
            if !default_profile_found {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_INDEX_DEFAULT_PROFILE_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("default profile id not found in device index: {def:?}"),
                ));
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let index_summary = json!({
      "path": index_digest.map(|d| d.path).unwrap_or_else(|| index_path.display().to_string()),
      "ok": index_ok,
      "profiles_count": profiles_count,
      "default_profile_id": default_profile_id,
      "default_profile_found": default_profile_found,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.device.index.validate.report@0.1.0",
      "command": "x07-wasm.device.index.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": "index_v1",
        "index": index_summary,
      }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

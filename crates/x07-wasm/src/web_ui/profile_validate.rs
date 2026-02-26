use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, WebUiProfileValidateArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_WEB_UI_PROFILE_ID: &str = "web_ui_release";

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexDoc {
    profiles: Vec<WebUiIndexProfileRef>,
    #[serde(default)]
    defaults: Option<WebUiIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexDefaults {
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexProfileRef {
    id: String,
    path: String,
}

pub fn cmd_web_ui_profile_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WebUiProfileValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut profiles_status: Vec<Value> = Vec::new();

    if let Some(file) = &args.profile_file {
        let profile_status =
            validate_profile_file(&store, file, None, &mut meta, &mut diagnostics)?;
        profiles_status.push(profile_status);

        let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
        let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
        let report_doc = json!({
          "schema_version": "x07.wasm.web_ui.profile.validate.report@0.1.0",
          "command": "x07-wasm.web-ui.profile.validate",
          "ok": ok,
          "exit_code": exit_code,
          "diagnostics": diagnostics,
          "meta": meta,
          "result": {
            "mode": "profile_file_v1",
            "index": null,
            "profiles": profiles_status,
          }
        });
        store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
        return Ok(exit_code);
    }

    let index_path = args.index.clone();
    let mut index_digest = report::meta::FileDigest {
        path: index_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let index_bytes = match std::fs::read(&index_path) {
        Ok(b) => {
            index_digest.sha256 = util::sha256_hex(&b);
            index_digest.bytes_len = b.len() as u64;
            b
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read index {}: {err}", index_path.display()),
            ));
            Vec::new()
        }
    };
    meta.inputs.push(index_digest.clone());

    let mut index_ok = true;
    let mut index_doc_json_ok = true;
    let index_doc: Value = match serde_json::from_slice(&index_bytes) {
        Ok(v) => v,
        Err(err) => {
            index_doc_json_ok = false;
            index_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_INDEX_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index JSON: {err}"),
            ));
            json!(null)
        }
    };

    if index_doc_json_ok {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-arch.web_ui.index.schema.json",
            &index_doc,
        )?);
        if diagnostics.iter().any(|d| d.severity == Severity::Error) {
            index_ok = false;
        }
    }

    let index_parsed: Option<WebUiIndexDoc> = if index_doc_json_ok {
        match serde_json::from_value(index_doc.clone()) {
            Ok(v) => Some(v),
            Err(err) => {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_INDEX_PARSE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse index doc: {err}"),
                ));
                None
            }
        }
    } else {
        None
    };

    if let Some(idx) = index_parsed.as_ref() {
        let mut seen = std::collections::BTreeSet::new();
        for p in &idx.profiles {
            if !seen.insert(p.id.clone()) {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_INDEX_DUPLICATE_PROFILE_ID",
                    Severity::Error,
                    Stage::Parse,
                    format!("duplicate profile id in index: {:?}", p.id),
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
            .or_else(|| Some(DEFAULT_WEB_UI_PROFILE_ID.to_string()));
        if let Some(def) = default_profile_id.as_deref() {
            default_profile_found = idx.profiles.iter().any(|p| p.id == def);
            if !default_profile_found {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_INDEX_DEFAULT_PROFILE_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("default profile id not found in profiles list: {def:?}"),
                ));
            }
        }
    }

    let mode = if !args.profile.is_empty() {
        "profile_id_v1"
    } else {
        "index_v1"
    };

    if let Some(idx) = index_parsed.as_ref() {
        for p in &idx.profiles {
            if !args.profile.is_empty() && !args.profile.iter().any(|x| x == &p.id) {
                continue;
            }
            let profile_path = PathBuf::from(&p.path);
            let status =
                validate_profile_file(&store, &profile_path, Some(p), &mut meta, &mut diagnostics)?;
            profiles_status.push(status);
        }
        if profiles_status.is_empty() && !args.profile.is_empty() {
            index_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_INDEX_PROFILE_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!("profile id not found: {:?}", args.profile),
            ));
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.web_ui.profile.validate.report@0.1.0",
      "command": "x07-wasm.web-ui.profile.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "index": {
          "path": index_path.display().to_string(),
          "ok": index_ok,
          "profiles_count": profiles_count,
          "default_profile_id": default_profile_id,
          "default_profile_found": default_profile_found,
        },
        "profiles": profiles_status,
      }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn validate_profile_file(
    store: &SchemaStore,
    path: &Path,
    idx_ref: Option<&WebUiIndexProfileRef>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Value> {
    let digest = match util::file_digest(path) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read profile {}: {err:#}", path.display()),
            ));
            report::meta::FileDigest {
                path: path.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let bytes = std::fs::read(path).unwrap_or_default();
    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_PROFILE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse profile JSON: {err}"),
            ));
            json!(null)
        }
    };

    let diags = store.validate(
        "https://x07.io/spec/x07-web_ui.profile.schema.json",
        &doc_json,
    )?;
    let schema_valid = diags.is_empty();
    diagnostics.extend(diags);

    let schema_version = doc_json
        .get("schema_version")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let id = doc_json
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let v = doc_json.get("v").and_then(Value::as_u64);

    let mut id_matches_index = true;
    if let Some(idx) = idx_ref {
        if let Some(id) = id.as_deref() {
            if id != idx.id {
                id_matches_index = false;
            }
        } else {
            id_matches_index = false;
        }
    }

    let ok = schema_valid && id_matches_index;

    let profile_ref = match (id.clone(), v) {
        (Some(id), Some(v)) => Some(json!({ "id": id, "v": v })),
        _ => None,
    };

    let _ = digest;
    Ok(json!({
      "ref": profile_ref,
      "path": path.display().to_string(),
      "ok": ok,
      "schema_version": schema_version,
      "schema_valid": schema_valid,
      "id_matches_index": id_matches_index,
    }))
}

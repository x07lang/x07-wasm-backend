use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{DeviceProfileValidateArgs, MachineArgs, Scope};
use crate::device::contracts::{DeviceIndexDoc, DeviceIndexProfileRef, DeviceProfileDoc};
use crate::device::sidecars::load_profile_sidecars;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_DEVICE_PROFILE_ID: &str = "device_dev";

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexDoc {
    profiles: Vec<WebUiIndexProfileRef>,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexProfileRef {
    id: String,
}

pub fn cmd_device_profile_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceProfileValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut profiles_status: Vec<Value> = Vec::new();

    let web_ui_ids = load_web_ui_profile_ids(&mut diagnostics);

    if let Some(file) = &args.profile_file {
        let profile_status = validate_profile_file(
            &store,
            file,
            None,
            &mut meta,
            &mut diagnostics,
            web_ui_ids.as_ref(),
        );
        profiles_status.push(profile_status);

        if args.strict {
            for d in diagnostics.iter_mut() {
                if d.severity == Severity::Warning {
                    d.severity = Severity::Error;
                }
            }
        }

        let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
        let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
        let report_doc = json!({
          "schema_version": "x07.wasm.device.profile.validate.report@0.1.0",
          "command": "x07-wasm.device.profile.validate",
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
            let status = validate_profile_file(
                &store,
                &profile_path,
                Some(p),
                &mut meta,
                &mut diagnostics,
                web_ui_ids.as_ref(),
            );
            profiles_status.push(status);
        }
        if profiles_status.is_empty() && !args.profile.is_empty() {
            index_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_INDEX_PROFILE_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!("profile id not found: {:?}", args.profile),
            ));
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.device.profile.validate.report@0.1.0",
      "command": "x07-wasm.device.profile.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "index": {
          "path": index_digest.map(|d| d.path).unwrap_or_else(|| index_path.display().to_string()),
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
    profile_path: &Path,
    profile_ref: Option<&DeviceIndexProfileRef>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    web_ui_ids: Option<&std::collections::BTreeSet<String>>,
) -> Value {
    let mut digest = report::meta::FileDigest {
        path: profile_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    let bytes = match std::fs::read(profile_path) {
        Ok(b) => {
            digest.sha256 = util::sha256_hex(&b);
            digest.bytes_len = b.len() as u64;
            b
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read device profile {}: {err}",
                    profile_path.display()
                ),
            ));
            Vec::new()
        }
    };
    meta.inputs.push(digest.clone());

    let mut schema_version: Option<String> = None;
    let mut ok = true;
    let mut id_matches_index = true;
    let mut profile_ref_out: Option<Value> = None;

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse device profile JSON: {err}"),
            ));
            json!(null)
        }
    };

    if let Some(sv) = doc_json.get("schema_version").and_then(Value::as_str) {
        schema_version = Some(sv.to_string());
    }

    let mut schema_valid = false;
    if doc_json != Value::Null {
        let schema_diags = match store.validate(
            "https://x07.io/spec/x07-device.profile.schema.json",
            &doc_json,
        ) {
            Ok(v) => v,
            Err(err) => {
                ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_INTERNAL_SCHEMA_VALIDATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                Vec::new()
            }
        };

        if schema_diags.iter().any(|d| d.severity == Severity::Error) {
            ok = false;
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "device profile schema invalid".to_string(),
            );
            d.data.insert("errors".to_string(), json!(schema_diags));
            diagnostics.push(d);
        } else if ok {
            schema_valid = true;

            match serde_json::from_value::<DeviceProfileDoc>(doc_json.clone()) {
                Ok(doc) => {
                    profile_ref_out = Some(json!({ "id": doc.id, "v": doc.v }));

                    if let Some(pref) = profile_ref {
                        if doc.id != pref.id {
                            ok = false;
                            id_matches_index = false;
                            diagnostics.push(Diagnostic::new(
                                "X07WASM_DEVICE_PROFILE_ID_MISMATCH",
                                Severity::Error,
                                Stage::Parse,
                                format!(
                                    "device profile id mismatch: index has {:?}, file has {:?}",
                                    pref.id, doc.id
                                ),
                            ));
                        }
                    }

                    if let Some(ids) = web_ui_ids {
                        if !ids.contains(&doc.ui.web_ui_profile_id) {
                            ok = false;
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_PROFILE_WEB_UI_PROFILE_NOT_FOUND",
                                Severity::Error,
                                Stage::Parse,
                                "web-ui profile id not found in arch/web_ui index".to_string(),
                            );
                            d.data.insert(
                                "web_ui_profile_id".to_string(),
                                json!(doc.ui.web_ui_profile_id),
                            );
                            diagnostics.push(d);
                        }
                    }

                    if !doc.ui.project.is_file() {
                        ok = false;
                        let mut d = Diagnostic::new(
                            "X07WASM_DEVICE_PROFILE_UI_PROJECT_MISSING",
                            Severity::Error,
                            Stage::Parse,
                            "ui project path is missing".to_string(),
                        );
                        d.data.insert(
                            "path".to_string(),
                            json!(doc.ui.project.display().to_string()),
                        );
                        diagnostics.push(d);
                    }

                    let _ = doc.target;

                    if load_profile_sidecars(store, &doc, meta, diagnostics).is_none() {
                        ok = false;
                    }
                }
                Err(err) => {
                    ok = false;
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_PROFILE_PARSE_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!("failed to parse device profile doc: {err}"),
                    ));
                }
            }
        }
    } else {
        ok = false;
    }

    json!({
      "ref": profile_ref_out,
      "path": digest.path,
      "ok": ok,
      "schema_version": schema_version,
      "schema_valid": schema_valid,
      "id_matches_index": id_matches_index,
    })
}

fn load_web_ui_profile_ids(
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<std::collections::BTreeSet<String>> {
    let index_path = PathBuf::from("arch/web_ui/index.x07webui.json");
    let bytes = match std::fs::read(&index_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_WEB_UI_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read web-ui index {}: {err}",
                    index_path.display()
                ),
            ));
            return None;
        }
    };
    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_WEB_UI_INDEX_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse web-ui index JSON: {err}"),
            ));
            return None;
        }
    };

    let parsed: WebUiIndexDoc = match serde_json::from_value(doc_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_WEB_UI_INDEX_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse web-ui index doc: {err}"),
            ));
            return None;
        }
    };

    let mut out = std::collections::BTreeSet::new();
    for p in parsed.profiles {
        out.insert(p.id);
    }
    Some(out)
}

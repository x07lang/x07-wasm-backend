use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{ComponentProfileValidateArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexDoc {
    profiles: Vec<ComponentIndexProfileRef>,
    #[serde(default)]
    defaults: Option<ComponentIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexDefaults {
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexProfileRef {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentProfileDoc {
    id: String,
    v: u64,
}

pub fn cmd_component_profile_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ComponentProfileValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let (index_digest, index_doc_json, index_parsed) =
        match read_index(&store, &args.index, &mut meta, &mut diagnostics) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_COMPONENT_INDEX_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("{err:#}"),
                ));
                let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
                let report_doc = json!({
                  "schema_version": "x07.wasm.component.profile.validate.report@0.1.0",
                  "command": "x07-wasm.component.profile.validate",
                  "ok": false,
                  "exit_code": exit_code,
                  "diagnostics": diagnostics,
                  "meta": meta,
                  "result": {
                    "strict": args.strict,
                    "index": report::meta::FileDigest {
                        path: args.index.display().to_string(),
                        sha256: "0".repeat(64),
                        bytes_len: 0,
                    },
                    "profiles_total": 0,
                    "profiles_ok": 0,
                    "profiles_failed": 0,
                    "profiles": [],
                  }
                });
                store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
                return Ok(exit_code);
            }
        };

    let wanted: Option<BTreeSet<String>> = if args.profile.is_empty() {
        None
    } else {
        Some(args.profile.iter().cloned().collect())
    };

    let mut seen = BTreeSet::new();
    for p in &index_parsed.profiles {
        if !seen.insert(p.id.clone()) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_INDEX_DUPLICATE_PROFILE_ID",
                Severity::Error,
                Stage::Parse,
                format!("duplicate profile id in index: {:?}", p.id),
            ));
        }
    }

    let default_profile_id = index_parsed
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone());
    if let Some(def) = default_profile_id.as_deref() {
        if !index_parsed.profiles.iter().any(|p| p.id == def) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_INDEX_DEFAULT_PROFILE_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!("default_profile_id not found in index: {:?}", def),
            ));
        }
    }

    let mut profiles_status: Vec<Value> = Vec::new();
    let mut profiles_ok = 0u32;
    let mut profiles_failed = 0u32;

    let base_dir = Path::new(".");
    for p in &index_parsed.profiles {
        if let Some(w) = wanted.as_ref() {
            if !w.contains(&p.id) {
                continue;
            }
        }

        let status = validate_profile_file(&store, base_dir, p, &mut meta, &mut diagnostics);
        let ok = status.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if ok {
            profiles_ok += 1;
        } else {
            profiles_failed += 1;
        }
        profiles_status.push(status);
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
      "schema_version": "x07.wasm.component.profile.validate.report@0.1.0",
      "command": "x07-wasm.component.profile.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "strict": args.strict,
        "index": index_digest,
        "profiles_total": (profiles_ok + profiles_failed),
        "profiles_ok": profiles_ok,
        "profiles_failed": profiles_failed,
        "profiles": profiles_status,
      }
    });

    let _ = index_doc_json;
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn read_index(
    store: &SchemaStore,
    index_path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(report::meta::FileDigest, Value, ComponentIndexDoc)> {
    let digest = util::file_digest(index_path)?;
    meta.inputs.push(digest.clone());

    let bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;

    diagnostics.extend(store.validate(
        "https://x07.io/spec/x07-arch.wasm.component.index.schema.json",
        &doc_json,
    )?);

    let doc: ComponentIndexDoc =
        serde_json::from_value(doc_json.clone()).context("parse component index")?;
    Ok((digest, doc_json, doc))
}

fn validate_profile_file(
    store: &SchemaStore,
    base_dir: &Path,
    p: &ComponentIndexProfileRef,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Value {
    let path = base_dir.join(&p.path);
    let mut ok = true;
    let mut schema_valid = false;
    let mut v: Option<u64> = None;

    match util::file_digest(&path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read profile {}: {err:#}", path.display()),
            ));
            ok = false;
        }
    }

    let doc_json: Value = if ok {
        match std::fs::read(&path) {
            Ok(b) => match serde_json::from_slice(&b) {
                Ok(v) => v,
                Err(err) => {
                    ok = false;
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_COMPONENT_PROFILE_JSON_INVALID",
                        Severity::Error,
                        Stage::Parse,
                        format!("failed to parse profile JSON {}: {err}", path.display()),
                    ));
                    json!(null)
                }
            },
            Err(_) => json!(null),
        }
    } else {
        json!(null)
    };

    if ok {
        let diags = store.validate(
            "https://x07.io/spec/x07-wasm.component.profile.schema.json",
            &doc_json,
        );
        match diags {
            Ok(ds) => {
                if ds.is_empty() {
                    schema_valid = true;
                } else {
                    diagnostics.extend(ds);
                }
            }
            Err(err) => {
                ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SCHEMA_VALIDATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
            }
        }
    }

    let mut id_matches_index = false;
    if schema_valid {
        match serde_json::from_value::<ComponentProfileDoc>(doc_json.clone()) {
            Ok(doc) => {
                v = Some(doc.v);
                id_matches_index = doc.id == p.id;
                if !id_matches_index {
                    ok = false;
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_COMPONENT_PROFILE_ID_MISMATCH",
                        Severity::Error,
                        Stage::Parse,
                        format!(
                            "profile id mismatch: index has {:?} but file declares {:?}",
                            p.id, doc.id
                        ),
                    ));
                }
            }
            Err(err) => {
                ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_COMPONENT_PROFILE_PARSE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse profile {}: {err}", path.display()),
                ));
            }
        }
    } else {
        ok = false;
    }

    json!({
      "ref": { "id": p.id, "v": v.unwrap_or(1) },
      "path": p.path,
      "ok": ok,
      "id_matches_index": id_matches_index,
      "schema_valid": schema_valid,
    })
}

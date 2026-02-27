use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, OpsValidateArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::ops::{load_ops_profile_with_refs, LoadedOpsProfile};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
struct OpsIndexDoc {
    #[serde(default)]
    defaults: Option<OpsIndexDefaults>,
    profiles: Vec<OpsIndexProfileRef>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpsIndexDefaults {
    #[serde(default)]
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpsIndexProfileRef {
    id: String,
    path: String,
}

pub fn cmd_ops_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: OpsValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let ops_profile_path =
        match resolve_ops_profile_path(&store, &args, &mut meta, &mut diagnostics) {
            Some(p) => p,
            None => {
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    args.index.display().to_string(),
                    None,
                );
            }
        };

    let loaded =
        load_ops_profile_with_refs(&store, &ops_profile_path, &mut meta, &mut diagnostics)?;

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        ops_profile_path.display().to_string(),
        loaded,
    )
}

fn resolve_ops_profile_path(
    store: &SchemaStore,
    args: &OpsValidateArgs,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    if let Some(p) = args.profile.as_ref() {
        return Some(p.clone());
    }

    let index_path = &args.index;
    let bytes = match std::fs::read(index_path) {
        Ok(v) => {
            meta.inputs.push(report::meta::FileDigest {
                path: index_path.display().to_string(),
                sha256: util::sha256_hex(&v),
                bytes_len: v.len() as u64,
            });
            v
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read ops index {}: {err}", index_path.display()),
            ));
            return None;
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("ops index JSON invalid: {err}"),
            ));
            return None;
        }
    };

    let schema_diags = match store.validate(
        "https://x07.io/spec/x07-arch.app.ops.index.schema.json",
        &doc_json,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_INTERNAL_SCHEMA_VALIDATE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            Vec::new()
        }
    };
    if !schema_diags.is_empty() {
        for dd in schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_OPS_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return None;
    }

    let parsed: OpsIndexDoc = match serde_json::from_value(doc_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse ops index: {err}"),
            ));
            return None;
        }
    };

    let profile_id = if let Some(id) = args.profile_id.as_ref() {
        id.clone()
    } else if let Some(id) = parsed
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
    {
        id
    } else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_OPS_PROFILE_READ_FAILED",
            Severity::Error,
            Stage::Parse,
            "no --profile provided and ops index has no defaults.default_profile_id".to_string(),
        ));
        return None;
    };

    let resolved = parsed
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .map(|p| PathBuf::from(p.path.clone()));

    match resolved {
        Some(p) => Some(p),
        None => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_PROFILE_ID_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!("profile id not found in ops index: {profile_id:?}"),
            ));
            None
        }
    }
}

fn file_status(path: &Path, ok: bool, schema_valid: bool, sha256: Option<String>) -> Value {
    json!({
      "path": path.display().to_string(),
      "ok": ok,
      "schema_valid": schema_valid,
      "sha256": sha256,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    ops_profile_path: String,
    loaded: Option<LoadedOpsProfile>,
) -> Result<u8> {
    let (ops_status, caps_status, policy_cards_status, slo_status, compatibility_hash) =
        if let Some(l) = loaded.as_ref() {
            let ops_status = file_status(
                &l.ops.path,
                l.ops.ok,
                l.ops.schema_valid,
                Some(l.ops.digest.sha256.clone()),
            );
            let caps_status = file_status(
                &l.capabilities.path,
                l.capabilities.ok,
                l.capabilities.schema_valid,
                Some(l.capabilities.digest.sha256.clone()),
            );
            let policy_cards_status: Vec<Value> = l
                .policy_cards
                .iter()
                .map(|c| file_status(&c.path, c.ok, c.schema_valid, Some(c.digest.sha256.clone())))
                .collect();
            let slo_status = l
                .slo_profile
                .as_ref()
                .map(|s| file_status(&s.path, s.ok, s.schema_valid, Some(s.digest.sha256.clone())));

            let compat_doc = json!({
              "ops_profile_sha256": l.ops.digest.sha256,
              "capabilities_sha256": l.capabilities.digest.sha256,
              "policy_cards_sha256": l.policy_cards.iter().map(|c| c.digest.sha256.clone()).collect::<Vec<_>>(),
              "slo_profile_sha256": l.slo_profile.as_ref().map(|s| s.digest.sha256.clone()),
            });
            let bytes = report::canon::canonical_json_bytes(&compat_doc)?;
            let compatibility_hash = util::sha256_hex(&bytes);

            (
                ops_status,
                caps_status,
                policy_cards_status,
                slo_status,
                compatibility_hash,
            )
        } else {
            (
                json!({
                  "path": ops_profile_path,
                  "ok": false,
                  "schema_valid": false,
                  "sha256": null,
                }),
                json!({
                  "path": "missing",
                  "ok": false,
                  "schema_valid": false,
                  "sha256": null,
                }),
                Vec::new(),
                None,
                "0".repeat(64),
            )
        };

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.ops.validate.report@0.1.0",
      "command": "x07-wasm.ops.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "ops_profile": ops_status,
        "capabilities": caps_status,
        "policy_cards": policy_cards_status,
        "slo_profile": slo_status,
        "compatibility_hash": compatibility_hash,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

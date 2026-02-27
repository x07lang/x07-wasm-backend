use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{MachineArgs, PolicyValidateArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_policy_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: PolicyValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let card_paths = discover_card_paths(&args, &mut diagnostics)?;
    let mut cards_status: Vec<Value> = Vec::new();
    let mut card_digests: Vec<Value> = Vec::new();

    for path in card_paths {
        let (status, digest_opt) = validate_card(&store, &path, &mut meta, &mut diagnostics)?;
        if let Some(d) = digest_opt {
            card_digests
                .push(json!({"path": d.path, "sha256": d.sha256, "bytes_len": d.bytes_len}));
        }
        cards_status.push(status);
    }

    if args.strict
        && cards_status
            .iter()
            .any(|s| !s.get("ok").and_then(Value::as_bool).unwrap_or(false))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_POLICY_STRICT_FAILED",
            Severity::Error,
            Stage::Lint,
            "one or more policy cards failed validation".to_string(),
        ));
    }

    let compat_doc = json!({ "cards": card_digests });
    let bytes = report::canon::canonical_json_bytes(&compat_doc)?;
    let compatibility_hash = util::sha256_hex(&bytes);

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.policy.validate.report@0.1.0",
      "command": "x07-wasm.policy.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "cards": cards_status,
        "compatibility_hash": compatibility_hash,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn discover_card_paths(
    args: &PolicyValidateArgs,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    out.extend(args.card.iter().cloned());

    if let Some(dir) = args.cards_dir.as_ref() {
        let entries =
            std::fs::read_dir(dir).with_context(|| format!("read_dir: {}", dir.display()));
        match entries {
            Ok(rd) => {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.is_file() && p.extension().is_some_and(|e| e == "json") {
                        out.push(p);
                    }
                }
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_POLICY_CARDS_DIR_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("{err:#}"),
                ));
            }
        }
    }

    out.sort_by_key(|a| a.display().to_string());
    out.dedup();
    Ok(out)
}

fn validate_card(
    store: &SchemaStore,
    path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Value, Option<report::meta::FileDigest>)> {
    let diag_sev = Severity::Warning;

    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_POLICY_CARD_READ_FAILED",
                diag_sev,
                Stage::Parse,
                format!("failed to read policy card {}: {err}", path.display()),
            ));
            let status = json!({
              "path": path.display().to_string(),
              "ok": false,
              "schema_valid": false,
              "sha256": null,
            });
            return Ok((status, None));
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
                "X07WASM_POLICY_SCHEMA_INVALID",
                diag_sev,
                Stage::Parse,
                format!("policy card JSON invalid: {err}"),
            ));
            let status = json!({
              "path": digest.path,
              "ok": false,
              "schema_valid": false,
              "sha256": digest.sha256,
            });
            return Ok((status, Some(digest)));
        }
    };

    let schema_diags =
        store.validate("https://x07.io/spec/x07-policy.card.schema.json", &doc_json)?;
    let schema_valid = schema_diags.is_empty();
    if !schema_valid {
        for dd in schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_POLICY_SCHEMA_INVALID",
                diag_sev,
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

    Ok((status, Some(digest)))
}

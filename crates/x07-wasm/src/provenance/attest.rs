use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, ProvenanceAttestArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::ops::load_ops_profile_with_refs;
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_provenance_attest(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ProvenanceAttestArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = true;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let pack_bytes = match std::fs::read(&args.pack_manifest) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_INPUT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read pack manifest {}: {err}",
                    args.pack_manifest.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.pack_manifest,
                None,
            );
        }
    };

    let pack_digest = report::meta::FileDigest {
        path: args.pack_manifest.display().to_string(),
        sha256: util::sha256_hex(&pack_bytes),
        bytes_len: pack_bytes.len() as u64,
    };
    meta.inputs.push(pack_digest.clone());

    let pack_doc: Value = match serde_json::from_slice(&pack_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("pack manifest JSON invalid: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.pack_manifest,
                None,
            );
        }
    };

    let pack_schema_diags =
        store.validate("https://x07.io/spec/x07-app.pack.schema.json", &pack_doc)?;
    if !pack_schema_diags.is_empty() {
        for dd in pack_schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_PROVENANCE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.pack_manifest,
            None,
        );
    }

    let pack_dir = args
        .pack_manifest
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let loaded_ops = load_ops_profile_with_refs(&store, &args.ops, &mut meta, &mut diagnostics)?;
    let Some(loaded_ops) = loaded_ops else {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.pack_manifest,
            None,
        );
    };

    let mut subjects: Vec<Value> = Vec::new();

    let pack_name = rel_path_or_fallback(&pack_dir, &args.pack_manifest, "app.pack.json");
    subjects.push(json!({
      "name": pack_name,
      "digest": { "sha256": pack_digest.sha256 },
      "mediaType": "application/json",
    }));

    let assets = pack_doc
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for a in assets {
        let Some(fp) = a
            .get("file")
            .and_then(|f| f.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };

        let asset_path = pack_dir.join(fp);
        let bytes = match std::fs::read(&asset_path) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_PROVENANCE_MISSING_INPUT",
                    Severity::Error,
                    Stage::Parse,
                    format!("missing pack asset {}: {err}", asset_path.display()),
                ));
                continue;
            }
        };
        let sha256 = util::sha256_hex(&bytes);
        meta.inputs.push(report::meta::FileDigest {
            path: asset_path.display().to_string(),
            sha256: sha256.clone(),
            bytes_len: bytes.len() as u64,
        });
        subjects.push(json!({
          "name": fp,
          "digest": { "sha256": sha256 },
        }));
    }
    subjects.sort_by_key(|s| {
        s.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });

    let mut resolved_deps: Vec<Value> = Vec::new();
    resolved_deps.push(resource_descriptor(&loaded_ops.ops.digest));
    resolved_deps.push(resource_descriptor(&loaded_ops.capabilities.digest));
    for c in loaded_ops.policy_cards.iter() {
        resolved_deps.push(resource_descriptor(&c.digest));
    }
    if let Some(s) = loaded_ops.slo_profile.as_ref() {
        resolved_deps.push(resource_descriptor(&s.digest));
    }

    let started_on = rfc3339_utc_now();
    let finished_on = rfc3339_utc_now();

    let compatibility_hash = crate::ops::compute_ops_compatibility_hash(&loaded_ops)?;
    let mut x07_pred = json!({
      "pack_manifest_sha256": pack_digest.sha256,
      "ops_profile_sha256": loaded_ops.ops.digest.sha256,
      "capabilities_sha256": loaded_ops.capabilities.digest.sha256,
      "compatibility_hash": compatibility_hash,
    });
    if let Some(s) = loaded_ops.slo_profile.as_ref() {
        x07_pred.as_object_mut().unwrap().insert(
            "slo_profile_sha256".to_string(),
            json!(s.digest.sha256.clone()),
        );
    }

    let attestation_doc = json!({
      "_type": "https://in-toto.io/Statement/v1",
      "subject": subjects,
      "predicateType": "https://slsa.dev/provenance/v1",
      "predicate": {
        "buildDefinition": {
          "buildType": "x07-wasm.provenance.attest@0.1.0",
          "externalParameters": {},
          "internalParameters": {},
          "resolvedDependencies": resolved_deps,
        },
        "runDetails": {
          "builder": { "id": format!("x07-wasm@{}", env!("CARGO_PKG_VERSION")) },
          "metadata": {
            "startedOn": started_on,
            "finishedOn": finished_on,
          }
        },
        "x07": x07_pred
      }
    });

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-provenance.slsa.attestation.schema.json",
        &attestation_doc,
    )?;
    if !schema_diags.is_empty() {
        for dd in schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_PROVENANCE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.pack_manifest,
            None,
        );
    }

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let attestation_bytes = report::canon::canonical_pretty_json_bytes(&attestation_doc)?;
    if let Err(err) = std::fs::write(&args.out, &attestation_bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_PROVENANCE_ATTEST_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to write attestation {}: {err}", args.out.display()),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.pack_manifest,
            None,
        );
    }

    let attestation_digest = util::file_digest(&args.out)?;
    meta.outputs.push(attestation_digest.clone());

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args.pack_manifest,
        Some(attestation_digest),
    )
}

fn resource_descriptor(d: &report::meta::FileDigest) -> Value {
    json!({
      "uri": d.path,
      "digest": { "sha256": d.sha256 },
    })
}

fn rel_path_or_fallback(root: &Path, path: &Path, fallback: &str) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| fallback.to_string())
}

fn rfc3339_utc_now() -> String {
    let dt = time::OffsetDateTime::now_utc();
    let date = dt.date().to_string();
    let time = dt.time().to_string();
    let main = time.split('.').next().unwrap_or(time.as_str());
    format!("{date}T{main}Z")
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
    pack_manifest_path: &Path,
    attestation: Option<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let pack_manifest = util::file_digest(pack_manifest_path).unwrap_or(report::meta::FileDigest {
        path: pack_manifest_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    });
    let attestation = attestation.unwrap_or(report::meta::FileDigest {
        path: pack_manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("provenance.slsa.json")
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.provenance.attest.report@0.1.0",
      "command": "x07-wasm.provenance.attest",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "pack_manifest": pack_manifest,
        "attestation": attestation,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

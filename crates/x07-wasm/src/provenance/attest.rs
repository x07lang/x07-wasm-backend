use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, ProvenanceAttestArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::ops::load_ops_profile_with_refs;
use crate::provenance::dsse;
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

    // Fail-closed invariant: remove any stale output before we start so errors cannot leave
    // a usable DSSE envelope behind.
    let out_tmp: PathBuf = util::preunlink_out(&args.out);

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

    if let Some(bundle) = pack_doc.get("bundle_manifest") {
        let rel = bundle.get("path").and_then(Value::as_str).unwrap_or("");
        if !rel.is_empty() {
            let bundle_path = match util::safe_join_under_dir(&pack_dir, rel) {
                Ok(v) => Some(v),
                Err(err) => {
                    let mut d = Diagnostic::new(
                        "X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE",
                        Severity::Error,
                        Stage::Run,
                        format!("unsafe subject path: {rel:?}"),
                    );
                    d.data.insert("subject".to_string(), json!(rel.to_string()));
                    d.data.insert("path".to_string(), json!(err.rel));
                    d.data.insert("kind".to_string(), json!(err.kind));
                    d.data.insert("detail".to_string(), json!(err.detail));
                    diagnostics.push(d);
                    None
                }
            };
            if let Some(bundle_path) = bundle_path.as_ref() {
                let digest = match util::file_digest(bundle_path) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        let mut d = Diagnostic::new(
                            "X07WASM_PROVENANCE_SUBJECT_MISSING",
                            Severity::Error,
                            Stage::Run,
                            format!("missing subject file {}: {err:#}", bundle_path.display()),
                        );
                        d.data.insert("subject".to_string(), json!(rel.to_string()));
                        diagnostics.push(d);
                        None
                    }
                };
                if let Some(digest) = digest {
                    meta.inputs.push(digest.clone());
                    subjects.push(json!({
                      "name": rel,
                      "digest": { "sha256": digest.sha256 },
                      "mediaType": "application/json",
                    }));
                }
            }
        }
    }

    if let Some(component) = pack_doc
        .get("backend")
        .and_then(|b| b.get("component"))
        .and_then(Value::as_object)
    {
        let rel = component.get("path").and_then(Value::as_str).unwrap_or("");
        if !rel.is_empty() {
            let component_path = match util::safe_join_under_dir(&pack_dir, rel) {
                Ok(v) => Some(v),
                Err(err) => {
                    let mut d = Diagnostic::new(
                        "X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE",
                        Severity::Error,
                        Stage::Run,
                        format!("unsafe subject path: {rel:?}"),
                    );
                    d.data.insert("subject".to_string(), json!(rel.to_string()));
                    d.data.insert("path".to_string(), json!(err.rel));
                    d.data.insert("kind".to_string(), json!(err.kind));
                    d.data.insert("detail".to_string(), json!(err.detail));
                    diagnostics.push(d);
                    None
                }
            };
            if let Some(component_path) = component_path.as_ref() {
                let digest = match util::file_digest(component_path) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        let mut d = Diagnostic::new(
                            "X07WASM_PROVENANCE_SUBJECT_MISSING",
                            Severity::Error,
                            Stage::Run,
                            format!("missing subject file {}: {err:#}", component_path.display()),
                        );
                        d.data.insert("subject".to_string(), json!(rel.to_string()));
                        diagnostics.push(d);
                        None
                    }
                };
                if let Some(digest) = digest {
                    meta.inputs.push(digest.clone());
                    subjects.push(json!({
                      "name": rel,
                      "digest": { "sha256": digest.sha256 },
                      "mediaType": "application/wasm",
                    }));
                }
            }
        }
    }

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

        let asset_path = match util::safe_join_under_dir(&pack_dir, fp) {
            Ok(v) => v,
            Err(err) => {
                let mut d = Diagnostic::new(
                    "X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE",
                    Severity::Error,
                    Stage::Run,
                    format!("unsafe subject path: {fp:?}"),
                );
                d.data.insert("subject".to_string(), json!(fp.to_string()));
                d.data.insert("path".to_string(), json!(err.rel));
                d.data.insert("kind".to_string(), json!(err.kind));
                d.data.insert("detail".to_string(), json!(err.detail));
                diagnostics.push(d);
                continue;
            }
        };
        let digest = match util::file_digest(&asset_path) {
            Ok(v) => v,
            Err(err) => {
                let mut d = Diagnostic::new(
                    "X07WASM_PROVENANCE_SUBJECT_MISSING",
                    Severity::Error,
                    Stage::Run,
                    format!("missing subject file {}: {err:#}", asset_path.display()),
                );
                d.data.insert("subject".to_string(), json!(fp.to_string()));
                diagnostics.push(d);
                continue;
            }
        };
        meta.inputs.push(digest.clone());
        subjects.push(json!({
          "name": fp,
          "digest": { "sha256": digest.sha256 },
        }));
    }
    subjects.sort_by_key(|s| {
        s.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });

    // Fail closed: do not emit a DSSE envelope when any Error diag exists.
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        let _ = std::fs::remove_file(&args.out);
        let _ = std::fs::remove_file(&out_tmp);
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.pack_manifest,
            Some(report::meta::FileDigest {
                path: args.out.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }),
        );
    }

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
      "predicateType": args.predicate_type.clone(),
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

    let signing_key_seed_b64 = match std::fs::read_to_string(&args.signing_key) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_SIGNING_KEY_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read signing key {}: {err}",
                    args.signing_key.display()
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
    let signing_key_seed = match STANDARD.decode(signing_key_seed_b64.trim()) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_SIGNING_KEY_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("signing key base64 decode failed: {err}"),
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
    let signing_key_seed: [u8; 32] = match signing_key_seed.try_into() {
        Ok(v) => v,
        Err(_) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_SIGNING_KEY_INVALID",
                Severity::Error,
                Stage::Parse,
                "signing key seed must be exactly 32 bytes".to_string(),
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
    let signing_key = SigningKey::from_bytes(&signing_key_seed);
    let public_key = signing_key.verifying_key().to_bytes();
    let keyid = Some(util::sha256_hex(&public_key));

    let payload_bytes = report::canon::canonical_json_bytes(&attestation_doc)?;
    let envelope = dsse::sign_ed25519_envelope(
        dsse::IN_TOTO_STATEMENT_PAYLOAD_TYPE,
        &payload_bytes,
        &signing_key,
        keyid,
    );
    let envelope_doc = serde_json::to_value(&envelope)?;
    let envelope_schema_diags = store.validate(
        "https://x07.io/spec/x07-provenance.dsse.envelope.schema.json",
        &envelope_doc,
    )?;
    if !envelope_schema_diags.is_empty() {
        for dd in envelope_schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_PROVENANCE_DSSE_SCHEMA_INVALID",
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
    let attestation_bytes = report::canon::canonical_pretty_json_bytes(&envelope_doc)?;
    if let Err(err) = util::write_file_atomic(&args.out, &attestation_bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_PROVENANCE_ATTEST_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to write attestation {} -> {}: {err}",
                out_tmp.display(),
                args.out.display()
            ),
        ));
        let _ = std::fs::remove_file(&args.out);
        let _ = std::fs::remove_file(&out_tmp);
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

    let attestation_digest = report::meta::FileDigest {
        path: args.out.display().to_string(),
        sha256: util::sha256_hex(&attestation_bytes),
        bytes_len: attestation_bytes.len() as u64,
    };
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
            .join("provenance.dsse.json")
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

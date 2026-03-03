use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, ProvenanceVerifyArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::provenance::dsse;
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_provenance_verify(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ProvenanceVerifyArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let attest_bytes = match std::fs::read(&args.attestation) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_INPUT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read attestation {}: {err}",
                    args.attestation.display()
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
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };

    let attest_digest = report::meta::FileDigest {
        path: args.attestation.display().to_string(),
        sha256: util::sha256_hex(&attest_bytes),
        bytes_len: attest_bytes.len() as u64,
    };
    meta.inputs.push(attest_digest.clone());

    let envelope_doc: Value = match serde_json::from_slice(&attest_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_DSSE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("attestation JSON invalid: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-provenance.dsse.envelope.schema.json",
        &envelope_doc,
    )?;
    if !schema_diags.is_empty() {
        for dd in schema_diags {
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
            &args.attestation,
            0,
            0,
            1,
        );
    }

    let envelope: dsse::DsseEnvelope = match serde_json::from_value(envelope_doc) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_DSSE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse DSSE envelope: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };

    let trusted_public_key_b64 = match std::fs::read_to_string(&args.trusted_public_key) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_PUBLIC_KEY_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read trusted public key {}: {err}",
                    args.trusted_public_key.display()
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
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };
    let trusted_public_key = match STANDARD.decode(trusted_public_key_b64.trim()) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_PUBLIC_KEY_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("trusted public key base64 decode failed: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };
    let trusted_public_key: [u8; 32] = match trusted_public_key.try_into() {
        Ok(v) => v,
        Err(_) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_PUBLIC_KEY_INVALID",
                Severity::Error,
                Stage::Parse,
                "trusted public key must be exactly 32 bytes".to_string(),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };
    let trusted_public_key = match VerifyingKey::from_bytes(&trusted_public_key) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_PUBLIC_KEY_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("trusted public key invalid: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };

    if let Err(()) = dsse::verify_ed25519_signature(&envelope, &trusted_public_key) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_PROVENANCE_SIGNATURE_INVALID",
            Severity::Error,
            Stage::Run,
            "DSSE signature verification failed".to_string(),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.attestation,
            0,
            0,
            1,
        );
    }

    if envelope.payload_type != dsse::IN_TOTO_STATEMENT_PAYLOAD_TYPE {
        diagnostics.push(Diagnostic::new(
            "X07WASM_PROVENANCE_SCHEMA_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unsupported DSSE payloadType: {:?}", envelope.payload_type),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.attestation,
            0,
            0,
            1,
        );
    }

    let payload_bytes = match dsse::decode_payload(&envelope) {
        Ok(v) => v,
        Err(()) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_DSSE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "failed to decode DSSE payload base64".to_string(),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };
    let attest_doc: Value = match serde_json::from_slice(&payload_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROVENANCE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("attestation payload JSON invalid: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.attestation,
                0,
                0,
                1,
            );
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-provenance.slsa.attestation.schema.json",
        &attest_doc,
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
            &args.attestation,
            0,
            0,
            1,
        );
    }

    let predicate_type = attest_doc
        .get("predicateType")
        .and_then(Value::as_str)
        .unwrap_or("");
    if predicate_type != "https://slsa.dev/provenance/v1" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_PROVENANCE_PREDICATE_TYPE_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!("unsupported predicateType: {predicate_type:?}"),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.attestation,
            0,
            0,
            1,
        );
    }

    let subjects = attest_doc
        .get("subject")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut subjects_checked: u64 = 0;
    let mut subjects_mismatched: u64 = 0;

    for s in subjects {
        let Some(name) = s.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(want) = s
            .get("digest")
            .and_then(|d| d.get("sha256"))
            .and_then(Value::as_str)
        else {
            continue;
        };

        subjects_checked += 1;

        let full = match util::safe_join_under_dir(&args.pack_dir, name) {
            Ok(v) => v,
            Err(err) => {
                subjects_mismatched += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_PROVENANCE_SUBJECT_PATH_UNSAFE",
                    Severity::Error,
                    Stage::Run,
                    format!("unsafe subject path: {name:?}"),
                );
                d.data
                    .insert("subject".to_string(), json!(name.to_string()));
                d.data.insert("path".to_string(), json!(err.rel));
                d.data.insert("kind".to_string(), json!(err.kind));
                d.data.insert("detail".to_string(), json!(err.detail));
                diagnostics.push(d);
                continue;
            }
        };
        let bytes = match std::fs::read(&full) {
            Ok(v) => v,
            Err(err) => {
                subjects_mismatched += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_PROVENANCE_SUBJECT_MISSING",
                    Severity::Error,
                    Stage::Run,
                    format!("missing subject file {}: {err}", full.display()),
                );
                d.data
                    .insert("subject".to_string(), json!(name.to_string()));
                diagnostics.push(d);
                continue;
            }
        };
        let got = util::sha256_hex(&bytes);
        meta.inputs.push(report::meta::FileDigest {
            path: full.display().to_string(),
            sha256: got.clone(),
            bytes_len: bytes.len() as u64,
        });
        if got != want {
            subjects_mismatched += 1;
            let mut d = Diagnostic::new(
                "X07WASM_PROVENANCE_DIGEST_MISMATCH",
                Severity::Error,
                Stage::Run,
                format!("digest mismatch for subject {name:?}"),
            );
            d.data
                .insert("subject".to_string(), json!(name.to_string()));
            d.data.insert("expected_sha256".to_string(), json!(want));
            d.data.insert("actual_sha256".to_string(), json!(got));
            diagnostics.push(d);
        }
    }

    let exit_code = if subjects_mismatched > 0 { 1u8 } else { 0u8 };

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args.attestation,
        subjects_checked,
        subjects_mismatched,
        exit_code,
    )
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
    attestation_path: &Path,
    subjects_checked: u64,
    subjects_mismatched: u64,
    exit_code: u8,
) -> Result<u8> {
    let report_doc = json!({
      "schema_version": "x07.wasm.provenance.verify.report@0.1.0",
      "command": "x07-wasm.provenance.verify",
      "ok": exit_code == 0,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "attestation": util::file_digest(attestation_path).unwrap_or(report::meta::FileDigest {
          path: attestation_path.display().to_string(),
          sha256: "0".repeat(64),
          bytes_len: 0,
        }),
        "subjects_checked": subjects_checked,
        "subjects_mismatched": subjects_mismatched,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

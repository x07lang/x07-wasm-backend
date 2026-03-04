use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde_json::{json, Value};

use crate::cli::{DeviceProvenanceAttestArgs, DeviceProvenanceVerifyArgs, MachineArgs, Scope};
use crate::device::contracts::DeviceBundleManifestDoc;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::provenance::dsse;
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_BUNDLE_MANIFEST_FILE: &str = "bundle.manifest.json";

pub fn cmd_device_provenance_attest(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceProvenanceAttestArgs,
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
    let out_tmp = util::preunlink_out(&args.out);

    let bundle_dir = args.bundle_dir.clone();
    let manifest_path = bundle_dir.join(DEVICE_BUNDLE_MANIFEST_FILE);

    let manifest_bytes = match std::fs::read(&manifest_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read bundle manifest {}: {err}",
                    manifest_path.display()
                ),
            ));
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
                None,
            );
        }
    };

    let manifest_digest = report::meta::FileDigest {
        path: manifest_path.display().to_string(),
        sha256: util::sha256_hex(&manifest_bytes),
        bytes_len: manifest_bytes.len() as u64,
    };
    meta.inputs.push(manifest_digest.clone());

    let manifest_json: Value = match serde_json::from_slice(&manifest_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("bundle manifest is not JSON: {err}"),
            ));
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
                None,
            );
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-device.bundle.manifest.schema.json",
        &manifest_json,
    )?;
    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_BUNDLE_MANIFEST_SCHEMA_INVALID",
            Severity::Error,
            Stage::Parse,
            "device bundle manifest schema invalid".to_string(),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        diagnostics.push(d);
        return emit_attest_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &manifest_path,
            None,
        );
    }

    let doc: DeviceBundleManifestDoc = match serde_json::from_value(manifest_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse device bundle manifest doc: {err}"),
            ));
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
                None,
            );
        }
    };

    let mut subjects: Vec<Value> = Vec::new();
    subjects.push(json!({
      "name": DEVICE_BUNDLE_MANIFEST_FILE,
      "digest": { "sha256": manifest_digest.sha256 },
      "mediaType": "application/json",
    }));

    let subject_specs = [
        (
            "ui_wasm.path",
            doc.ui_wasm.path,
            doc.ui_wasm.sha256,
            doc.ui_wasm.bytes_len,
            "application/wasm",
        ),
        (
            "profile.file.path",
            doc.profile.file.path,
            doc.profile.file.sha256,
            doc.profile.file.bytes_len,
            "application/json",
        ),
    ];

    for (field, rel, want_sha, want_len, media_type) in subject_specs {
        let full = match util::safe_join_under_dir(&bundle_dir, &rel) {
            Ok(v) => v,
            Err(err) => {
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_PATH_UNSAFE",
                    Severity::Error,
                    Stage::Parse,
                    "unsafe bundle path".to_string(),
                );
                d.data.insert("field".to_string(), json!(field));
                d.data.insert("path".to_string(), json!(err.rel));
                d.data.insert("kind".to_string(), json!(err.kind));
                d.data.insert("detail".to_string(), json!(err.detail));
                diagnostics.push(d);
                return emit_attest_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    &manifest_path,
                    None,
                );
            }
        };

        let bytes = match std::fs::read(&full) {
            Ok(v) => v,
            Err(err) => {
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("missing bundle file {}: {err}", full.display()),
                );
                d.data.insert("path".to_string(), json!(rel.clone()));
                diagnostics.push(d);
                return emit_attest_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    &manifest_path,
                    None,
                );
            }
        };

        let got_sha = util::sha256_hex(&bytes);
        let got_len = bytes.len() as u64;
        meta.inputs.push(report::meta::FileDigest {
            path: full.display().to_string(),
            sha256: got_sha.clone(),
            bytes_len: got_len,
        });
        if got_sha != want_sha || got_len != want_len {
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH",
                Severity::Error,
                Stage::Parse,
                "bundle file digest mismatch".to_string(),
            );
            d.data.insert("path".to_string(), json!(rel.clone()));
            d.data.insert("want_sha256".to_string(), json!(want_sha));
            d.data.insert("got_sha256".to_string(), json!(got_sha));
            d.data.insert("want_bytes_len".to_string(), json!(want_len));
            d.data.insert("got_bytes_len".to_string(), json!(got_len));
            diagnostics.push(d);
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
                None,
            );
        }

        subjects.push(json!({
          "name": rel,
          "digest": { "sha256": got_sha },
          "mediaType": media_type,
        }));
    }

    subjects.sort_by_key(|s| {
        s.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });

    let started_on = rfc3339_utc_now();
    let finished_on = rfc3339_utc_now();

    let attestation_doc = json!({
      "_type": "https://in-toto.io/Statement/v1",
      "subject": subjects,
      "predicateType": args.predicate_type.clone(),
      "predicate": {
        "buildDefinition": {
          "buildType": "x07-wasm.device.provenance.attest@0.1.0",
          "externalParameters": {},
          "internalParameters": {},
          "resolvedDependencies": [],
        },
        "runDetails": {
          "builder": { "id": format!("x07-wasm@{}", env!("CARGO_PKG_VERSION")) },
          "metadata": {
            "startedOn": started_on,
            "finishedOn": finished_on,
          }
        },
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
        return emit_attest_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &manifest_path,
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
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
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
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
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
            return emit_attest_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &manifest_path,
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
        return emit_attest_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &manifest_path,
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
        return emit_attest_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &manifest_path,
            None,
        );
    }

    let attestation_digest = report::meta::FileDigest {
        path: args.out.display().to_string(),
        sha256: util::sha256_hex(&attestation_bytes),
        bytes_len: attestation_bytes.len() as u64,
    };
    meta.outputs.push(attestation_digest.clone());

    emit_attest_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &manifest_path,
        Some(attestation_digest),
    )
}

pub fn cmd_device_provenance_verify(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceProvenanceVerifyArgs,
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
            return emit_verify_report(
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
            return emit_verify_report(
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
        return emit_verify_report(
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
            return emit_verify_report(
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
            return emit_verify_report(
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
            return emit_verify_report(
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
            return emit_verify_report(
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
            return emit_verify_report(
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
        return emit_verify_report(
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
        return emit_verify_report(
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
            return emit_verify_report(
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
            return emit_verify_report(
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

    let statement_diags = store.validate(
        "https://x07.io/spec/x07-provenance.slsa.attestation.schema.json",
        &attest_doc,
    )?;
    if !statement_diags.is_empty() {
        for dd in statement_diags {
            let mut d = Diagnostic::new(
                "X07WASM_PROVENANCE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return emit_verify_report(
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
        return emit_verify_report(
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

        let full = match util::safe_join_under_dir(&args.bundle_dir, name) {
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

    emit_verify_report(
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

fn rfc3339_utc_now() -> String {
    let dt = time::OffsetDateTime::now_utc();
    let date = dt.date().to_string();
    let time = dt.time().to_string();
    let main = time.split('.').next().unwrap_or(time.as_str());
    format!("{date}T{main}Z")
}

#[allow(clippy::too_many_arguments)]
fn emit_attest_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    bundle_manifest_path: &Path,
    attestation: Option<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let bundle_manifest =
        util::file_digest(bundle_manifest_path).unwrap_or(report::meta::FileDigest {
            path: bundle_manifest_path.display().to_string(),
            sha256: "0".repeat(64),
            bytes_len: 0,
        });
    let attestation = attestation.unwrap_or(report::meta::FileDigest {
        path: bundle_manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("provenance.dsse.json")
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.device.provenance.attest.report@0.1.0",
      "command": "x07-wasm.device.provenance.attest",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "bundle_manifest": bundle_manifest,
        "attestation": attestation,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[allow(clippy::too_many_arguments)]
fn emit_verify_report(
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
      "schema_version": "x07.wasm.device.provenance.verify.report@0.1.0",
      "command": "x07-wasm.device.provenance.verify",
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

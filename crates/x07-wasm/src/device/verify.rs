use std::ffi::OsString;

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{DeviceVerifyArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_BUNDLE_MANIFEST_FILE: &str = "bundle.manifest.json";

#[derive(Debug, Clone, Deserialize)]
struct DeviceBundleManifestDoc {
    schema_version: String,
    kind: String,
    target: String,
    profile: DeviceBundleProfileRef,
    ui_wasm: DeviceBundleFileDigest,
    host: DeviceBundleHost,
    bundle_digest: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceBundleProfileRef {
    id: String,
    v: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceBundleFileDigest {
    path: String,
    sha256: String,
    bytes_len: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceBundleHost {
    kind: String,
    abi_name: String,
    abi_version: String,
    host_abi_hash: String,
}

pub fn cmd_device_verify(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceVerifyArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let bundle_dir = args.dir;
    let manifest_path = bundle_dir.join(DEVICE_BUNDLE_MANIFEST_FILE);

    let manifest_digest = match util::file_digest(&manifest_path) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read bundle manifest {}: {err:#}",
                    manifest_path.display()
                ),
            ));
            report::meta::FileDigest {
                path: manifest_path.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

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
            Vec::new()
        }
    };

    let manifest_json: Value = match serde_json::from_slice(&manifest_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("bundle manifest is not JSON: {err}"),
            ));
            json!(null)
        }
    };

    let mut doc: Option<DeviceBundleManifestDoc> = None;
    if manifest_json != Value::Null {
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
        } else {
            match serde_json::from_value::<DeviceBundleManifestDoc>(manifest_json.clone()) {
                Ok(v) => doc = Some(v),
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUNDLE_MANIFEST_PARSE_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!("failed to parse device bundle manifest doc: {err}"),
                    ));
                }
            }
        }
    }

    let mut files_checked: u64 = 0;
    let mut missing_files: u64 = 0;
    let mut digest_mismatches: u64 = 0;
    let mut bundle_digest_ok = false;
    let mut host_abi_hash_ok = false;

    if let Some(doc) = doc.as_ref() {
        let _ = (
            &doc.schema_version,
            &doc.kind,
            &doc.target,
            &doc.profile.id,
            doc.profile.v,
        );
        let _ = (&doc.host.kind, &doc.host.abi_name, &doc.host.abi_version);

        // Check reducer wasm digest.
        let ui_path = bundle_dir.join(&doc.ui_wasm.path);
        if !ui_path.is_file() {
            missing_files += 1;
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                Severity::Error,
                Stage::Run,
                "bundle file missing".to_string(),
            );
            d.data.insert("path".to_string(), json!(doc.ui_wasm.path));
            diagnostics.push(d);
        } else {
            match std::fs::read(&ui_path) {
                Ok(bytes) => {
                    files_checked += 1;
                    let got_sha = util::sha256_hex(&bytes);
                    let got_len = bytes.len() as u64;
                    if got_sha != doc.ui_wasm.sha256 || got_len != doc.ui_wasm.bytes_len {
                        digest_mismatches += 1;
                        let mut d = Diagnostic::new(
                            "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH",
                            Severity::Error,
                            Stage::Run,
                            "bundle file digest mismatch".to_string(),
                        );
                        d.data
                            .insert("path".to_string(), json!(doc.ui_wasm.path.clone()));
                        d.data
                            .insert("want_sha256".to_string(), json!(doc.ui_wasm.sha256.clone()));
                        d.data.insert("got_sha256".to_string(), json!(got_sha));
                        d.data
                            .insert("want_bytes_len".to_string(), json!(doc.ui_wasm.bytes_len));
                        d.data.insert("got_bytes_len".to_string(), json!(got_len));
                        diagnostics.push(d);
                    }
                }
                Err(err) => {
                    missing_files += 1;
                    let mut d = Diagnostic::new(
                        "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                        Severity::Error,
                        Stage::Run,
                        "failed to read bundle file".to_string(),
                    );
                    d.data.insert("path".to_string(), json!(doc.ui_wasm.path));
                    d.data.insert("error".to_string(), json!(err.to_string()));
                    diagnostics.push(d);
                }
            }
        }

        // Check host ABI hash matches the pinned host ABI.
        let want_host_hash = x07_device_host_abi::host_abi_hash_hex();
        host_abi_hash_ok = doc.host.host_abi_hash == want_host_hash;
        if !host_abi_hash_ok {
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH",
                Severity::Error,
                Stage::Run,
                "host ABI hash mismatch".to_string(),
            );
            d.data
                .insert("want_host_abi_hash".to_string(), json!(want_host_hash));
            d.data.insert(
                "got_host_abi_hash".to_string(),
                json!(doc.host.host_abi_hash.clone()),
            );
            diagnostics.push(d);
        }

        // Recompute bundle_digest using canonical JSON with bundle_digest zeroed.
        let mut v = manifest_json.clone();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("bundle_digest".to_string(), json!("0".repeat(64)));
        }
        let bytes = report::canon::canonical_json_bytes(&v)?;
        let got_bundle_digest = util::sha256_hex(&bytes);
        bundle_digest_ok = got_bundle_digest == doc.bundle_digest;
        if !bundle_digest_ok {
            digest_mismatches += 1;
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_DIGEST_MISMATCH",
                Severity::Error,
                Stage::Run,
                "bundle_digest mismatch".to_string(),
            );
            d.data.insert(
                "want_bundle_digest".to_string(),
                json!(doc.bundle_digest.clone()),
            );
            d.data
                .insert("got_bundle_digest".to_string(), json!(got_bundle_digest));
            diagnostics.push(d);
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.device.verify.report@0.1.0",
      "command": "x07-wasm.device.verify",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "bundle_dir": bundle_dir.display().to_string(),
        "bundle_manifest": manifest_digest,
        "files_checked": files_checked,
        "missing_files": missing_files,
        "digest_mismatches": digest_mismatches,
        "bundle_digest_ok": bundle_digest_ok,
        "host_abi_hash_ok": host_abi_hash_ok,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

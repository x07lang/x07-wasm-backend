use std::ffi::OsString;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{DeviceVerifyArgs, MachineArgs, Scope};
use crate::device::contracts::DeviceBundleManifestDoc;
use crate::device::host_abi;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_BUNDLE_MANIFEST_FILE: &str = "bundle.manifest.json";

// Hard caps to prevent verification from reading unbounded data.
const MAX_BUNDLE_MANIFEST_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB
const MAX_BUNDLE_FILE_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB

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

    let mut manifest_digest = report::meta::FileDigest {
        path: manifest_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    let manifest_bytes: Option<Vec<u8>> =
        match util::read_file_capped(&manifest_path, MAX_BUNDLE_MANIFEST_BYTES) {
            Ok(v) => {
                manifest_digest = report::meta::FileDigest {
                    path: manifest_path.display().to_string(),
                    sha256: util::sha256_hex(&v),
                    bytes_len: v.len() as u64,
                };
                meta.inputs.push(manifest_digest.clone());
                Some(v)
            }
            Err(err) => {
                manifest_digest.bytes_len = err.bytes_len;
                if err.kind == "too_large" {
                    let mut d = Diagnostic::new(
                        "X07WASM_DEVICE_BUNDLE_MANIFEST_TOO_LARGE",
                        Severity::Error,
                        Stage::Parse,
                        format!("bundle manifest exceeds size cap: {}", err.path),
                    );
                    d.data.insert("path".to_string(), json!(err.path.clone()));
                    d.data.insert("bytes_len".to_string(), json!(err.bytes_len));
                    d.data
                        .insert("max_bytes_len".to_string(), json!(err.max_bytes));
                    diagnostics.push(d);
                } else {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUNDLE_MANIFEST_READ_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!(
                            "failed to read bundle manifest {}: {}",
                            manifest_path.display(),
                            err.detail
                        ),
                    ));
                }
                None
            }
        };

    let manifest_json: Value = match manifest_bytes.as_deref() {
        Some(bytes) => match serde_json::from_slice(bytes) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_MANIFEST_JSON_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("bundle manifest is not JSON: {err}"),
                ));
                Value::Null
            }
        },
        None => Value::Null,
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
        let ui_path = match util::safe_join_under_dir(&bundle_dir, &doc.ui_wasm.path) {
            Ok(v) => Some(v),
            Err(err) => {
                missing_files += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_PATH_UNSAFE",
                    Severity::Error,
                    Stage::Run,
                    "unsafe bundle path".to_string(),
                );
                d.data.insert("field".to_string(), json!("ui_wasm.path"));
                d.data.insert("path".to_string(), json!(err.rel));
                d.data.insert("kind".to_string(), json!(err.kind));
                d.data.insert("detail".to_string(), json!(err.detail));
                diagnostics.push(d);
                None
            }
        };
        if let Some(ui_path) = ui_path.as_ref() {
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
                match util::sha256_file_hex_capped(ui_path, MAX_BUNDLE_FILE_BYTES) {
                    Ok((got_sha, got_len)) => {
                        files_checked += 1;
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
                            d.data.insert(
                                "want_sha256".to_string(),
                                json!(doc.ui_wasm.sha256.clone()),
                            );
                            d.data.insert("got_sha256".to_string(), json!(got_sha));
                            d.data
                                .insert("want_bytes_len".to_string(), json!(doc.ui_wasm.bytes_len));
                            d.data.insert("got_bytes_len".to_string(), json!(got_len));
                            diagnostics.push(d);
                        }
                    }
                    Err(err) => {
                        missing_files += 1;
                        if err.kind == "too_large" {
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_BUNDLE_FILE_TOO_LARGE",
                                Severity::Error,
                                Stage::Run,
                                format!("bundle file exceeds size cap: {}", ui_path.display()),
                            );
                            d.data
                                .insert("path".to_string(), json!(doc.ui_wasm.path.clone()));
                            d.data.insert("bytes_len".to_string(), json!(err.bytes_len));
                            d.data
                                .insert("max_bytes_len".to_string(), json!(err.max_bytes));
                            d.data.insert("role".to_string(), json!("ui_wasm"));
                            diagnostics.push(d);
                        } else {
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                                Severity::Error,
                                Stage::Run,
                                "failed to read bundle file".to_string(),
                            );
                            d.data
                                .insert("path".to_string(), json!(doc.ui_wasm.path.clone()));
                            d.data.insert("error".to_string(), json!(err.detail));
                            diagnostics.push(d);
                        }
                    }
                }
            }
        }

        // Check embedded device profile digest (bundle must be self-contained).
        let profile_path = match util::safe_join_under_dir(&bundle_dir, &doc.profile.file.path) {
            Ok(v) => Some(v),
            Err(err) => {
                missing_files += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_PATH_UNSAFE",
                    Severity::Error,
                    Stage::Run,
                    "unsafe bundle path".to_string(),
                );
                d.data
                    .insert("field".to_string(), json!("profile.file.path"));
                d.data.insert("path".to_string(), json!(err.rel));
                d.data.insert("kind".to_string(), json!(err.kind));
                d.data.insert("detail".to_string(), json!(err.detail));
                diagnostics.push(d);
                None
            }
        };
        if let Some(profile_path) = profile_path.as_ref() {
            if !profile_path.is_file() {
                missing_files += 1;
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                    Severity::Error,
                    Stage::Run,
                    "bundle file missing".to_string(),
                );
                d.data
                    .insert("path".to_string(), json!(doc.profile.file.path.clone()));
                diagnostics.push(d);
            } else {
                match util::sha256_file_hex_capped(profile_path, MAX_BUNDLE_FILE_BYTES) {
                    Ok((got_sha, got_len)) => {
                        files_checked += 1;
                        if got_sha != doc.profile.file.sha256
                            || got_len != doc.profile.file.bytes_len
                        {
                            digest_mismatches += 1;
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH",
                                Severity::Error,
                                Stage::Run,
                                "bundle file digest mismatch".to_string(),
                            );
                            d.data
                                .insert("path".to_string(), json!(doc.profile.file.path.clone()));
                            d.data.insert(
                                "want_sha256".to_string(),
                                json!(doc.profile.file.sha256.clone()),
                            );
                            d.data.insert("got_sha256".to_string(), json!(got_sha));
                            d.data.insert(
                                "want_bytes_len".to_string(),
                                json!(doc.profile.file.bytes_len),
                            );
                            d.data.insert("got_bytes_len".to_string(), json!(got_len));
                            diagnostics.push(d);
                        }
                    }
                    Err(err) => {
                        missing_files += 1;
                        if err.kind == "too_large" {
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_BUNDLE_FILE_TOO_LARGE",
                                Severity::Error,
                                Stage::Run,
                                format!("bundle file exceeds size cap: {}", profile_path.display()),
                            );
                            d.data
                                .insert("path".to_string(), json!(doc.profile.file.path.clone()));
                            d.data.insert("bytes_len".to_string(), json!(err.bytes_len));
                            d.data
                                .insert("max_bytes_len".to_string(), json!(err.max_bytes));
                            d.data.insert("role".to_string(), json!("profile"));
                            diagnostics.push(d);
                        } else {
                            let mut d = Diagnostic::new(
                                "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                                Severity::Error,
                                Stage::Run,
                                "failed to read bundle file".to_string(),
                            );
                            d.data
                                .insert("path".to_string(), json!(doc.profile.file.path.clone()));
                            d.data.insert("error".to_string(), json!(err.detail));
                            diagnostics.push(d);
                        }
                    }
                }
            }
        }

        // Check host ABI hash matches the pinned host ABI.
        let want_host_hash = host_abi::HOST_ABI_HASH_HEX;
        if want_host_hash.len() == 64 {
            host_abi_hash_ok = doc.host.host_abi_hash == want_host_hash;
            if !host_abi_hash_ok {
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH",
                    Severity::Error,
                    Stage::Parse,
                    "bundle host ABI hash does not match pinned toolchain host ABI".to_string(),
                );
                d.data
                    .insert("expected_host_abi_hash".to_string(), json!(want_host_hash));
                d.data.insert(
                    "bundle_host_abi_hash".to_string(),
                    json!(doc.host.host_abi_hash.clone()),
                );
                diagnostics.push(d);
            }
        } else if host_abi::HOST_ABI_HASH_HEX.len() != 64 {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_HOST_ABI_HASH_INVALID",
                Severity::Error,
                Stage::Parse,
                "pinned HOST_ABI_HASH_HEX is not a valid sha256".to_string(),
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("x07-wasm-device-verify-{tag}-{}-{n}", std::process::id());
        std::env::temp_dir().join(name)
    }

    #[test]
    fn device_verify_uses_embedded_host_abi_hash_outside_repo_root() {
        let tmp = tmp_dir("embedded_host_abi");
        let bundle_dir = tmp.join("bundle");
        std::fs::create_dir_all(bundle_dir.join("ui")).expect("create ui dir");
        std::fs::create_dir_all(bundle_dir.join("profile")).expect("create profile dir");

        let ui_bytes = b"not-a-real-wasm-module";
        let profile_bytes = br#"{"id":"device_dev","v":1}"#;
        std::fs::write(bundle_dir.join("ui/reducer.wasm"), ui_bytes).expect("write reducer.wasm");
        std::fs::write(
            bundle_dir.join("profile/device.profile.json"),
            profile_bytes,
        )
        .expect("write device.profile.json");

        let mut manifest = json!({
          "schema_version": "x07.device.bundle.manifest@0.1.0",
          "kind": "device_bundle",
          "target": "desktop",
          "profile": {
            "id": "device_dev",
            "v": 1,
            "file": {
              "path": "profile/device.profile.json",
              "sha256": util::sha256_hex(profile_bytes),
              "bytes_len": profile_bytes.len(),
            },
          },
          "ui_wasm": {
            "path": "ui/reducer.wasm",
            "sha256": util::sha256_hex(ui_bytes),
            "bytes_len": ui_bytes.len(),
          },
          "host": {
            "kind": host_abi::HOST_KIND,
            "abi_name": host_abi::ABI_NAME,
            "abi_version": host_abi::ABI_VERSION,
            "host_abi_hash": host_abi::HOST_ABI_HASH_HEX,
          },
          "bundle_digest": "0".repeat(64),
        });

        let manifest_bytes =
            report::canon::canonical_json_bytes(&manifest).expect("manifest canon");
        let bundle_digest = util::sha256_hex(&manifest_bytes);
        manifest["bundle_digest"] = json!(bundle_digest);

        let manifest_path = bundle_dir.join(DEVICE_BUNDLE_MANIFEST_FILE);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write bundle.manifest.json");

        let report_out = tmp.join("verify.report.json");
        let expected_manifest_path = bundle_dir.join("bundle.manifest.json");
        let expected_manifest_path = expected_manifest_path.display().to_string();
        let machine = MachineArgs {
            json: Some(String::new()),
            report_json: None,
            report_out: Some(report_out.clone()),
            quiet_json: true,
            json_schema: false,
            json_schema_id: false,
        };

        let exit_code = cmd_device_verify(
            &[
                OsString::from("x07-wasm"),
                OsString::from("device"),
                OsString::from("verify"),
            ],
            Scope::DeviceVerify,
            &machine,
            DeviceVerifyArgs { dir: bundle_dir },
        )
        .expect("cmd_device_verify");
        assert_eq!(exit_code, 0);

        let report_doc: Value =
            serde_json::from_slice(&std::fs::read(&report_out).expect("read verify.report.json"))
                .expect("parse verify.report.json");

        assert_eq!(report_doc.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(
            report_doc
                .pointer("/result/host_abi_hash_ok")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report_doc
                .pointer("/meta/inputs")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            report_doc
                .pointer("/meta/inputs/0/path")
                .and_then(Value::as_str),
            Some(expected_manifest_path.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

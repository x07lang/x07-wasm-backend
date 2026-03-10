use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::device::contracts::{DeviceBundleFileDigest, DeviceBundleManifestDoc, DeviceProfileDoc};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_CAPABILITIES_SCHEMA_ID: &str =
    "https://x07.io/spec/x07-device.capabilities.schema.json";
const DEVICE_TELEMETRY_PROFILE_SCHEMA_ID: &str =
    "https://x07.io/spec/x07-device.telemetry.profile.schema.json";
const REQUIRED_TELEMETRY_EVENT_CLASSES: &[&str] = &[
    "app.lifecycle",
    "app.http",
    "runtime.error",
    "bridge.timing",
    "reducer.timing",
    "policy.violation",
    "host.webview_crash",
];

#[derive(Debug, Clone)]
pub(crate) struct DeviceProfileSidecars {
    pub(crate) capabilities: ValidatedJsonFile,
    pub(crate) telemetry_profile: ValidatedJsonFile,
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceBundleSidecars {
    pub(crate) capabilities: ValidatedJsonFile,
    pub(crate) telemetry_profile: ValidatedJsonFile,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedJsonFile {
    pub(crate) path: PathBuf,
    pub(crate) doc: Value,
}

struct JsonContractSpec<'a> {
    schema_id: &'a str,
    read_failed_code: &'a str,
    json_invalid_code: &'a str,
    schema_invalid_code: &'a str,
    label: &'a str,
}

pub(crate) fn load_profile_sidecars(
    store: &SchemaStore,
    profile_doc: &DeviceProfileDoc,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<DeviceProfileSidecars> {
    let capabilities_spec = JsonContractSpec {
        schema_id: DEVICE_CAPABILITIES_SCHEMA_ID,
        read_failed_code: "X07WASM_DEVICE_CAPABILITIES_READ_FAILED",
        json_invalid_code: "X07WASM_DEVICE_CAPABILITIES_JSON_INVALID",
        schema_invalid_code: "X07WASM_DEVICE_CAPABILITIES_SCHEMA_INVALID",
        label: "device capabilities",
    };
    let capabilities = load_json_contract_file(
        store,
        &profile_doc.capabilities.path,
        &capabilities_spec,
        meta,
        diagnostics,
    )?;
    let telemetry_spec = JsonContractSpec {
        schema_id: DEVICE_TELEMETRY_PROFILE_SCHEMA_ID,
        read_failed_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_READ_FAILED",
        json_invalid_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_JSON_INVALID",
        schema_invalid_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_SCHEMA_INVALID",
        label: "device telemetry profile",
    };
    let telemetry_profile = load_json_contract_file(
        store,
        &profile_doc.telemetry_profile.path,
        &telemetry_spec,
        meta,
        diagnostics,
    )?;
    Some(DeviceProfileSidecars {
        capabilities,
        telemetry_profile,
    })
}

pub(crate) fn load_bundle_sidecars(
    store: &SchemaStore,
    bundle_dir: &Path,
    bundle_doc: &DeviceBundleManifestDoc,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<DeviceBundleSidecars> {
    let capabilities_spec = JsonContractSpec {
        schema_id: DEVICE_CAPABILITIES_SCHEMA_ID,
        read_failed_code: "X07WASM_DEVICE_CAPABILITIES_READ_FAILED",
        json_invalid_code: "X07WASM_DEVICE_CAPABILITIES_JSON_INVALID",
        schema_invalid_code: "X07WASM_DEVICE_CAPABILITIES_SCHEMA_INVALID",
        label: "device capabilities",
    };
    let capabilities = load_bundle_json_contract_file(
        store,
        bundle_dir,
        "capabilities.path",
        &bundle_doc.capabilities,
        &capabilities_spec,
        meta,
        diagnostics,
    )?;
    let telemetry_spec = JsonContractSpec {
        schema_id: DEVICE_TELEMETRY_PROFILE_SCHEMA_ID,
        read_failed_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_READ_FAILED",
        json_invalid_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_JSON_INVALID",
        schema_invalid_code: "X07WASM_DEVICE_TELEMETRY_PROFILE_SCHEMA_INVALID",
        label: "device telemetry profile",
    };
    let telemetry_profile = load_bundle_json_contract_file(
        store,
        bundle_dir,
        "telemetry_profile.path",
        &bundle_doc.telemetry_profile,
        &telemetry_spec,
        meta,
        diagnostics,
    )?;
    Some(DeviceBundleSidecars {
        capabilities,
        telemetry_profile,
    })
}

fn load_json_contract_file(
    store: &SchemaStore,
    path: &Path,
    spec: &JsonContractSpec<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ValidatedJsonFile> {
    let bytes = read_and_track_json_file(path, spec, meta, diagnostics)?;
    let doc_json = validate_json_contract_bytes(store, &bytes, spec, diagnostics, path)?;

    Some(ValidatedJsonFile {
        path: path.to_path_buf(),
        doc: doc_json,
    })
}

fn load_bundle_json_contract_file(
    store: &SchemaStore,
    bundle_dir: &Path,
    field: &str,
    file: &DeviceBundleFileDigest,
    spec: &JsonContractSpec<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ValidatedJsonFile> {
    let full_path = match util::safe_join_under_dir(bundle_dir, &file.path) {
        Ok(path) => path,
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
            return None;
        }
    };

    let bytes = read_and_track_bundle_file(&full_path, file, spec, meta, diagnostics)?;
    let doc_json = validate_json_contract_bytes(store, &bytes, spec, diagnostics, &full_path)?;

    Some(ValidatedJsonFile {
        path: full_path,
        doc: doc_json,
    })
}

fn read_and_track_json_file(
    path: &Path,
    spec: &JsonContractSpec<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Vec<u8>> {
    match util::file_digest(path) {
        Ok(d) => {
            meta.inputs.push(d);
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                spec.read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {} {}: {err:#}", spec.label, path.display()),
            ));
            return None;
        }
    }

    match std::fs::read(path) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                spec.read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {} {}: {err}", spec.label, path.display()),
            ));
            None
        }
    }
}

fn read_and_track_bundle_file(
    full_path: &Path,
    file: &DeviceBundleFileDigest,
    spec: &JsonContractSpec<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Vec<u8>> {
    let bytes = match std::fs::read(full_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                spec.read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read {} {}: {err}",
                    spec.label,
                    full_path.display()
                ),
            ));
            return None;
        }
    };

    let got_sha = util::sha256_hex(&bytes);
    let got_len = bytes.len() as u64;
    meta.inputs.push(report::meta::FileDigest {
        path: full_path.display().to_string(),
        sha256: got_sha.clone(),
        bytes_len: got_len,
    });

    if got_sha != file.sha256 || got_len != file.bytes_len {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH",
            Severity::Error,
            Stage::Parse,
            "bundle file digest mismatch".to_string(),
        );
        d.data.insert("path".to_string(), json!(file.path.clone()));
        d.data
            .insert("want_sha256".to_string(), json!(file.sha256.clone()));
        d.data.insert("got_sha256".to_string(), json!(got_sha));
        d.data
            .insert("want_bytes_len".to_string(), json!(file.bytes_len));
        d.data.insert("got_bytes_len".to_string(), json!(got_len));
        diagnostics.push(d);
        return None;
    }

    Some(bytes)
}

fn validate_json_contract_bytes(
    store: &SchemaStore,
    bytes: &[u8],
    spec: &JsonContractSpec<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    path: &Path,
) -> Option<Value> {
    let doc_json: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                spec.json_invalid_code,
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to parse {} JSON {}: {err}",
                    spec.label,
                    path.display()
                ),
            ));
            return None;
        }
    };

    let schema_diags = match store.validate(spec.schema_id, &doc_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_INTERNAL_SCHEMA_VALIDATE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return None;
        }
    };

    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        let mut d = Diagnostic::new(
            spec.schema_invalid_code,
            Severity::Error,
            Stage::Parse,
            format!("{} schema invalid", spec.label),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        diagnostics.push(d);
        return None;
    }

    if spec.schema_id == DEVICE_TELEMETRY_PROFILE_SCHEMA_ID
        && !validate_telemetry_profile_contract(&doc_json, diagnostics)
    {
        return None;
    }

    Some(doc_json)
}

fn validate_telemetry_profile_contract(
    doc_json: &Value,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let classes = doc_json
        .get("event_classes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let have = classes
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let missing = REQUIRED_TELEMETRY_EVENT_CLASSES
        .iter()
        .filter(|name| !have.contains(**name))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_TELEMETRY_EVENT_CLASSES_INCOMPLETE",
            Severity::Error,
            Stage::Parse,
            "device telemetry profile must declare the full required event-class set".to_string(),
        );
        d.data.insert("missing".to_string(), json!(missing));
        diagnostics.push(d);
        return false;
    }

    true
}

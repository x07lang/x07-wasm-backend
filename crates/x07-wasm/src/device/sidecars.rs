use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::device::contracts::DeviceProfileDoc;
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
pub(crate) struct ValidatedJsonFile {
    pub(crate) path: PathBuf,
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

fn load_json_contract_file(
    store: &SchemaStore,
    path: &Path,
    spec: &JsonContractSpec<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ValidatedJsonFile> {
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

    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                spec.read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {} {}: {err}", spec.label, path.display()),
            ));
            return None;
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
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

    Some(ValidatedJsonFile {
        path: path.to_path_buf(),
    })
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

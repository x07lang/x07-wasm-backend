use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::device::contracts::DeviceProfileDoc;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_CAPABILITIES_SCHEMA_ID: &str = "https://x07.io/spec/x07-device.capabilities.schema.json";
const DEVICE_TELEMETRY_PROFILE_SCHEMA_ID: &str =
    "https://x07.io/spec/x07-device.telemetry.profile.schema.json";

#[derive(Debug, Clone)]
pub(crate) struct DeviceProfileSidecars {
    pub(crate) capabilities: ValidatedJsonFile,
    pub(crate) telemetry_profile: ValidatedJsonFile,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedJsonFile {
    pub(crate) path: PathBuf,
}

pub(crate) fn load_profile_sidecars(
    store: &SchemaStore,
    profile_doc: &DeviceProfileDoc,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<DeviceProfileSidecars> {
    let capabilities = load_json_contract_file(
        store,
        &profile_doc.capabilities.path,
        DEVICE_CAPABILITIES_SCHEMA_ID,
        "X07WASM_DEVICE_CAPABILITIES_READ_FAILED",
        "X07WASM_DEVICE_CAPABILITIES_JSON_INVALID",
        "X07WASM_DEVICE_CAPABILITIES_SCHEMA_INVALID",
        "device capabilities",
        meta,
        diagnostics,
    )?;
    let telemetry_profile = load_json_contract_file(
        store,
        &profile_doc.telemetry_profile.path,
        DEVICE_TELEMETRY_PROFILE_SCHEMA_ID,
        "X07WASM_DEVICE_TELEMETRY_PROFILE_READ_FAILED",
        "X07WASM_DEVICE_TELEMETRY_PROFILE_JSON_INVALID",
        "X07WASM_DEVICE_TELEMETRY_PROFILE_SCHEMA_INVALID",
        "device telemetry profile",
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
    schema_id: &str,
    read_failed_code: &str,
    json_invalid_code: &str,
    schema_invalid_code: &str,
    label: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ValidatedJsonFile> {
    match util::file_digest(path) {
        Ok(d) => {
            meta.inputs.push(d);
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {label} {}: {err:#}", path.display()),
            ));
            return None;
        }
    }

    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {label} {}: {err}", path.display()),
            ));
            return None;
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                json_invalid_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to parse {label} JSON {}: {err}", path.display()),
            ));
            return None;
        }
    };

    let schema_diags = match store.validate(schema_id, &doc_json) {
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
            schema_invalid_code,
            Severity::Error,
            Stage::Parse,
            format!("{label} schema invalid"),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        diagnostics.push(d);
        return None;
    }

    Some(ValidatedJsonFile {
        path: path.to_path_buf(),
    })
}

use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
pub struct AppBundleDoc {
    pub profile_id: String,
    pub frontend: AppBundleFrontend,
    pub backend: AppBundleBackend,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppBundleFrontend {
    pub format: String,
    pub dir_rel: String,
    pub artifacts: Vec<BundleFileDigest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppBundleBackend {
    pub adapter: String,
    pub artifact: BundleFileDigest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundleFileDigest {
    pub path: String,
    pub sha256: String,
    pub bytes_len: u64,
}

pub struct LoadedAppBundle {
    pub doc_json: Value,
    pub doc: AppBundleDoc,
}

pub fn load_app_bundle(
    store: &SchemaStore,
    bundle_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<LoadedAppBundle>> {
    let bundle_path = bundle_dir.join("app.bundle.json");
    if !bundle_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BUNDLE_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("missing bundle manifest: {}", bundle_path.display()),
        ));
        return Ok(None);
    }

    match util::file_digest(&bundle_path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_DIGEST_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to digest {}: {err:#}", bundle_path.display()),
            ));
        }
    };

    let bytes = match std::fs::read(&bundle_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read {}: {err}", bundle_path.display()),
            ));
            Vec::new()
        }
    };
    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse JSON {}: {err}", bundle_path.display()),
            ));
            Value::Null
        }
    };

    let diag_before = diagnostics.len();
    diagnostics
        .extend(store.validate("https://x07.io/spec/x07-app.bundle.schema.json", &doc_json)?);
    if diagnostics[diag_before..]
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Ok(None);
    }

    let doc: AppBundleDoc = match serde_json::from_value(doc_json.clone()) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse bundle doc: {err}"),
            ));
            return Ok(None);
        }
    };

    let mut ok = true;
    if doc.backend.adapter.trim() != "wasi_http_proxy_v1" {
        ok = false;
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BUNDLE_BACKEND_ADAPTER_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!("unsupported backend.adapter: {:?}", doc.backend.adapter),
        ));
    }
    match doc.frontend.format.as_str() {
        "core_wasm_v1" | "component_jco_v1" => {}
        other => {
            ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_FRONTEND_FORMAT_UNSUPPORTED",
                Severity::Error,
                Stage::Parse,
                format!("unsupported frontend.format: {other:?}"),
            ));
        }
    }

    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for a in doc.frontend.artifacts.iter() {
        if !seen.insert(a.path.as_str()) {
            ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_FRONTEND_ARTIFACT_DUPLICATE",
                Severity::Error,
                Stage::Parse,
                format!("duplicate artifact path in frontend list: {:?}", a.path),
            ));
        }
        if !a.path.starts_with(&format!("{}/", doc.frontend.dir_rel)) {
            ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_FRONTEND_ARTIFACT_OUT_OF_DIR",
                Severity::Error,
                Stage::Parse,
                format!(
                    "frontend artifact path {:?} is not under frontend.dir_rel {:?}",
                    a.path, doc.frontend.dir_rel
                ),
            ));
        }
        if !verify_bundle_file_digest(bundle_dir, a, diagnostics)? {
            ok = false;
        }
    }

    if !verify_bundle_file_digest(bundle_dir, &doc.backend.artifact, diagnostics)? {
        ok = false;
    }

    if !ok {
        return Ok(None);
    }

    Ok(Some(LoadedAppBundle { doc_json, doc }))
}

fn verify_bundle_file_digest(
    bundle_dir: &Path,
    entry: &BundleFileDigest,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<bool> {
    let path = bundle_dir.join(&entry.path);
    if !path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BUNDLE_ARTIFACT_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("bundle artifact not found: {}", path.display()),
        ));
        return Ok(false);
    }

    let actual = match util::file_digest(&path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_BUNDLE_ARTIFACT_DIGEST_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to digest bundle artifact {}: {err:#}",
                    path.display()
                ),
            ));
            return Ok(false);
        }
    };
    let ok = actual.sha256 == entry.sha256 && actual.bytes_len == entry.bytes_len;
    if !ok {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BUNDLE_ARTIFACT_DIGEST_MISMATCH",
            Severity::Error,
            Stage::Parse,
            format!(
                "bundle artifact digest mismatch for {:?}: expected sha256={} bytes_len={}, got sha256={} bytes_len={}",
                entry.path, entry.sha256, entry.bytes_len, actual.sha256, actual.bytes_len
            ),
        ));
    }
    Ok(ok)
}

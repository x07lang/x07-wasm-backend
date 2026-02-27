pub mod validate;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone)]
pub struct LoadedJsonRef {
    pub path: PathBuf,
    pub digest: report::meta::FileDigest,
    pub doc_json: Value,
    pub schema_valid: bool,
    pub ok: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedOpsProfile {
    pub ops: LoadedJsonRef,
    pub capabilities: LoadedJsonRef,
    pub policy_cards: Vec<LoadedJsonRef>,
    pub slo_profile: Option<LoadedJsonRef>,
}

pub fn load_ops_profile_with_refs(
    store: &SchemaStore,
    ops_profile_path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<LoadedOpsProfile>> {
    let ops = match load_json_ref(
        store,
        ops_profile_path,
        None,
        "https://x07.io/spec/x07-app.ops.profile.schema.json",
        "X07WASM_OPS_PROFILE_READ_FAILED",
        "X07WASM_OPS_PROFILE_SCHEMA_INVALID",
        meta,
        diagnostics,
    )? {
        Some(v) => v,
        None => return Ok(None),
    };

    let caps_ref = ops
        .doc_json
        .get("capabilities")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let caps_path = caps_ref.get("path").and_then(Value::as_str).unwrap_or("");
    if caps_path.trim().is_empty() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_OPS_REF_SCHEMA_INVALID",
            Severity::Error,
            Stage::Parse,
            "ops profile missing capabilities.path".to_string(),
        ));
        return Ok(Some(LoadedOpsProfile {
            ops,
            capabilities: missing_ref_status(Path::new("missing")),
            policy_cards: Vec::new(),
            slo_profile: None,
        }));
    }
    let caps_expected_sha256 = caps_ref
        .get("sha256")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let capabilities = match load_json_ref(
        store,
        &PathBuf::from(caps_path),
        caps_expected_sha256,
        "https://x07.io/spec/x07-app.capabilities.schema.json",
        "X07WASM_OPS_REF_MISSING",
        "X07WASM_OPS_REF_SCHEMA_INVALID",
        meta,
        diagnostics,
    )? {
        Some(v) => v,
        None => missing_ref_status(&PathBuf::from(caps_path)),
    };

    let mut policy_cards: Vec<LoadedJsonRef> = Vec::new();
    let policy_refs = ops
        .doc_json
        .get("policy_cards")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, r) in policy_refs.iter().enumerate() {
        let Some(obj) = r.as_object() else {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_REF_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("policy_cards[{i}] is not an object"),
            ));
            continue;
        };
        let path = obj.get("path").and_then(Value::as_str).unwrap_or("");
        if path.trim().is_empty() {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_REF_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("policy_cards[{i}] missing path"),
            ));
            continue;
        }
        let expected_sha256 = obj
            .get("sha256")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let loaded = load_json_ref(
            store,
            &PathBuf::from(path),
            expected_sha256,
            "https://x07.io/spec/x07-policy.card.schema.json",
            "X07WASM_OPS_REF_MISSING",
            "X07WASM_OPS_REF_SCHEMA_INVALID",
            meta,
            diagnostics,
        )?
        .unwrap_or_else(|| missing_ref_status(&PathBuf::from(path)));
        policy_cards.push(loaded);
    }

    let slo_ref_obj = ops.doc_json.get("slo").and_then(Value::as_object).cloned();
    let slo_profile = if let Some(obj) = slo_ref_obj {
        let path = obj.get("path").and_then(Value::as_str).unwrap_or("");
        if path.trim().is_empty() {
            diagnostics.push(Diagnostic::new(
                "X07WASM_OPS_REF_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "ops profile has slo object without path".to_string(),
            ));
            None
        } else {
            let expected_sha256 = obj
                .get("sha256")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Some(
                load_json_ref(
                    store,
                    &PathBuf::from(path),
                    expected_sha256,
                    "https://x07.io/spec/x07-slo.profile.schema.json",
                    "X07WASM_OPS_REF_MISSING",
                    "X07WASM_OPS_REF_SCHEMA_INVALID",
                    meta,
                    diagnostics,
                )?
                .unwrap_or_else(|| missing_ref_status(&PathBuf::from(path))),
            )
        }
    } else {
        None
    };

    // Semantic check: analysis steps require slo profile.
    let needs_slo = ops
        .doc_json
        .get("deploy")
        .and_then(|d| d.get("canary"))
        .and_then(|c| c.get("steps"))
        .and_then(Value::as_array)
        .is_some_and(|steps| steps.iter().any(|s| s.get("analysis").is_some()));
    if needs_slo && slo_profile.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_OPS_DEPLOY_STRATEGY_INVALID",
            Severity::Error,
            Stage::Parse,
            "ops deploy.canary.steps includes analysis but slo is null".to_string(),
        ));
    }

    // Semantic check: predicate_type supported.
    let predicate_type = ops
        .doc_json
        .get("provenance")
        .and_then(|p| p.get("predicate_type"))
        .and_then(Value::as_str)
        .unwrap_or("https://slsa.dev/provenance/v1");
    if predicate_type != "https://slsa.dev/provenance/v1" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_OPS_PROVENANCE_REQUIREMENTS_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unsupported provenance.predicate_type: {predicate_type:?}"),
        ));
    }

    Ok(Some(LoadedOpsProfile {
        ops,
        capabilities,
        policy_cards,
        slo_profile,
    }))
}

#[allow(clippy::too_many_arguments)]
fn load_json_ref(
    store: &SchemaStore,
    path: &Path,
    expected_sha256: Option<String>,
    schema_id: &str,
    code_read_failed: &str,
    code_schema_invalid: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<LoadedJsonRef>> {
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                code_read_failed,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {}: {err}", path.display()),
            ));
            return Ok(None);
        }
    };
    let digest = report::meta::FileDigest {
        path: path.display().to_string(),
        sha256: util::sha256_hex(&bytes),
        bytes_len: bytes.len() as u64,
    };
    meta.inputs.push(digest.clone());

    let mut digest_ok = true;
    if let Some(want) = expected_sha256.as_deref() {
        if want != digest.sha256 {
            digest_ok = false;
            let mut d = Diagnostic::new(
                "X07WASM_OPS_REF_DIGEST_MISMATCH",
                Severity::Error,
                Stage::Parse,
                format!(
                    "digest mismatch for {} (expected sha256={want}, got sha256={})",
                    path.display(),
                    digest.sha256
                ),
            );
            d.data.insert("expected_sha256".to_string(), json!(want));
            d.data
                .insert("actual_sha256".to_string(), json!(digest.sha256.clone()));
            diagnostics.push(d);
        }
    }

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                code_schema_invalid,
                Severity::Error,
                Stage::Parse,
                format!("JSON invalid: {err}"),
            ));
            return Ok(Some(LoadedJsonRef {
                path: path.to_path_buf(),
                digest,
                doc_json: Value::Null,
                schema_valid: false,
                ok: false,
            }));
        }
    };

    let schema_diags = store
        .validate(schema_id, &doc_json)
        .with_context(|| format!("validate schema {schema_id:?}"))?;
    let schema_valid = schema_diags.is_empty();
    if !schema_valid {
        for dd in schema_diags {
            let mut d = Diagnostic::new(
                code_schema_invalid,
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
    }

    Ok(Some(LoadedJsonRef {
        path: path.to_path_buf(),
        digest,
        doc_json,
        schema_valid,
        ok: digest_ok && schema_valid,
    }))
}

fn missing_ref_status(path: &Path) -> LoadedJsonRef {
    LoadedJsonRef {
        path: path.to_path_buf(),
        digest: report::meta::FileDigest {
            path: path.display().to_string(),
            sha256: "0".repeat(64),
            bytes_len: 0,
        },
        doc_json: Value::Null,
        schema_valid: false,
        ok: false,
    }
}

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub(crate) fn load_validated_json_doc(
    store: &SchemaStore,
    path: &Path,
    schema_id: &str,
    code_read_failed: &str,
    code_schema_invalid: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<(report::meta::FileDigest, Value)>> {
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

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                code_schema_invalid,
                Severity::Error,
                Stage::Parse,
                format!("JSON invalid: {err}"),
            ));
            return Ok(None);
        }
    };

    let schema_diags = store.validate(schema_id, &doc_json)?;
    if !schema_diags.is_empty() {
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
        return Ok(None);
    }

    Ok(Some((digest, doc_json)))
}

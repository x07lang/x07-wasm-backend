use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{HttpContractsValidateArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_http_contracts_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: HttpContractsValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let schema_ids: Vec<&'static str> = vec![
        "https://x07.io/spec/x07-http.request.envelope.schema.json",
        "https://x07.io/spec/x07-http.response.envelope.schema.json",
        "https://x07.io/spec/x07-http.effect.schema.json",
        "https://x07.io/spec/x07-http.dispatch.schema.json",
        "https://x07.io/spec/x07-http.frame.schema.json",
        "https://x07.io/spec/x07-http.trace.schema.json",
        "https://x07.io/spec/x07-wasm.http.contracts.validate.report.schema.json",
        "https://x07.io/spec/x07-wasm.http.serve.report.schema.json",
        "https://x07.io/spec/x07-wasm.http.test.report.schema.json",
        "https://x07.io/spec/x07-wasm.http.regress.from.incident.report.schema.json",
    ];

    let mut schema_checks: Vec<Value> = Vec::new();
    for id in schema_ids {
        let ok = store.schema(id).is_ok();
        if !ok {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_SCHEMA_MISSING",
                Severity::Error,
                Stage::Run,
                format!("missing embedded schema: {id:?}"),
            ));
        }
        schema_checks.push(json!({ "id": id, "ok": ok }));
    }

    let fixtures = discover_default_fixtures();

    let mut fixture_checks: Vec<Value> = Vec::new();
    for path in fixtures {
        let ok = match validate_fixture(&store, &path, &mut meta, &mut diagnostics) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_HTTP_FIXTURE_VALIDATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to validate fixture {}: {err:#}", path.display()),
                ));
                false
            }
        };
        fixture_checks.push(json!({ "path": path.display().to_string(), "ok": ok }));
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.http.contracts.validate.report@0.1.0",
      "command": "x07-wasm.http.contracts.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "schemas": schema_checks,
        "fixtures": fixture_checks,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn discover_default_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir = Path::new("spec/fixtures/http");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn fixture_schema_id(doc: &Value) -> Option<&'static str> {
    if let Some(kind) = doc.get("kind").and_then(Value::as_str) {
        return match kind {
            "x07.http.dispatch" => Some("https://x07.io/spec/x07-http.dispatch.schema.json"),
            "x07.http.frame" => Some("https://x07.io/spec/x07-http.frame.schema.json"),
            "x07.http.trace" => Some("https://x07.io/spec/x07-http.trace.schema.json"),
            _ => None,
        };
    }
    None
}

fn validate_fixture(
    store: &SchemaStore,
    path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<bool> {
    match util::file_digest(path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_FIXTURE_VALIDATE_FAILED",
            Severity::Error,
            Stage::Parse,
            format!("failed to read fixture {}: {err:#}", path.display()),
        )),
    }
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let Some(schema_id) = fixture_schema_id(&doc) else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_FIXTURE_SCHEMA_UNKNOWN",
            Severity::Error,
            Stage::Parse,
            format!("unable to infer schema for fixture: {}", path.display()),
        ));
        return Ok(false);
    };
    let diags = store.validate(schema_id, &doc)?;
    let ok = diags.is_empty();
    diagnostics.extend(diags);
    Ok(ok)
}

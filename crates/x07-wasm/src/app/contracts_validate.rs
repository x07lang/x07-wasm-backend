use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{AppContractsValidateArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_app_contracts_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppContractsValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let schema_ids: Vec<&'static str> = vec![
        "https://x07.io/spec/x07-arch.app.index.schema.json",
        "https://x07.io/spec/x07-app.profile.schema.json",
        "https://x07.io/spec/x07-app.bundle.schema.json",
        "https://x07.io/spec/x07-app.backend.request.schema.json",
        "https://x07.io/spec/x07-app.backend.response.schema.json",
        "https://x07.io/spec/x07-http.request.envelope.schema.json",
        "https://x07.io/spec/x07-http.response.envelope.schema.json",
        "https://x07.io/spec/x07-app.trace.schema.json",
        "https://x07.io/spec/x07-web_ui.dispatch.schema.json",
        "https://x07.io/spec/x07-web_ui.tree.schema.json",
        "https://x07.io/spec/x07-web_ui.patchset.schema.json",
        "https://x07.io/spec/x07-web_ui.frame.schema.json",
        "https://x07.io/spec/x07-wasm.app.contracts.validate.report.schema.json",
    ];

    let mut schema_checks: Vec<Value> = Vec::new();
    for id in schema_ids {
        let ok = store.schema(id).is_ok();
        if !ok {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_SCHEMA_MISSING",
                Severity::Error,
                Stage::Run,
                format!("missing embedded schema: {id:?}"),
            ));
        }
        schema_checks.push(json!({ "id": id, "ok": ok }));
    }

    let fixtures = if !args.fixture.is_empty() {
        args.fixture.clone()
    } else {
        discover_default_fixtures()
    };

    let mut fixture_checks: Vec<Value> = Vec::new();
    for path in fixtures {
        let ok = match validate_fixture(&store, &path, &mut meta, &mut diagnostics) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_FIXTURE_VALIDATE_FAILED",
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
      "schema_version": "x07.wasm.app.contracts.validate.report@0.1.0",
      "command": "x07-wasm.app.contracts.validate",
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
    vec![
        PathBuf::from("arch/app/index.x07app.json"),
        PathBuf::from("arch/app/profiles/app_dev.json"),
        PathBuf::from("arch/app/profiles/app_release.json"),
        PathBuf::from("arch/app/contracts/bundle.example.json"),
        PathBuf::from("arch/app/contracts/backend.request.example.json"),
        PathBuf::from("arch/app/contracts/backend.response.example.json"),
        PathBuf::from("arch/app/contracts/trace.example.json"),
    ]
}

fn fixture_schema_id(doc: &Value) -> Option<&'static str> {
    if let Some(schema_version) = doc.get("schema_version").and_then(Value::as_str) {
        return match schema_version {
            "x07.arch.app.index@0.1.0" => {
                Some("https://x07.io/spec/x07-arch.app.index.schema.json")
            }
            "x07.app.profile@0.2.0" => Some("https://x07.io/spec/x07-app.profile.schema.json"),
            "x07.app.bundle@0.1.0" => Some("https://x07.io/spec/x07-app.bundle.schema.json"),
            "x07.app.backend.request@0.1.0" => {
                Some("https://x07.io/spec/x07-app.backend.request.schema.json")
            }
            "x07.app.backend.response@0.1.0" => {
                Some("https://x07.io/spec/x07-app.backend.response.schema.json")
            }
            "x07.app.trace@0.1.0" => Some("https://x07.io/spec/x07-app.trace.schema.json"),
            "x07.http.request.envelope@0.1.0" => {
                Some("https://x07.io/spec/x07-http.request.envelope.schema.json")
            }
            "x07.http.response.envelope@0.1.0" => {
                Some("https://x07.io/spec/x07-http.response.envelope.schema.json")
            }
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
            "X07WASM_APP_FIXTURE_READ_FAILED",
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
            "X07WASM_APP_FIXTURE_SCHEMA_UNKNOWN",
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

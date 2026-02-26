use std::ffi::OsString;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{AppRegressFromIncidentArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_app_regress_from_incident(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppRegressFromIncidentArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut generated: Vec<report::meta::FileDigest> = Vec::new();

    if !args.incident_dir.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_REGRESS_INCIDENT_DIR_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("incident dir not found: {}", args.incident_dir.display()),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
        );
    }

    let trace_path = args.incident_dir.join("trace.json");
    if !trace_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_REGRESS_TRACE_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("missing incident trace: {}", trace_path.display()),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
        );
    }

    if let Ok(d) = util::file_digest(&trace_path) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(&trace_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_REGRESS_TRACE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read trace.json: {err}"),
            ));
            Vec::new()
        }
    };
    let trace_doc: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_REGRESS_TRACE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("trace.json is not JSON: {err}"),
            ));
            Value::Null
        }
    };

    let diag_before = diagnostics.len();
    diagnostics
        .extend(store.validate("https://x07.io/spec/x07-app.trace.schema.json", &trace_doc)?);
    if diagnostics[diag_before..]
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
        );
    }

    if !args.dry_run {
        std::fs::create_dir_all(&args.out_dir)
            .with_context(|| format!("create dir: {}", args.out_dir.display()))?;

        let out_trace = args.out_dir.join(format!("{}.trace.json", args.name));
        let out_bytes = report::canon::canonical_pretty_json_bytes(&trace_doc)?;
        std::fs::write(&out_trace, out_bytes)
            .with_context(|| format!("write: {}", out_trace.display()))?;
        let d = util::file_digest(&out_trace)?;
        meta.outputs.push(d.clone());
        generated.push(d);

        // Optional: write a convenience snapshot of the final UI tree.
        if let Some(ui) = final_ui_tree(&trace_doc) {
            let out_ui = args.out_dir.join(format!("{}.final.ui.json", args.name));
            let bytes = report::canon::canonical_pretty_json_bytes(&ui)?;
            if std::fs::write(&out_ui, bytes).is_ok() {
                if let Ok(d) = util::file_digest(&out_ui) {
                    meta.outputs.push(d.clone());
                    generated.push(d);
                }
            }
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args,
        generated,
    )
}

fn final_ui_tree(trace_doc: &Value) -> Option<Value> {
    let steps = trace_doc.get("steps").and_then(Value::as_array)?;
    let last = steps.last()?;
    let frame = last.get("ui_frame")?;
    frame.get("ui").cloned()
}

#[allow(clippy::too_many_arguments)]
fn emit_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    args: &AppRegressFromIncidentArgs,
    generated: Vec<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let stdout_json = json!({
      "incident_dir": args.incident_dir.display().to_string(),
      "out_dir": args.out_dir.display().to_string(),
      "dry_run": args.dry_run,
      "generated": generated
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.app.regress.from_incident.report@0.1.0",
      "command": "x07-wasm.app.regress.from_incident",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "stdout": { "bytes_len": 0 },
        "stderr": { "bytes_len": 0 },
        "stdout_json": stdout_json
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

use std::ffi::OsString;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, WebUiRegressFromIncidentArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_web_ui_regress_from_incident(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WebUiRegressFromIncidentArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut generated: Vec<report::meta::FileDigest> = Vec::new();

    if let Ok(d) = util::file_digest(&args.incident) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(&args.incident) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_INCIDENT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read incident {}: {err}", args.incident.display()),
            ));
            return emit_regress_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args,
                generated,
                args.strict,
            );
        }
    };

    let doc: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_INCIDENT_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse incident JSON: {err}"),
            ));
            return emit_regress_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args,
                generated,
                args.strict,
            );
        }
    };

    let kind = doc.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind != "x07.web_ui.incident" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_INCIDENT_KIND_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unexpected incident.kind: {kind:?}"),
        ));
        return emit_regress_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
            args.strict,
        );
    }

    let trace = doc.get("trace").cloned().unwrap_or(Value::Null);
    let mut trace_clean = trace.clone();

    // Normalize unstable fields for deterministic fixtures.
    let mut normalized_started_at = false;
    if let Some(meta_obj) = trace_clean.get_mut("meta").and_then(Value::as_object_mut) {
        if meta_obj.contains_key("startedAtUnixMs") {
            meta_obj.insert("startedAtUnixMs".to_string(), json!(0));
            normalized_started_at = true;
        }
    }
    let mut removed_wall_ms = 0usize;
    if let Some(steps) = trace_clean.get_mut("steps").and_then(Value::as_array_mut) {
        for step in steps.iter_mut() {
            if let Some(obj) = step.as_object_mut() {
                if obj.remove("wallMs").is_some() {
                    removed_wall_ms += 1;
                }
            }
        }
    }

    let diags = store.validate(
        "https://x07.io/spec/x07-web_ui.trace.schema.json",
        &trace_clean,
    )?;
    diagnostics.extend(diags);

    if normalized_started_at {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_INCIDENT_NORMALIZED_STARTED_AT",
            Severity::Warning,
            Stage::Parse,
            "normalized trace.meta.startedAtUnixMs to 0".to_string(),
        ));
    }
    if removed_wall_ms > 0 {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_INCIDENT_NORMALIZED_WALLMS",
            Severity::Warning,
            Stage::Parse,
            format!("removed wallMs from {removed_wall_ms} trace step(s)"),
        ));
    }

    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return emit_regress_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
            args.strict,
        );
    }

    if let Err(err) = std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create dir: {}", args.out_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_REGRESS_OUTDIR_CREATE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        return emit_regress_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
            args.strict,
        );
    }

    let out_trace = args.out_dir.join(format!("{}.trace.json", args.name));
    let bytes = report::canon::canonical_pretty_json_bytes(&trace_clean)?;
    if let Err(err) = std::fs::write(&out_trace, &bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_REGRESS_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to write {}: {err}", out_trace.display()),
        ));
    } else {
        let d = util::file_digest(&out_trace)?;
        meta.outputs.push(d.clone());
        generated.push(d);
    }

    // Emit a snapshot of the last UI tree (if present).
    if let Some(steps) = trace_clean.get("steps").and_then(Value::as_array) {
        if let Some(last) = steps.last() {
            if let Some(ui) = last.get("frame").and_then(|f| f.get("ui")) {
                let out_ui = args.out_dir.join(format!("{}.final.ui.json", args.name));
                let bytes = report::canon::canonical_pretty_json_bytes(ui)?;
                if std::fs::write(&out_ui, &bytes).is_ok() {
                    let d = util::file_digest(&out_ui)?;
                    meta.outputs.push(d.clone());
                    generated.push(d);
                }
            }
        }
    }

    emit_regress_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args,
        generated,
        args.strict,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_regress_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    mut diagnostics: Vec<Diagnostic>,
    args: &WebUiRegressFromIncidentArgs,
    generated: Vec<report::meta::FileDigest>,
    strict: bool,
) -> Result<u8> {
    if strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.web_ui.regress.from.incident.report@0.1.0",
      "command": "x07-wasm.web-ui.regress-from-incident",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "incident": args.incident.display().to_string(),
        "out_dir": args.out_dir.display().to_string(),
        "generated": generated,
      }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

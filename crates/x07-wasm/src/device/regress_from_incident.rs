use std::ffi::OsString;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{DeviceRegressFromIncidentArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_device_regress_from_incident(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceRegressFromIncidentArgs,
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
            "X07WASM_DEVICE_REGRESS_INCIDENT_DIR_MISSING",
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

    let incident_path = args.incident_dir.join("incident.json");
    if let Ok(d) = util::file_digest(&incident_path) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(&incident_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_REGRESS_INCIDENT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read incident.json: {err}"),
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
    };
    let incident_doc: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_REGRESS_INCIDENT_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("incident.json is not JSON: {err}"),
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
    };

    let kind = incident_doc
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != "x07.web_ui.incident" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_INCIDENT_KIND_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unexpected incident.kind: {kind:?}"),
        ));
    }

    let trace_doc = incident_doc.get("trace").cloned().unwrap_or(Value::Null);
    if trace_doc == Value::Null {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_TRACE_MISSING",
            Severity::Error,
            Stage::Parse,
            "device incident is missing trace".to_string(),
        ));
    } else {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-web_ui.trace.schema.json",
            &trace_doc,
        )?);
    }

    let app_trace_doc = incident_doc.get("appTrace").cloned().unwrap_or(Value::Null);
    if app_trace_doc != Value::Null {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-app.trace.schema.json",
            &app_trace_doc,
        )?);
    }

    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        if args.strict {
            for d in diagnostics.iter_mut() {
                if d.severity == Severity::Warning {
                    d.severity = Severity::Error;
                }
            }
        }
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

        let incident_out = args.out_dir.join(format!("{}.incident.json", args.name));
        std::fs::write(
            &incident_out,
            report::canon::canonical_pretty_json_bytes(&incident_doc)?,
        )
        .with_context(|| format!("write: {}", incident_out.display()))?;
        let incident_digest = util::file_digest(&incident_out)?;
        meta.outputs.push(incident_digest.clone());
        generated.push(incident_digest);

        let trace_out = args.out_dir.join(format!("{}.trace.json", args.name));
        std::fs::write(
            &trace_out,
            report::canon::canonical_pretty_json_bytes(&trace_doc)?,
        )
        .with_context(|| format!("write: {}", trace_out.display()))?;
        let trace_digest = util::file_digest(&trace_out)?;
        meta.outputs.push(trace_digest.clone());
        generated.push(trace_digest);

        if app_trace_doc != Value::Null {
            let app_trace_out = args.out_dir.join(format!("{}.app.trace.json", args.name));
            std::fs::write(
                &app_trace_out,
                report::canon::canonical_pretty_json_bytes(&app_trace_doc)?,
            )
            .with_context(|| format!("write: {}", app_trace_out.display()))?;
            let app_trace_digest = util::file_digest(&app_trace_out)?;
            meta.outputs.push(app_trace_digest.clone());
            generated.push(app_trace_digest);
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

#[allow(clippy::too_many_arguments)]
fn emit_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    args: &DeviceRegressFromIncidentArgs,
    generated: Vec<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.device.regress.from_incident.report@0.1.0",
      "command": "x07-wasm.device.regress.from-incident",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "incident_dir": args.incident_dir.display().to_string(),
        "out_dir": args.out_dir.display().to_string(),
        "name": args.name,
        "dry_run": args.dry_run,
        "generated": generated
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

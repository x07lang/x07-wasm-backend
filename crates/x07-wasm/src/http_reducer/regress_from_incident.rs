use std::ffi::OsString;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{HttpRegressFromIncidentArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_http_regress_from_incident(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: HttpRegressFromIncidentArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let incident_json = args.incident_dir.join("incident.json");
    if let Ok(d) = util::file_digest(&incident_json) {
        meta.inputs.push(d);
    }

    let bytes = match std::fs::read(&incident_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_INCIDENT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read incident {}: {err}", incident_json.display()),
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
                None,
                None,
                Vec::new(),
            );
        }
    };

    let doc: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_INCIDENT_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse incident JSON: {err}"),
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
                None,
                None,
                Vec::new(),
            );
        }
    };

    let kind = doc.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind != "x07.http.incident" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_INCIDENT_KIND_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unexpected incident.kind: {kind:?}"),
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
            None,
            None,
            Vec::new(),
        );
    }

    let trace = doc.get("trace").cloned().unwrap_or(Value::Null);
    diagnostics.extend(store.validate("https://x07.io/spec/x07-http.trace.schema.json", &trace)?);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            None,
            None,
            Vec::new(),
        );
    }

    if let Err(err) = std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create dir: {}", args.out_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_REGRESS_OUTDIR_CREATE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
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
            None,
            None,
            Vec::new(),
        );
    }

    let seed = format!("http-regress:{}", args.incident_dir.display());
    let base = util::sha256_hex(seed.as_bytes());
    let base = base.chars().take(16).collect::<String>();

    let fixture_path = args.out_dir.join(format!("{base}.trace.json"));
    let fixture_bytes = report::canon::canonical_pretty_json_bytes(&trace)?;
    if let Err(err) = std::fs::write(&fixture_path, &fixture_bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_REGRESS_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to write fixture {}: {err}", fixture_path.display()),
        ));
    }

    let test_path = args.out_dir.join(format!("{base}.sh"));
    let component_path = doc
        .get("inputs")
        .and_then(|v| v.get("component"))
        .and_then(|v| v.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("dist/http_reducer.wasm");
    let test_bytes = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nx07-wasm http test --component \"{}\" --trace \"{}\" --json --quiet-json\n",
        component_path,
        fixture_path.display()
    );
    if let Err(err) = std::fs::write(&test_path, test_bytes.as_bytes()) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_REGRESS_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to write test {}: {err}", test_path.display()),
        ));
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if let Ok(mut perms) = std::fs::metadata(&test_path).map(|m| m.permissions()) {
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&test_path, perms);
            }
        }
    }

    let generated_fixture = util::file_digest(&fixture_path).ok();
    let generated_test = util::file_digest(&test_path).ok();

    if let Some(d) = generated_fixture.clone() {
        meta.outputs.push(d);
    }
    if let Some(d) = generated_test.clone() {
        meta.outputs.push(d);
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
        generated_fixture,
        generated_test,
        Vec::new(),
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
    args: &HttpRegressFromIncidentArgs,
    generated_fixture: Option<report::meta::FileDigest>,
    generated_test: Option<report::meta::FileDigest>,
    updated_files: Vec<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.http.regress.from.incident.report@0.1.0",
      "command": "x07-wasm.http.regress.from-incident",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "incident_dir": args.incident_dir.display().to_string(),
        "generated_fixture": generated_fixture.unwrap_or(report::meta::FileDigest {
          path: args.out_dir.join("generated.trace.json").display().to_string(),
          sha256: "0".repeat(64),
          bytes_len: 0,
        }),
        "generated_test": generated_test.unwrap_or(report::meta::FileDigest {
          path: args.out_dir.join("generated.sh").display().to_string(),
          sha256: "0".repeat(64),
          bytes_len: 0,
        }),
        "updated_files": updated_files,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

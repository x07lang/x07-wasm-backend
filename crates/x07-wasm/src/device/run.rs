use std::ffi::OsString;
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{DeviceRunArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::machine::{self, JsonMode};
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_RUN_REPORT_SCHEMA_ID: &str =
    "https://x07.io/spec/x07-wasm.device.run.report.schema.json";

pub fn cmd_device_run(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceRunArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let target = args.target.trim();
    if target != "desktop" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_RUN_TARGET_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!("unsupported device run target: {target:?}"),
        ));
        return emit_device_run_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            diagnostics,
            meta,
            &args,
            Value::Null,
        );
    }

    let manifest_path = args.bundle.join("bundle.manifest.json");
    if let Ok(d) = util::file_digest(&manifest_path) {
        meta.inputs.push(d);
    }

    let json_mode = machine::json_mode(machine).map_err(anyhow::Error::msg)?;

    let tool = device_host_desktop_tool();
    let mut cmd = Command::new(&tool);
    cmd.arg("run");
    cmd.arg("--bundle");
    cmd.arg(&args.bundle);
    if args.headless_smoke {
        cmd.arg("--headless-smoke");
    }

    if json_mode == JsonMode::Off {
        let status = cmd.status();
        return Ok(exit_code_from_status(
            status.context("spawn x07-device-host-desktop")?,
        ));
    }

    cmd.arg("--json");
    let output = match cmd.output() {
        Ok(v) => v,
        Err(err) => {
            let code = if err.kind() == std::io::ErrorKind::NotFound {
                "X07WASM_DEVICE_RUN_HOST_TOOL_MISSING"
            } else {
                "X07WASM_DEVICE_RUN_FAILED"
            };
            diagnostics.push(Diagnostic::new(
                code,
                Severity::Error,
                Stage::Run,
                format!("failed to spawn x07-device-host-desktop: {err}"),
            ));
            return emit_device_run_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                diagnostics,
                meta,
                &args,
                Value::Null,
            );
        }
    };

    let stdout = output.stdout;
    let stderr = output.stderr;
    let status_code = exit_code_from_status(output.status);

    let host_doc: Value = match serde_json::from_slice(&stdout) {
        Ok(v) => v,
        Err(err) => {
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_RUN_HOST_REPORT_PARSE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to parse host JSON report: {err}"),
            );
            d.data
                .insert("host_exit_code".to_string(), json!(status_code));
            if !stdout.is_empty() {
                d.data.insert(
                    "host_stdout".to_string(),
                    json!(util::truncate_bytes_lossy(&stdout, 2000)),
                );
            }
            if !stderr.is_empty() {
                d.data.insert(
                    "host_stderr".to_string(),
                    json!(util::truncate_bytes_lossy(&stderr, 2000)),
                );
            }
            diagnostics.push(d);
            return emit_device_run_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                diagnostics,
                meta,
                &args,
                Value::Null,
            );
        }
    };

    let schema_diags = store.validate(DEVICE_RUN_REPORT_SCHEMA_ID, &host_doc)?;
    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_RUN_HOST_REPORT_SCHEMA_INVALID",
            Severity::Error,
            Stage::Run,
            "host report schema invalid".to_string(),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        d.data
            .insert("host_exit_code".to_string(), json!(status_code));
        diagnostics.push(d);
        return emit_device_run_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            diagnostics,
            meta,
            &args,
            Value::Null,
        );
    }

    let exit_code = host_doc
        .get("exit_code")
        .and_then(Value::as_u64)
        .unwrap_or(status_code as u64)
        .min(255) as u8;

    store.validate_report_and_emit(scope, machine, started, raw_argv, host_doc)?;
    Ok(exit_code)
}

fn emit_device_run_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    diagnostics: Vec<Diagnostic>,
    meta: report::meta::ReportMeta,
    args: &DeviceRunArgs,
    host_doc: Value,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let host_tool = if let Some(s) = host_doc.get("result").and_then(|r| r.get("host_tool")) {
        s.as_str().unwrap_or("x07-device-host-desktop").to_string()
    } else {
        "x07-device-host-desktop".to_string()
    };

    let ui_ready = host_doc
        .get("result")
        .and_then(|r| r.get("ui_ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let report_doc = json!({
      "schema_version": "x07.wasm.device.run.report@0.1.0",
      "command": "x07-wasm.device.run",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "bundle_dir": args.bundle.display().to_string(),
        "host_tool": host_tool,
        "ui_ready": ui_ready,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn device_host_desktop_tool() -> String {
    if let Some(p) = std::env::var_os("X07_DEVICE_HOST_DESKTOP") {
        if !p.is_empty() {
            return p.to_string_lossy().to_string();
        }
    }
    "x07-device-host-desktop".to_string()
}

fn exit_code_from_status(status: std::process::ExitStatus) -> u8 {
    status.code().unwrap_or(2).clamp(0, 255) as u8
}

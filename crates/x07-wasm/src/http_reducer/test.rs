use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{HttpTestArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::web_ui::replay::CoreWasmRunner;

pub fn cmd_http_test(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: HttpTestArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let component_digest = match util::file_digest(&args.component) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_TEST_COMPONENT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to digest reducer wasm {}: {err:#}",
                    args.component.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                report::meta::FileDigest {
                    path: args.component.display().to_string(),
                    sha256: "0".repeat(64),
                    bytes_len: 0,
                },
                crate::arch::WasmRuntimeLimits {
                    max_fuel: None,
                    max_memory_bytes: None,
                    max_table_elements: None,
                    max_wasm_stack_bytes: None,
                    notes: None,
                },
                Vec::new(),
            );
        }
    };

    let loaded_wasm_profile = match crate::arch::load_profile(
        &store,
        &PathBuf::from("arch/wasm/index.x07wasm.json"),
        None,
        None,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
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
                component_digest,
                crate::arch::WasmRuntimeLimits {
                    max_fuel: None,
                    max_memory_bytes: None,
                    max_table_elements: None,
                    max_wasm_stack_bytes: None,
                    notes: None,
                },
                Vec::new(),
            );
        }
    };
    meta.inputs.push(loaded_wasm_profile.digest.clone());
    if let Some(d) = loaded_wasm_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let mut runtime_limits = loaded_wasm_profile.doc.runtime.clone();
    if runtime_limits.max_wasm_stack_bytes.is_none() {
        runtime_limits.max_wasm_stack_bytes = Some(2 * 1024 * 1024);
    }

    let arena_cap_bytes =
        u32::try_from(loaded_wasm_profile.doc.defaults.arena_cap_bytes).unwrap_or(32 * 1024 * 1024);
    let max_output_bytes =
        u32::try_from(loaded_wasm_profile.doc.defaults.max_output_bytes).unwrap_or(2 * 1024 * 1024);

    let mut core = match CoreWasmRunner::new(
        &args.component,
        &runtime_limits,
        arena_cap_bytes,
        max_output_bytes,
        &mut meta,
        &mut diagnostics,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_TEST_COMPONENT_LOAD_FAILED",
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
                component_digest,
                runtime_limits,
                Vec::new(),
            );
        }
    };

    let mut cases: Vec<PathBuf> = args.trace.clone();
    cases.sort();
    if cases.is_empty() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_TEST_TRACE_MISSING",
            Severity::Error,
            Stage::Parse,
            "no --trace cases provided".to_string(),
        ));
    }

    for c in &cases {
        if let Ok(d) = util::file_digest(c) {
            meta.inputs.push(d);
        }
    }

    let mut case_results: Vec<Value> = Vec::new();
    let mut cases_ok: u64 = 0;
    let mut cases_failed: u64 = 0;

    for case_path in cases {
        let name = case_stem(&case_path);
        let (ok, incident_dir) =
            match run_trace_case(&store, &mut core, &case_path, &mut diagnostics) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_TEST_CASE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("case {name:?} failed: {err:#}"),
                    ));
                    (false, None)
                }
            };
        if ok {
            cases_ok += 1;
        } else {
            cases_failed += 1;
        }
        case_results.push(json!({
          "name": name,
          "ok": ok,
          "trace_path": case_path.display().to_string(),
          "incident_dir": incident_dir.map(|p| p.display().to_string()),
        }));
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.http.test.report@0.1.0",
      "command": "x07-wasm.http.test",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "component": component_digest,
        "runtime_limits": runtime_limits,
        "cases": case_results,
        "cases_ok": cases_ok,
        "cases_failed": cases_failed,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn case_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "case".to_string())
}

fn run_trace_case(
    store: &SchemaStore,
    core: &mut CoreWasmRunner,
    trace_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(bool, Option<PathBuf>)> {
    let bytes =
        std::fs::read(trace_path).with_context(|| format!("read: {}", trace_path.display()))?;
    let trace: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", trace_path.display()))?;

    diagnostics.extend(store.validate("https://x07.io/spec/x07-http.trace.schema.json", &trace)?);

    let steps = trace
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut ok = true;
    let mut failed_step: Option<usize> = None;
    let mut failed_env: Option<Value> = None;
    let mut failed_expected_frame: Option<Value> = None;
    let mut failed_actual_frame: Option<Value> = None;

    for (i, step) in steps.into_iter().enumerate() {
        let env = step.get("env").cloned().unwrap_or(Value::Null);
        let expected_frame = step.get("frame").cloned().unwrap_or(Value::Null);

        let env_diags =
            store.validate("https://x07.io/spec/x07-http.dispatch.schema.json", &env)?;
        if env_diags.iter().any(|d| d.severity == Severity::Error) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_DISPATCH_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("step {i}: dispatch schema invalid"),
            ));
            diagnostics.extend(env_diags);
            ok = false;
            failed_step = Some(i);
            failed_env = Some(env);
            failed_expected_frame = Some(expected_frame);
            break;
        }

        let input_bytes = crate::report::canon::canonical_json_bytes(&env)?;
        let out_bytes = match core.call(&input_bytes) {
            Ok(v) => v,
            Err(err) => {
                if let Some(kind) = crate::wasmtime_limits::classify_budget_exceeded(&err) {
                    let (code, msg) = match kind {
                        crate::wasmtime_limits::BudgetExceededKind::CpuFuel => (
                            "X07WASM_BUDGET_EXCEEDED_CPU_FUEL",
                            "execution exceeded Wasmtime fuel budget",
                        ),
                        crate::wasmtime_limits::BudgetExceededKind::WasmStack => (
                            "X07WASM_BUDGET_EXCEEDED_WASM_STACK",
                            "execution exceeded Wasmtime wasm stack budget",
                        ),
                        crate::wasmtime_limits::BudgetExceededKind::Memory => (
                            "X07WASM_BUDGET_EXCEEDED_MEMORY",
                            "execution exceeded Wasmtime memory budget",
                        ),
                        crate::wasmtime_limits::BudgetExceededKind::Table => (
                            "X07WASM_BUDGET_EXCEEDED_TABLE",
                            "execution exceeded Wasmtime table budget",
                        ),
                    };
                    diagnostics.push(Diagnostic::new(
                        code,
                        Severity::Error,
                        Stage::Run,
                        msg.to_string(),
                    ));
                } else {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_TEST_CALL_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("step {i}: {err:#}"),
                    ));
                }
                ok = false;
                failed_step = Some(i);
                failed_env = Some(env);
                failed_expected_frame = Some(expected_frame);
                break;
            }
        };

        let actual_frame: Value = match serde_json::from_slice(&out_bytes) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_HTTP_FRAME_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("step {i}: frame is not JSON: {err}"),
                ));
                ok = false;
                failed_step = Some(i);
                failed_env = Some(env);
                failed_expected_frame = Some(expected_frame);
                break;
            }
        };

        let frame_diags = store.validate(
            "https://x07.io/spec/x07-http.frame.schema.json",
            &actual_frame,
        )?;
        if frame_diags.iter().any(|d| d.severity == Severity::Error) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_FRAME_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("step {i}: frame schema invalid"),
            ));
            diagnostics.extend(frame_diags);
            ok = false;
            failed_step = Some(i);
            failed_env = Some(env);
            failed_expected_frame = Some(expected_frame);
            failed_actual_frame = Some(actual_frame);
            break;
        }

        let expected_bytes = crate::report::canon::canonical_json_bytes(&expected_frame)?;
        let actual_bytes = crate::report::canon::canonical_json_bytes(&actual_frame)?;
        if expected_bytes != actual_bytes {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_TEST_FRAME_MISMATCH",
                Severity::Error,
                Stage::Run,
                format!("step {i}: frame mismatch"),
            ));
            ok = false;
            failed_step = Some(i);
            failed_env = Some(env);
            failed_expected_frame = Some(expected_frame);
            failed_actual_frame = Some(actual_frame);
            break;
        }
    }

    if ok {
        return Ok((true, None));
    }

    let incident_dir = write_http_test_incident(
        trace_path,
        failed_step,
        failed_env.as_ref(),
        failed_expected_frame.as_ref(),
        failed_actual_frame.as_ref(),
        diagnostics,
    );
    Ok((false, incident_dir))
}

fn write_http_test_incident(
    trace_path: &Path,
    failed_step: Option<usize>,
    failed_env: Option<&Value>,
    failed_expected_frame: Option<&Value>,
    failed_actual_frame: Option<&Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let case_digest = util::file_digest(trace_path).ok();
    let case_sha = case_digest
        .as_ref()
        .map(|d| d.sha256.as_str())
        .unwrap_or("");
    let step = failed_step.unwrap_or(0);
    let seed = format!("http-test:{case_sha}:{step}");
    let id = util::sha256_hex(seed.as_bytes());
    let id = id.chars().take(32).collect::<String>();

    let dir = PathBuf::from(".x07-wasm/incidents")
        .join("http-test")
        .join(id);
    if std::fs::create_dir_all(&dir).is_err() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_HTTP_TEST_INCIDENT_DIR_CREATE_FAILED",
            Severity::Warning,
            Stage::Run,
            format!("failed to create incident dir {}", dir.display()),
        ));
        return None;
    }

    let trace: Value = match std::fs::read(trace_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(v) => v,
        None => Value::Null,
    };

    let doc = json!({
      "v": 1,
      "kind": "x07.http.incident",
      "error": "http test failure",
      "trace": trace,
      "failed": {
        "case": trace_path.display().to_string(),
        "step": failed_step,
        "env": failed_env,
        "expected_frame": failed_expected_frame,
        "actual_frame": failed_actual_frame,
      },
    });

    let incident_path = dir.join("incident.json");
    let bytes = match report::canon::canonical_pretty_json_bytes(&doc) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if std::fs::write(&incident_path, bytes).is_err() {
        return None;
    }
    Some(dir)
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
    component: report::meta::FileDigest,
    runtime_limits: crate::arch::WasmRuntimeLimits,
    cases: Vec<Value>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let mut cases_ok: u64 = 0;
    let mut cases_failed: u64 = 0;
    for c in &cases {
        if c.get("ok").and_then(Value::as_bool) == Some(true) {
            cases_ok += 1;
        } else {
            cases_failed += 1;
        }
    }

    let report_doc = json!({
      "schema_version": "x07.wasm.http.test.report@0.1.0",
      "command": "x07-wasm.http.test",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "component": component,
        "runtime_limits": runtime_limits,
        "cases": cases,
        "cases_ok": cases_ok,
        "cases_failed": cases_failed,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

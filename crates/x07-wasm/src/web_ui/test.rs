use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, WebUiTestArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::web_ui::replay;

pub fn cmd_web_ui_test(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WebUiTestArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let incident_dir = args.incidents_dir.display().to_string();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut case_results: Vec<Value> = Vec::new();

    if !args.dist_dir.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_DIST_DIR_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("dist dir not found: {}", args.dist_dir.display()),
        ));
        return emit_test_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            case_results,
            Some(incident_dir.clone()),
            args.strict,
        );
    }

    let wasm_path = args.dist_dir.join("app.wasm");
    if !wasm_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_WASM_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("missing wasm: {}", wasm_path.display()),
        ));
        return emit_test_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            case_results,
            Some(incident_dir.clone()),
            args.strict,
        );
    }

    let (arena_cap_bytes, max_output_bytes) =
        replay::load_web_ui_budgets(&store, &args.dist_dir, &mut meta, &mut diagnostics);
    let Some(runtime_limits) =
        replay::load_wasm_runtime_limits(&store, &args.dist_dir, &mut meta, &mut diagnostics)
    else {
        return emit_test_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            case_results,
            Some(incident_dir.clone()),
            args.strict,
        );
    };

    let mut core = match replay::CoreWasmRunner::new(
        &wasm_path,
        &runtime_limits,
        arena_cap_bytes,
        max_output_bytes,
        &mut meta,
        &mut diagnostics,
    ) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_WASM_LOAD_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            None
        }
    };

    let transpiled_mjs = args.dist_dir.join("transpiled").join("app.mjs");
    let has_component_esm = transpiled_mjs.is_file();

    if args.case.is_empty() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_CASE_MISSING",
            Severity::Error,
            Stage::Parse,
            "no --case provided".to_string(),
        ));
    }

    for case_path in &args.case {
        let mut case_ok = true;
        let mut snapshot_path: Option<String> = None;
        let mut case_error: Option<String> = None;
        let mut observed_steps: Vec<Value> = Vec::new();
        let mut failed_step: Option<usize> = None;
        let mut failed_env: Option<Value> = None;
        let mut failed_expected_frame: Option<Value> = None;
        let mut failed_actual_frame: Option<Value> = None;

        let case_digest = match util::file_digest(case_path) {
            Ok(d) => {
                meta.inputs.push(d.clone());
                Some(d)
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_TEST_CASE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to digest case {}: {err:#}", case_path.display()),
                ));
                None
            }
        };

        let bytes = match std::fs::read(case_path) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_TEST_CASE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to read case {}: {err}", case_path.display()),
                ));
                case_results.push(json!({"path": case_path.display().to_string(), "ok": false, "snapshot_path": null}));
                continue;
            }
        };
        let mut trace: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_TEST_CASE_JSON_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse JSON {}: {err}", case_path.display()),
                ));
                case_results.push(json!({"path": case_path.display().to_string(), "ok": false, "snapshot_path": null}));
                continue;
            }
        };

        let diag_before = diagnostics.len();
        diagnostics
            .extend(store.validate("https://x07.io/spec/x07-web_ui.trace.schema.json", &trace)?);
        if diagnostics[diag_before..]
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            case_ok = false;
        }

        let steps = trace
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .cloned()
            .unwrap_or_default();

        let Some(core) = core.as_mut() else {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_WASM_NOT_AVAILABLE",
                Severity::Error,
                Stage::Run,
                "core wasm runner unavailable".to_string(),
            ));
            case_results.push(json!({"path": case_path.display().to_string(), "ok": false, "snapshot_path": null}));
            continue;
        };

        // Core-wasm replay.
        let mut updated = false;
        let mut prev_ui: Option<Value> = None;

        let max_steps = args.max_steps as usize;
        for (i, step) in steps.into_iter().enumerate().take(max_steps) {
            let env = step.get("env").cloned().unwrap_or(Value::Null);
            let expected_frame = step.get("frame").cloned().unwrap_or(Value::Null);

            let input_bytes = replay::canonical_json_bytes_no_newline(&env)?;
            let actual_bytes = match core.call(&input_bytes) {
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
                        case_error = Some(format!("step {i}: {msg}"));
                    } else {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_WEB_UI_TEST_CALL_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("step {i}: {err:#}"),
                        ));
                        case_error = Some(format!("step {i}: call failed: {err:#}"));
                    }
                    failed_step = Some(i);
                    failed_env = Some(env);
                    failed_expected_frame = Some(expected_frame);
                    case_ok = false;
                    break;
                }
            };
            let actual_frame: Value = match serde_json::from_slice(&actual_bytes) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_WEB_UI_TEST_FRAME_JSON_INVALID",
                        Severity::Error,
                        Stage::Run,
                        format!("step {i}: output is not JSON: {err}"),
                    ));
                    case_error = Some(format!("step {i}: output is not JSON: {err}"));
                    failed_step = Some(i);
                    failed_env = Some(env);
                    failed_expected_frame = Some(expected_frame);
                    case_ok = false;
                    break;
                }
            };
            let frame_diag_before = diagnostics.len();
            diagnostics.extend(store.validate(
                "https://x07.io/spec/x07-web_ui.frame.schema.json",
                &actual_frame,
            )?);
            if diagnostics[frame_diag_before..]
                .iter()
                .any(|d| d.severity == Severity::Error)
            {
                case_error = Some(format!("step {i}: frame schema invalid"));
                failed_step = Some(i);
                failed_env = Some(env);
                failed_expected_frame = Some(expected_frame);
                failed_actual_frame = Some(actual_frame);
                case_ok = false;
                break;
            }

            observed_steps.push(json!({ "env": env.clone(), "frame": actual_frame.clone() }));

            if let Some(ui) = actual_frame.get("ui") {
                let next_ui = ui.clone();
                if let Some(prev) = prev_ui.as_ref() {
                    if let Some(patches) = actual_frame.get("patches") {
                        match replay::apply_json_patch(prev.clone(), patches) {
                            Ok(patched) => {
                                let a = report::canon::canonical_json_bytes(&patched)?;
                                let b = report::canon::canonical_json_bytes(&next_ui)?;
                                if a != b {
                                    diagnostics.push(Diagnostic::new(
                                        "X07WASM_WEB_UI_TEST_PATCH_MISMATCH",
                                        Severity::Error,
                                        Stage::Run,
                                        format!("step {i}: patches do not match ui tree"),
                                    ));
                                    case_error =
                                        Some(format!("step {i}: patches do not match ui tree"));
                                    failed_step = Some(i);
                                    failed_env = Some(env);
                                    failed_expected_frame = Some(expected_frame);
                                    failed_actual_frame = Some(actual_frame);
                                    case_ok = false;
                                    break;
                                }
                            }
                            Err(err) => {
                                diagnostics.push(Diagnostic::new(
                                    "X07WASM_WEB_UI_TEST_PATCH_APPLY_FAILED",
                                    Severity::Error,
                                    Stage::Run,
                                    format!("step {i}: {err:#}"),
                                ));
                                case_error = Some(format!("step {i}: patch apply failed: {err:#}"));
                                failed_step = Some(i);
                                failed_env = Some(env);
                                failed_expected_frame = Some(expected_frame);
                                failed_actual_frame = Some(actual_frame);
                                case_ok = false;
                                break;
                            }
                        }
                    }
                }
                prev_ui = Some(next_ui);
            }

            let expected_bytes = report::canon::canonical_json_bytes(&expected_frame)?;
            let actual_bytes_canon = report::canon::canonical_json_bytes(&actual_frame)?;
            if expected_bytes != actual_bytes_canon {
                if args.update_golden {
                    if let Some(step_obj) = trace
                        .get_mut("steps")
                        .and_then(Value::as_array_mut)
                        .and_then(|arr| arr.get_mut(i))
                        .and_then(Value::as_object_mut)
                    {
                        step_obj.insert("frame".to_string(), actual_frame.clone());
                        step_obj.remove("wallMs");
                        updated = true;
                    }
                } else {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_WEB_UI_TEST_FRAME_MISMATCH",
                        Severity::Error,
                        Stage::Run,
                        format!("step {i}: frame mismatch"),
                    ));
                    case_error = Some(format!("step {i}: frame mismatch"));
                    failed_step = Some(i);
                    failed_env = Some(env);
                    failed_expected_frame = Some(expected_frame);
                    failed_actual_frame = Some(actual_frame);
                    case_ok = false;
                    break;
                }
            }
        }

        if args.update_golden && updated {
            let bytes = report::canon::canonical_pretty_json_bytes(&trace)?;
            if let Err(err) = std::fs::write(case_path, &bytes) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_TEST_GOLDEN_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to write case {}: {err}", case_path.display()),
                ));
                case_ok = false;
            } else if let Ok(d) = util::file_digest(case_path) {
                meta.outputs.push(d);
            }
        }

        // Write snapshot of final UI (if any) to dist_dir/test_snapshots/.
        if let Some(ui) = prev_ui {
            let snap_dir = args.dist_dir.join("test_snapshots");
            let _ = std::fs::create_dir_all(&snap_dir);
            let name = replay::case_stem(case_path);
            let snap_path = snap_dir.join(format!("{name}.ui.json"));
            if let Ok(bytes) = report::canon::canonical_pretty_json_bytes(&ui) {
                if std::fs::write(&snap_path, &bytes).is_ok() {
                    if let Ok(d) = util::file_digest(&snap_path) {
                        meta.outputs.push(d);
                    }
                    snapshot_path = Some(snap_path.display().to_string());
                }
            }
        }

        // Optional: component+esm replay via node if transpiled artifacts exist.
        if has_component_esm && case_ok {
            match run_component_trace_in_node(&args.dist_dir, case_path, args.max_steps) {
                Ok(frames) => {
                    if let Some(exp_steps) = trace.get("steps").and_then(Value::as_array) {
                        for (i, exp_step) in exp_steps.iter().enumerate().take(frames.len()) {
                            let expected_frame =
                                exp_step.get("frame").cloned().unwrap_or(Value::Null);
                            let actual_frame = frames[i].clone();
                            let expected_bytes =
                                report::canon::canonical_json_bytes(&expected_frame)?;
                            let actual_bytes_canon =
                                report::canon::canonical_json_bytes(&actual_frame)?;
                            if expected_bytes != actual_bytes_canon {
                                diagnostics.push(Diagnostic::new(
                                    "X07WASM_WEB_UI_TEST_COMPONENT_FRAME_MISMATCH",
                                    Severity::Error,
                                    Stage::Run,
                                    format!("component step {i}: frame mismatch"),
                                ));
                                case_error = Some(format!("component step {i}: frame mismatch"));
                                case_ok = false;
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_WEB_UI_TEST_COMPONENT_RUN_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                    case_error = Some(format!("component run failed: {err:#}"));
                    case_ok = false;
                }
            }
        }

        if !case_ok {
            let error = case_error
                .clone()
                .unwrap_or_else(|| "web-ui test failure".to_string());
            let wasm_digest = Some(&core.wasm);
            let trace_doc = json!({
            "v": 1,
            "kind": "x07.web_ui.trace",
            "steps": observed_steps,
            "meta": {
              "case": case_path.display().to_string(),
                  }
              });
            let _ = write_web_ui_test_incident(
                WebUiTestIncidentArgs {
                    incidents_dir: &args.incidents_dir,
                    wasm_digest,
                    case_digest: case_digest.as_ref(),
                    case_path,
                    error: &error,
                    trace: &trace_doc,
                    failed_step,
                    failed_env: failed_env.as_ref(),
                    failed_expected_frame: failed_expected_frame.as_ref(),
                    failed_actual_frame: failed_actual_frame.as_ref(),
                },
                &mut meta,
                &mut diagnostics,
            );
        }

        case_results.push(json!({
          "path": case_path.display().to_string(),
          "ok": case_ok,
          "snapshot_path": snapshot_path,
        }));
    }

    emit_test_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        case_results,
        Some(incident_dir),
        args.strict,
    )
}

struct WebUiTestIncidentArgs<'a> {
    incidents_dir: &'a Path,
    wasm_digest: Option<&'a report::meta::FileDigest>,
    case_digest: Option<&'a report::meta::FileDigest>,
    case_path: &'a Path,
    error: &'a str,
    trace: &'a Value,
    failed_step: Option<usize>,
    failed_env: Option<&'a Value>,
    failed_expected_frame: Option<&'a Value>,
    failed_actual_frame: Option<&'a Value>,
}

fn write_web_ui_test_incident(
    args: WebUiTestIncidentArgs<'_>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let wasm_sha = args.wasm_digest.map(|d| d.sha256.as_str()).unwrap_or("");
    let case_sha = args.case_digest.map(|d| d.sha256.as_str()).unwrap_or("");
    let step = args.failed_step.unwrap_or(0);
    let seed = format!("web-ui-test:{wasm_sha}:{case_sha}:{step}");
    let id = util::sha256_hex(seed.as_bytes());
    let id = id.chars().take(32).collect::<String>();

    let dir = args.incidents_dir.join("web-ui-test").join(id);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_INCIDENT_DIR_CREATE_FAILED",
            Severity::Warning,
            Stage::Run,
            format!("failed to create incident dir {}: {err}", dir.display()),
        ));
        return None;
    }

    let doc = json!({
      "v": 1,
      "kind": "x07.web_ui.incident",
      "error": args.error,
      "trace": args.trace,
      "failed": {
        "case": args.case_path.display().to_string(),
        "step": args.failed_step,
        "env": args.failed_env,
        "expected_frame": args.failed_expected_frame,
        "actual_frame": args.failed_actual_frame,
      },
      "inputs": {
        "wasm": args.wasm_digest,
        "case": args.case_digest,
      }
    });

    let incident_path = dir.join("incident.json");
    let bytes = match report::canon::canonical_pretty_json_bytes(&doc) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_INCIDENT_CANON_FAILED",
                Severity::Warning,
                Stage::Run,
                format!("failed to canonicalize incident JSON: {err:#}"),
            ));
            return None;
        }
    };
    if let Err(err) = std::fs::write(&incident_path, bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_INCIDENT_WRITE_FAILED",
            Severity::Warning,
            Stage::Run,
            format!(
                "failed to write incident {}: {err}",
                incident_path.display()
            ),
        ));
        return None;
    }

    match util::file_digest(&incident_path) {
        Ok(d) => {
            meta.outputs.push(d);
        }
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_TEST_INCIDENT_DIGEST_FAILED",
            Severity::Warning,
            Stage::Run,
            format!(
                "failed to digest incident {}: {err:#}",
                incident_path.display()
            ),
        )),
    }

    Some(dir)
}

#[allow(clippy::too_many_arguments)]
fn emit_test_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    mut diagnostics: Vec<Diagnostic>,
    cases: Vec<Value>,
    incident_dir: Option<String>,
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
      "schema_version": "x07.wasm.web_ui.test.report@0.1.0",
      "command": "x07-wasm.web-ui.test",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "cases": cases,
        "incident_dir": incident_dir,
      }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn run_component_trace_in_node(
    dist_dir: &Path,
    trace_path: &Path,
    max_steps: u32,
) -> Result<Vec<Value>> {
    let script = r#"
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

function stableJson(value) {
  if (value === null) return "null";
  const t = typeof value;
  if (t === "boolean") return value ? "true" : "false";
  if (t === "number") return JSON.stringify(value);
  if (t === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (t !== "object") return "null";
  const keys = Object.keys(value).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${stableJson(value[k])}`).join(",")}}`;
}

const distDir = process.argv[2];
const tracePath = process.argv[3];
const maxSteps = Number(process.argv[4] || "1000");

const modPath = path.join(distDir, "transpiled", "app.mjs");
const modUrl = pathToFileURL(modPath).href;
const m = await import(modUrl);
if (!m || typeof m.init !== "function" || typeof m.step !== "function") {
  throw new Error("transpiled module must export init and step");
}

const trace = JSON.parse(fs.readFileSync(tracePath, "utf8"));
const steps = Array.isArray(trace.steps) ? trace.steps : [];
const frames = [];

for (let i = 0; i < steps.length && i < maxSteps; i++) {
  const env = steps[i]?.env ?? null;
  if (i === 0) {
    const out = await m.init();
    const bytes = out instanceof Uint8Array ? out : new TextEncoder().encode(String(out ?? ""));
    frames.push(JSON.parse(new TextDecoder("utf-8").decode(bytes)));
    continue;
  }
  const inputBytes = new TextEncoder().encode(stableJson(env));
  const out = await m.step(inputBytes);
  const bytes = out instanceof Uint8Array ? out : new TextEncoder().encode(String(out ?? ""));
  frames.push(JSON.parse(new TextDecoder("utf-8").decode(bytes)));
}

process.stdout.write(JSON.stringify({ frames }) + "\n");
"#
    .to_string();

    let tmp = std::env::temp_dir().join(format!(
        "x07-wasm-web-ui-node-runner-{}.mjs",
        std::process::id()
    ));
    std::fs::write(&tmp, script.as_bytes()).context("write node runner")?;

    let args = vec![
        tmp.display().to_string(),
        dist_dir.display().to_string(),
        trace_path.display().to_string(),
        max_steps.to_string(),
    ];
    let out = crate::cmdutil::run_cmd_capture("node", &args)?;
    if !out.status.success() {
        anyhow::bail!(
            "node runner failed (code={}): {}",
            out.code,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let doc: Value = serde_json::from_slice(&out.stdout).context("parse node output")?;
    let frames = doc
        .get("frames")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(frames)
}

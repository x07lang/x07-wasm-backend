use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Request, Uri};
use serde_json::{json, Value};
use tokio::task::LocalSet;

use crate::app::bundle::LoadedAppBundle;
use crate::cli::{AppTestArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::http_component_host::{self, HttpComponentBudgets, HttpComponentHost};
use crate::report;
use crate::schema::SchemaStore;
use crate::stream_payload::{bytes_to_stream_payload, stream_payload_to_bytes};
use crate::util;
use crate::web_ui::replay;

pub fn cmd_app_test(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppTestArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let Some(bundle) =
        crate::app::bundle::load_app_bundle(&store, &args.dir, &mut meta, &mut diagnostics)?
    else {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    };

    let frontend_dir = args.dir.join(&bundle.doc.frontend.dir_rel);
    let wasm_path = frontend_dir.join("app.wasm");
    if !wasm_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_TEST_WASM_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("missing frontend wasm: {}", wasm_path.display()),
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
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    }

    let backend_component_path = args.dir.join(&bundle.doc.backend.artifact.path);
    if !backend_component_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_TEST_BACKEND_COMPONENT_MISSING",
            Severity::Error,
            Stage::Parse,
            format!(
                "missing backend component: {}",
                backend_component_path.display()
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
            &args,
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    }

    let (app_budgets, http_budgets) =
        load_app_test_budgets(&store, &bundle, &frontend_dir, &mut meta, &mut diagnostics);
    let Some(runtime_limits) =
        replay::load_wasm_runtime_limits(&store, &frontend_dir, &mut meta, &mut diagnostics)
    else {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    };

    let core = match replay::CoreWasmRunner::new(
        &wasm_path,
        &runtime_limits,
        app_budgets.arena_cap_bytes,
        app_budgets.max_output_bytes,
        &mut meta,
        &mut diagnostics,
    ) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_WASM_LOAD_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            None
        }
    };

    let host = match HttpComponentHost::from_component_file(&backend_component_path) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_BACKEND_HOST_INIT_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            None
        }
    };

    if core.is_none() || host.is_none() {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    }
    let mut core = core.unwrap();
    let host = host.unwrap();

    let trace_digest = match util::file_digest(&args.trace) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            Some(d)
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_TRACE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to digest trace {}: {err:#}", args.trace.display()),
            ));
            None
        }
    };
    let trace_bytes = match std::fs::read(&args.trace) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_TRACE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read trace {}: {err}", args.trace.display()),
            ));
            Vec::new()
        }
    };
    let mut trace_doc: Value = match serde_json::from_slice(&trace_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_TRACE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("trace is not JSON: {err}"),
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
            vec![case_result(&args.trace, false, 0, 1)],
            None,
            false,
        );
    }

    let steps = trace_doc
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let local = LocalSet::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .context("build tokio runtime")?;

    let mut updated = false;
    let mut mismatches: u64 = 0;
    let mut steps_run: u64 = 0;
    let mut prev_ui: Option<Value> = None;
    let mut current_state: Value = Value::Null;
    let mut observed_steps: Vec<Value> = Vec::new();
    let mut incident_dir: Option<PathBuf> = None;

    let max_steps = args.max_steps as usize;
    let mut host = host;
    let (final_ok, final_mismatches) = rt.block_on(local.run_until(async {
        for (idx, step) in steps.into_iter().enumerate().take(max_steps) {
            steps_run = steps_run.saturating_add(1);
            let ui_dispatch = step.get("ui_dispatch").cloned().unwrap_or(Value::Null);
            let expected_frame = step.get("ui_frame").cloned().unwrap_or(Value::Null);
            let expected_http = step.get("http").cloned().unwrap_or_else(|| json!([]));

            let event = ui_dispatch.get("event").cloned().unwrap_or(Value::Null);

            let input_bytes =
                replay::canonical_json_bytes_no_newline(&ui_dispatch).unwrap_or_default();
            if input_bytes.len() as u64 > app_budgets.max_dispatch_bytes {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_TEST_DISPATCH_TOO_LARGE",
                    Severity::Error,
                    Stage::Run,
                    format!("step {idx}: dispatch exceeds max_dispatch_bytes"),
                ));
                mismatches = mismatches.saturating_add(1);
                break;
            }

            let out_bytes = match core.call(&input_bytes) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_TEST_CALL_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("step {idx}: {err:#}"),
                    ));
                    mismatches = mismatches.saturating_add(1);
                    break;
                }
            };
            if out_bytes.len() as u64 > app_budgets.max_frame_bytes {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_TEST_FRAME_TOO_LARGE",
                    Severity::Error,
                    Stage::Run,
                    format!("step {idx}: frame exceeds max_frame_bytes"),
                ));
                mismatches = mismatches.saturating_add(1);
                break;
            }
            let mut frame: Value = match serde_json::from_slice(&out_bytes) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_TEST_FRAME_JSON_INVALID",
                        Severity::Error,
                        Stage::Run,
                        format!("step {idx}: output is not JSON: {err}"),
                    ));
                    mismatches = mismatches.saturating_add(1);
                    break;
                }
            };

            diagnostics.extend(
                store.validate("https://x07.io/spec/x07-web_ui.frame.schema.json", &frame)?,
            );
            if diagnostics.iter().any(|d| d.severity == Severity::Error) {
                mismatches = mismatches.saturating_add(1);
                break;
            }

            if let Err(err) = commit_frame(idx, &frame, &mut prev_ui, &mut current_state) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_TEST_FRAME_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!("step {idx}: {err:#}"),
                ));
                mismatches = mismatches.saturating_add(1);
                break;
            }

            let (frame2, exchanges) = match run_http_effects_loop(
                &store,
                &mut host,
                &http_budgets,
                &app_budgets,
                idx,
                args.dir.as_path(),
                &bundle,
                &args,
                &mut core,
                &mut prev_ui,
                &mut current_state,
                &event,
                frame,
            )
            .await
            {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_TEST_HTTP_EFFECT_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("step {idx}: {err:#}"),
                    ));
                    mismatches = mismatches.saturating_add(1);
                    break;
                }
            };

            frame = frame2;

            observed_steps.push(json!({
              "i": idx,
              "ui_dispatch": ui_dispatch.clone(),
              "ui_frame": frame.clone(),
              "http": exchanges.clone(),
              "timing": { "ui_ms": 0, "http_ms": 0, "total_ms": 0 }
            }));

            let actual_http = Value::Array(exchanges);
            let mut step_mismatch = false;

            if !frames_equal(&expected_frame, &frame)? {
                step_mismatch = true;
            }

            if !http_equal(&expected_http, &actual_http)? {
                step_mismatch = true;
            }

            if step_mismatch {
                if args.update_golden {
                    if let Some(obj) = trace_doc
                        .get_mut("steps")
                        .and_then(Value::as_array_mut)
                        .and_then(|arr| arr.get_mut(idx))
                        .and_then(Value::as_object_mut)
                    {
                        obj.insert("ui_frame".to_string(), frame.clone());
                        obj.insert("http".to_string(), actual_http.clone());
                        obj.insert(
                            "timing".to_string(),
                            json!({ "ui_ms": 0, "http_ms": 0, "total_ms": 0 }),
                        );
                        updated = true;
                    }
                } else {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_TEST_MISMATCH",
                        Severity::Error,
                        Stage::Run,
                        format!("step {idx}: mismatch"),
                    ));
                    mismatches = mismatches.saturating_add(1);
                    incident_dir = write_app_test_incident(
                        args.dir.as_path(),
                        trace_digest.as_ref(),
                        trace_doc.get("meta").cloned(),
                        &observed_steps,
                        idx,
                        &expected_frame,
                        &frame,
                        &expected_http,
                        &actual_http,
                        &mut meta,
                        &mut diagnostics,
                    );
                    break;
                }
            }
        }

        Ok::<_, anyhow::Error>((mismatches == 0, mismatches))
    }))?;

    mismatches = final_mismatches;

    if args.update_golden && updated {
        let bytes = report::canon::canonical_pretty_json_bytes(&trace_doc)?;
        std::fs::write(&args.trace, &bytes)
            .with_context(|| format!("write: {}", args.trace.display()))?;
        if let Ok(d) = util::file_digest(&args.trace) {
            meta.outputs.push(d);
        }
    }

    if incident_dir.is_some() {
        meta.nondeterminism.uses_os_time = true;
    }

    let case_ok = final_ok || (args.update_golden && updated);
    let case_result = case_result(&args.trace, case_ok, steps_run, mismatches);
    let cases = vec![case_result];

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
        cases,
        incident_dir.as_deref(),
        updated,
    )
}

struct AppTestBudgets {
    max_dispatch_bytes: u64,
    max_frame_bytes: u64,
    arena_cap_bytes: u32,
    max_output_bytes: u32,
    api_prefix: String,
}

fn load_app_test_budgets(
    store: &SchemaStore,
    bundle: &LoadedAppBundle,
    frontend_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> (AppTestBudgets, HttpComponentBudgets) {
    let index_path = PathBuf::from("arch/app/index.x07app.json");
    let loaded = crate::app::load::load_app_profile(
        store,
        &index_path,
        Some(bundle.doc.profile_id.as_str()),
        None,
    );
    let loaded = match loaded {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_TEST_PROFILE_LOAD_FAILED",
                Severity::Warning,
                Stage::Parse,
                format!(
                    "failed to load app profile {:?}: {err:#}",
                    bundle.doc.profile_id
                ),
            ));
            return (
                AppTestBudgets {
                    max_dispatch_bytes: 65536,
                    max_frame_bytes: 16 * 1024 * 1024,
                    arena_cap_bytes: 32 * 1024 * 1024,
                    max_output_bytes: 2 * 1024 * 1024,
                    api_prefix: "/api".to_string(),
                },
                HttpComponentBudgets {
                    max_request_bytes: 1024 * 1024,
                    max_response_bytes: 1024 * 1024,
                    max_wall_ms: 2_000,
                },
            );
        }
    };

    meta.inputs.push(loaded.digest.clone());
    if let Some(d) = loaded.index_digest.as_ref() {
        meta.inputs.push(d.clone());
    }

    let (arena_cap_bytes, max_output_bytes) =
        replay::load_web_ui_budgets(store, frontend_dir, meta, diagnostics);

    let max_http = usize::try_from(loaded.doc.budgets.max_http_body_bytes).unwrap_or(1024 * 1024);
    let max_wall_ms = loaded.doc.budgets.max_request_wall_ms.max(1);
    let api_prefix = loaded.doc.routing.api_prefix.clone();

    (
        AppTestBudgets {
            max_dispatch_bytes: loaded.doc.budgets.max_dispatch_bytes,
            max_frame_bytes: loaded.doc.budgets.max_frame_bytes,
            arena_cap_bytes,
            max_output_bytes,
            api_prefix,
        },
        HttpComponentBudgets {
            max_request_bytes: max_http,
            max_response_bytes: max_http,
            max_wall_ms,
        },
    )
}

fn frames_equal(expected: &Value, actual: &Value) -> Result<bool> {
    let a = report::canon::canonical_json_bytes(expected)?;
    let b = report::canon::canonical_json_bytes(actual)?;
    Ok(a == b)
}

fn http_equal(expected: &Value, actual: &Value) -> Result<bool> {
    let ne = normalize_http_exchanges(expected)?;
    let na = normalize_http_exchanges(actual)?;
    Ok(report::canon::canonical_json_bytes(&ne)? == report::canon::canonical_json_bytes(&na)?)
}

fn normalize_http_exchanges(exchanges: &Value) -> Result<Value> {
    let mut out = Vec::new();
    let arr = exchanges.as_array().cloned().unwrap_or_default();
    for ex in arr {
        let req = ex.get("request").cloned().unwrap_or(Value::Null);
        let resp = ex.get("response").cloned().unwrap_or(Value::Null);
        out.push(json!({
          "request": normalize_request_envelope(&req)?,
          "response": normalize_response_envelope(&resp)?,
        }));
    }
    Ok(Value::Array(out))
}

fn normalize_request_envelope(req: &Value) -> Result<Value> {
    let id = req
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let path = req
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_string();
    let query = req
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let headers = normalize_headers(req.get("headers"));
    let body = normalize_stream_payload(req.get("body"))?;

    let mut obj = serde_json::Map::new();
    obj.insert(
        "schema_version".to_string(),
        json!("x07.http.request.envelope@0.1.0"),
    );
    obj.insert("id".to_string(), Value::String(id));
    obj.insert("method".to_string(), Value::String(method));
    obj.insert("path".to_string(), Value::String(path));
    if !query.is_empty() {
        obj.insert("query".to_string(), Value::String(query));
    }
    obj.insert("headers".to_string(), headers);
    obj.insert("body".to_string(), body);
    Ok(Value::Object(obj))
}

fn normalize_response_envelope(resp: &Value) -> Result<Value> {
    let request_id = resp
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = resp.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
    let headers = normalize_headers(resp.get("headers"));
    let body = normalize_stream_payload(resp.get("body"))?;

    Ok(json!({
      "schema_version": "x07.http.response.envelope@0.1.0",
      "request_id": request_id,
      "status": status,
      "headers": headers,
      "body": body,
    }))
}

fn normalize_headers(v: Option<&Value>) -> Value {
    let mut out = Vec::new();
    if let Some(arr) = v.and_then(Value::as_array) {
        for h in arr {
            let k = h.get("k").and_then(Value::as_str).unwrap_or("").to_string();
            let v = h.get("v").and_then(Value::as_str).unwrap_or("").to_string();
            if !k.is_empty() {
                out.push((k, v));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    Value::Array(
        out.into_iter()
            .map(|(k, v)| json!({ "k": k, "v": v }))
            .collect(),
    )
}

fn normalize_stream_payload(v: Option<&Value>) -> Result<Value> {
    let bytes = stream_payload_to_bytes(v.unwrap_or(&Value::Null))?;
    Ok(bytes_to_stream_payload(&bytes))
}

#[allow(clippy::too_many_arguments)]
async fn run_http_effects_loop(
    store: &SchemaStore,
    host: &mut HttpComponentHost,
    http_budgets: &HttpComponentBudgets,
    app_budgets: &AppTestBudgets,
    step_index: usize,
    _bundle_dir: &Path,
    _bundle: &LoadedAppBundle,
    _args: &AppTestArgs,
    core: &mut replay::CoreWasmRunner,
    prev_ui: &mut Option<Value>,
    current_state: &mut Value,
    event: &Value,
    first_frame: Value,
) -> Result<(Value, Vec<Value>)> {
    let mut frame = first_frame;
    let mut exchanges: Vec<Value> = Vec::new();

    let max_loops = 16;
    for _ in 0..max_loops {
        let reqs = find_http_request_effects(&frame)?;
        if reqs.is_empty() {
            break;
        }
        if reqs.len() != 1 {
            anyhow::bail!(
                "expected exactly one http request effect (got {})",
                reqs.len()
            );
        }

        let req0 = reqs.into_iter().next().unwrap();
        let req_env = build_exec_request_envelope(app_budgets.api_prefix.as_str(), &req0)?;

        let resp_env = execute_http_request(host, http_budgets, &req_env).await?;
        exchanges.push(json!({ "request": req_env.clone(), "response": resp_env.clone() }));

        let injected_state = inject_http_response_state(current_state.clone(), resp_env);
        let env = json!({
          "v": 1,
          "kind": "x07.web_ui.dispatch",
          "state": injected_state,
          "event": event,
        });
        let input_bytes = replay::canonical_json_bytes_no_newline(&env)?;
        if input_bytes.len() as u64 > app_budgets.max_dispatch_bytes {
            anyhow::bail!("dispatch exceeds max_dispatch_bytes");
        }
        let out_bytes = core.call(&input_bytes)?;
        if out_bytes.len() as u64 > app_budgets.max_frame_bytes {
            anyhow::bail!("frame exceeds max_frame_bytes");
        }
        let next_frame: Value = serde_json::from_slice(&out_bytes).context("frame JSON")?;
        let diags = store.validate(
            "https://x07.io/spec/x07-web_ui.frame.schema.json",
            &next_frame,
        )?;
        if diags.iter().any(|d| d.severity == Severity::Error) {
            anyhow::bail!("frame schema invalid: {diags:?}");
        }
        commit_frame(step_index, &next_frame, prev_ui, current_state)?;
        frame = next_frame;
    }

    Ok((frame, exchanges))
}

fn find_http_request_effects(frame: &Value) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let effects = frame
        .get("effects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for eff in effects {
        if let Some(req) = parse_http_request_effect(&eff)? {
            out.push(req);
        }
    }
    Ok(out)
}

fn parse_http_request_effect(effect: &Value) -> Result<Option<Value>> {
    let v = effect.get("v").and_then(Value::as_u64).unwrap_or(0);
    let kind = effect.get("kind").and_then(Value::as_str).unwrap_or("");
    if v != 1 || kind != "x07.web_ui.effect.http.request" {
        return Ok(None);
    }
    let req = effect
        .get("request")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("http request effect missing request"))?;
    let schema_version = req
        .get("schema_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if schema_version != "x07.http.request.envelope@0.1.0" {
        anyhow::bail!("unsupported request envelope schema_version: {schema_version:?}");
    }
    Ok(Some(req))
}

fn build_exec_request_envelope(api_prefix: &str, req0: &Value) -> Result<Value> {
    let id = req0
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        anyhow::bail!("http request effect missing id");
    }
    let path0 = req0
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if path0.is_empty() {
        anyhow::bail!("http request effect missing path");
    }
    let exec_path = join_url_path(api_prefix, &path0);

    let mut env = req0.clone();
    if let Some(obj) = env.as_object_mut() {
        obj.insert("path".to_string(), Value::String(exec_path));
        let query = obj.get("query").and_then(Value::as_str).unwrap_or("");
        if query.is_empty() {
            obj.remove("query");
        }
    }
    normalize_request_envelope(&env)
}

fn join_url_path(prefix: &str, path: &str) -> String {
    let pfx = prefix;
    let p = path;
    if pfx.is_empty() || pfx == "/" {
        return p.to_string();
    }
    if p.is_empty() {
        return pfx.to_string();
    }
    if p == pfx || p.starts_with(&format!("{pfx}/")) {
        return p.to_string();
    }
    if pfx.ends_with('/') && p.starts_with('/') {
        return format!("{pfx}{}", &p[1..]);
    }
    if !pfx.ends_with('/') && !p.starts_with('/') {
        return format!("{pfx}/{p}");
    }
    format!("{pfx}{p}")
}

fn inject_http_response_state(state: Value, resp_env: Value) -> Value {
    let mut obj = match state {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    obj.insert("__x07_http".to_string(), json!({ "response": resp_env }));
    Value::Object(obj)
}

async fn execute_http_request(
    host: &mut HttpComponentHost,
    budgets: &HttpComponentBudgets,
    req_env: &Value,
) -> Result<Value> {
    let method = req_env
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let path = req_env.get("path").and_then(Value::as_str).unwrap_or("/");
    let query = req_env.get("query").and_then(Value::as_str).unwrap_or("");
    let uri_str = if query.is_empty() {
        format!("http://localhost{path}")
    } else {
        format!("http://localhost{path}?{query}")
    };
    let uri: Uri = uri_str.parse().context("parse uri")?;

    let body_bytes = stream_payload_to_bytes(req_env.get("body").unwrap_or(&Value::Null))?;
    let mut builder = Request::builder().method(method.as_str()).uri(uri);
    builder = builder.header(hyper::header::HOST, "localhost");

    if let Some(arr) = req_env.get("headers").and_then(Value::as_array) {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for h in arr {
            let k = h.get("k").and_then(Value::as_str).unwrap_or("").to_string();
            let v = h.get("v").and_then(Value::as_str).unwrap_or("").to_string();
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (k, v) in pairs {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(&v),
            ) {
                builder = builder.header(name, val);
            }
        }
    }

    let req = builder
        .body(http_component_host::full_body(body_bytes))
        .context("build request")?;

    let resp = host.handle_request(req, budgets).await?;

    let mut headers = Vec::new();
    for (k, v) in resp.headers {
        if !k.is_empty() {
            headers.push(json!({ "k": k, "v": v }));
        }
    }
    headers.sort_by(|a, b| {
        let ak = a.get("k").and_then(Value::as_str).unwrap_or("");
        let bk = b.get("k").and_then(Value::as_str).unwrap_or("");
        let av = a.get("v").and_then(Value::as_str).unwrap_or("");
        let bv = b.get("v").and_then(Value::as_str).unwrap_or("");
        ak.cmp(bk).then(av.cmp(bv))
    });

    Ok(json!({
      "schema_version": "x07.http.response.envelope@0.1.0",
      "request_id": req_env.get("id").and_then(Value::as_str).unwrap_or(""),
      "status": resp.status,
      "headers": headers,
      "body": bytes_to_stream_payload(&resp.body),
    }))
}

fn commit_frame(
    step_idx: usize,
    frame: &Value,
    prev_ui: &mut Option<Value>,
    current_state: &mut Value,
) -> Result<()> {
    let next_state = frame.get("state").cloned().unwrap_or(Value::Null);
    let next_ui = frame.get("ui").cloned().unwrap_or(Value::Null);
    let patches = frame.get("patches").cloned().unwrap_or_else(|| json!([]));

    if let Some(prev) = prev_ui.as_ref() {
        let patched = replay::apply_json_patch(prev.clone(), &patches)?;
        let a = report::canon::canonical_json_bytes(&patched)?;
        let b = report::canon::canonical_json_bytes(&next_ui)?;
        if a != b {
            anyhow::bail!("step {step_idx}: patches do not match ui tree");
        }
    }

    *prev_ui = Some(next_ui);
    *current_state = next_state;
    Ok(())
}

fn case_result(path: &Path, ok: bool, steps: u64, mismatches: u64) -> Value {
    json!({
      "case": path.display().to_string(),
      "ok": ok,
      "steps": steps,
      "mismatches": mismatches,
    })
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
    args: &AppTestArgs,
    cases: Vec<Value>,
    incident_dir: Option<&Path>,
    updated_golden: bool,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let passed = cases
        .iter()
        .filter(|c| c.get("ok") == Some(&Value::Bool(true)))
        .count() as u64;
    let failed = cases.len() as u64 - passed;

    let stdout_json = json!({
      "dir": args.dir.display().to_string(),
      "passed": passed,
      "failed": failed,
      "updated_golden": updated_golden,
      "cases": cases
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.app.test.report@0.1.0",
      "command": "x07-wasm.app.test",
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

    let incident_report_bytes = incident_dir.map(|_| {
        report::canon::canonical_pretty_json_bytes(&report_doc).unwrap_or_else(|_| b"{}\n".to_vec())
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    if let (Some(dir), Some(bytes)) = (incident_dir, incident_report_bytes) {
        let _ = std::fs::write(dir.join("app.test.report.json"), bytes);
    }
    Ok(exit_code)
}

#[allow(clippy::too_many_arguments)]
fn write_app_test_incident(
    bundle_dir: &Path,
    trace_digest: Option<&report::meta::FileDigest>,
    trace_meta: Option<Value>,
    observed_steps: &[Value],
    failed_step: usize,
    expected_frame: &Value,
    actual_frame: &Value,
    expected_http: &Value,
    actual_http: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let trace_sha = trace_digest.map(|d| d.sha256.as_str()).unwrap_or("");
    let seed = format!("app-test:{trace_sha}:{failed_step}");
    let id = util::sha256_hex(seed.as_bytes());
    let id = id.chars().take(32).collect::<String>();

    let date = crate::wasm::incident::utc_date_yyyy_mm_dd();
    let dir = PathBuf::from(".x07-wasm")
        .join("incidents")
        .join("app")
        .join(date)
        .join(id);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_TEST_INCIDENT_DIR_CREATE_FAILED",
            Severity::Warning,
            Stage::Run,
            format!("failed to create incident dir {}: {err}", dir.display()),
        ));
        return None;
    }

    let trace_doc = json!({
      "schema_version": "x07.app.trace@0.1.0",
      "meta": trace_meta.unwrap_or_else(|| json!({ "tool": { "name": "x07-wasm", "version": env!("CARGO_PKG_VERSION") } })),
      "steps": observed_steps,
    });

    let _ = std::fs::write(
        dir.join("app.bundle.json"),
        std::fs::read(bundle_dir.join("app.bundle.json")).unwrap_or_else(|_| b"{}\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("trace.json"),
        report::canon::canonical_pretty_json_bytes(&trace_doc).unwrap_or_else(|_| b"{}\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("frontend.frame.expected.json"),
        report::canon::canonical_pretty_json_bytes(expected_frame)
            .unwrap_or_else(|_| b"{}\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("frontend.frame.observed.json"),
        report::canon::canonical_pretty_json_bytes(actual_frame)
            .unwrap_or_else(|_| b"{}\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("backend.http.expected.json"),
        report::canon::canonical_pretty_json_bytes(expected_http)
            .unwrap_or_else(|_| b"[]\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("backend.http.observed.json"),
        report::canon::canonical_pretty_json_bytes(actual_http)
            .unwrap_or_else(|_| b"[]\n".to_vec()),
    );

    let (req_env, resp_env) = actual_http
        .as_array()
        .and_then(|arr| arr.first())
        .map(|ex| {
            let req = ex.get("request").cloned();
            let resp = ex.get("response").cloned();
            (req, resp)
        })
        .unwrap_or((None, None));
    let req_env = req_env.unwrap_or_else(|| json!({}));
    let resp_env = resp_env.unwrap_or_else(|| json!({}));
    let _ = std::fs::write(
        dir.join("backend.request.envelope.json"),
        report::canon::canonical_pretty_json_bytes(&req_env).unwrap_or_else(|_| b"{}\n".to_vec()),
    );
    let _ = std::fs::write(
        dir.join("backend.response.envelope.json"),
        report::canon::canonical_pretty_json_bytes(&resp_env).unwrap_or_else(|_| b"{}\n".to_vec()),
    );

    let _ = std::fs::write(dir.join("stderr.txt"), b"");

    if let Ok(d) = util::file_digest(&dir.join("trace.json")) {
        meta.outputs.push(d);
    }

    Some(dir)
}

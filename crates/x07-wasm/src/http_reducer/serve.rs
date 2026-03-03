use std::ffi::OsString;
use std::io::Read as _;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::task::LocalSet;

use crate::cli::{HttpServeArgs, HttpServeMode, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::http_reducer::replay::{
    parse_http_fetch_allow_hosts_from_env, run_http_reducer_loop, HttpEffectLoopBudgets,
    HttpEffectState,
};
use crate::ops::load_ops_profile_with_refs;
use crate::report;
use crate::schema::SchemaStore;
use crate::stream_payload::stream_payload_to_bytes;
use crate::util;
use crate::web_ui::replay::CoreWasmRunner;

pub fn cmd_http_serve(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: HttpServeArgs,
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
                "X07WASM_HTTP_SERVE_COMPONENT_READ_FAILED",
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
                &args,
                report::meta::FileDigest {
                    path: args.component.display().to_string(),
                    sha256: "0".repeat(64),
                    bytes_len: 0,
                },
                crate::arch::WasmRuntimeLimits {
                    instance_allocator: crate::arch::WasmInstanceAllocator::OnDemand,
                    max_fuel: args.max_fuel,
                    max_memory_bytes: None,
                    max_table_elements: None,
                    max_wasm_stack_bytes: None,
                    cache_config: None,
                    notes: None,
                },
                0,
                Vec::new(),
                None,
                None,
            );
        }
    };

    let loaded_wasm_profile = match crate::arch::load_profile(
        &store,
        &std::path::PathBuf::from("arch/wasm/index.x07wasm.json"),
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
                &args,
                component_digest,
                crate::arch::WasmRuntimeLimits {
                    instance_allocator: crate::arch::WasmInstanceAllocator::OnDemand,
                    max_fuel: args.max_fuel,
                    max_memory_bytes: None,
                    max_table_elements: None,
                    max_wasm_stack_bytes: None,
                    cache_config: None,
                    notes: None,
                },
                0,
                Vec::new(),
                None,
                None,
            );
        }
    };
    meta.inputs.push(loaded_wasm_profile.digest.clone());
    if let Some(d) = loaded_wasm_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let mut runtime_limits = loaded_wasm_profile.doc.runtime.clone();
    if let Some(fuel) = args.max_fuel {
        runtime_limits.max_fuel = Some(fuel);
    }
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
                "X07WASM_HTTP_SERVE_COMPONENT_LOAD_FAILED",
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
                component_digest,
                runtime_limits,
                0,
                Vec::new(),
                None,
                None,
            );
        }
    };

    let budgets = HttpEffectLoopBudgets {
        max_effect_steps: args.max_effect_steps,
        max_effect_results_bytes: args.max_effect_results_bytes,
    };

    let mut caps: Option<crate::caps::doc::CapabilitiesDoc> = None;
    if let Some(ops_path) = args.ops.as_ref() {
        let loaded_ops = load_ops_profile_with_refs(&store, ops_path, &mut meta, &mut diagnostics)?;
        let Some(loaded_ops) = loaded_ops else {
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args,
                component_digest,
                runtime_limits,
                0,
                Vec::new(),
                None,
                None,
            );
        };
        match serde_json::from_value::<crate::caps::doc::CapabilitiesDoc>(
            loaded_ops.capabilities.doc_json.clone(),
        ) {
            Ok(v) => caps = Some(v),
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_CAPS_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse capabilities doc: {err}"),
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
                    component_digest,
                    runtime_limits,
                    0,
                    Vec::new(),
                    None,
                    None,
                );
            }
        }

        if diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && d.stage == Stage::Parse)
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
                component_digest,
                runtime_limits,
                0,
                Vec::new(),
                None,
                None,
            );
        }
    }

    let fetch_allow_hosts = if caps.is_some() {
        Vec::new()
    } else {
        parse_http_fetch_allow_hosts_from_env()
    };
    let mut effect_state = HttpEffectState::new(fetch_allow_hosts, caps);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();

    let (responses, requests) = rt.block_on(local.run_until(async {
        match args.mode {
            HttpServeMode::Canary => {
                let req_env = default_canary_request_envelope();
                let (resp_env, _trace, nondet) = run_http_reducer_loop(
                    &store,
                    &mut core,
                    &req_env,
                    budgets,
                    &mut effect_state,
                    &mut diagnostics,
                )
                .await?;

                effect_state.nondeterminism = nondet;
                let summary = resp_env
                    .as_ref()
                    .map(|r| response_summary_from_envelope(r, &mut diagnostics))
                    .unwrap_or_else(|| json!({"status": 500, "headers_count": 0, "body_bytes": 0}));
                Ok::<_, anyhow::Error>((vec![summary], 1u64))
            }
            HttpServeMode::Listen => {
                let reqs = read_request_envelopes_from_stdin(&store, &mut meta, &mut diagnostics)?;
                let reqs_len = reqs.len();
                let mut summaries: Vec<Value> = Vec::new();
                for req_env in reqs {
                    let (resp_env, _trace, nondet) = run_http_reducer_loop(
                        &store,
                        &mut core,
                        &req_env,
                        budgets,
                        &mut effect_state,
                        &mut diagnostics,
                    )
                    .await?;
                    effect_state.nondeterminism.uses_network |= nondet.uses_network;
                    effect_state.nondeterminism.uses_os_time |= nondet.uses_os_time;
                    summaries.push(
                        resp_env
                            .as_ref()
                            .map(|r| response_summary_from_envelope(r, &mut diagnostics))
                            .unwrap_or_else(
                                || json!({"status": 500, "headers_count": 0, "body_bytes": 0}),
                            ),
                    );
                }
                Ok::<_, anyhow::Error>((summaries, reqs_len as u64))
            }
        }
    }))?;

    meta.nondeterminism.uses_network = effect_state.nondeterminism.uses_network;
    meta.nondeterminism.uses_os_time = effect_state.nondeterminism.uses_os_time;

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args,
        component_digest,
        runtime_limits,
        requests,
        responses,
        None,
        None,
    )
}

fn default_canary_request_envelope() -> Value {
    json!({
      "schema_version": "x07.http.request.envelope@0.1.0",
      "id": "req0",
      "method": "GET",
      "path": "/",
      "headers": [],
      "body": { "bytes_len": 0 },
    })
}

fn read_request_envelopes_from_stdin(
    store: &SchemaStore,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<Value>> {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf)?;
    if buf.is_empty() {
        return Ok(Vec::new());
    }

    let stdin_digest = report::meta::FileDigest {
        path: "<stdin>".to_string(),
        sha256: util::sha256_hex(&buf),
        bytes_len: buf.len() as u64,
    };
    meta.inputs.push(stdin_digest);

    let mut out = Vec::new();
    let s = String::from_utf8_lossy(&buf);
    for (i, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let doc: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_HTTP_SERVE_STDIN_JSON_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("stdin line {} is not JSON: {err}", i + 1),
                ));
                continue;
            }
        };

        let diags = store.validate(
            "https://x07.io/spec/x07-http.request.envelope.schema.json",
            &doc,
        )?;
        if diags.iter().any(|d| d.severity == Severity::Error) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_SERVE_REQUEST_ENVELOPE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("stdin line {} request envelope schema invalid", i + 1),
            ));
            diagnostics.extend(diags);
            continue;
        }
        out.push(doc);
    }
    Ok(out)
}

fn response_summary_from_envelope(resp_env: &Value, diagnostics: &mut Vec<Diagnostic>) -> Value {
    let status = resp_env
        .get("status")
        .and_then(Value::as_u64)
        .unwrap_or(500) as u16;
    let headers_count = resp_env
        .get("headers")
        .and_then(Value::as_array)
        .map(|a| a.len() as u64)
        .unwrap_or(0);
    let body = resp_env.get("body").cloned().unwrap_or(Value::Null);
    let body_bytes = match stream_payload_to_bytes(&body) {
        Ok(v) => v.len() as u64,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_SERVE_RESPONSE_BODY_INVALID",
                Severity::Warning,
                Stage::Run,
                format!("invalid response body: {err:#}"),
            ));
            0
        }
    };
    json!({
      "status": status,
      "headers_count": headers_count,
      "body_bytes": body_bytes,
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
    args: &HttpServeArgs,
    component: report::meta::FileDigest,
    runtime_limits: crate::arch::WasmRuntimeLimits,
    requests: u64,
    responses: Vec<Value>,
    trace_path: Option<String>,
    incident_dir: Option<String>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.http.serve.report@0.1.0",
      "command": "x07-wasm.http.serve",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": match args.mode { HttpServeMode::Canary => "canary", HttpServeMode::Listen => "listen" },
        "component": component,
        "runtime_limits": runtime_limits,
        "requests": requests,
        "responses": responses,
        "trace_path": trace_path,
        "incident_dir": incident_dir,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

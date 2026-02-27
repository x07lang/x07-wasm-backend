use std::collections::HashMap;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use serde_json::{json, Value};

use crate::caps::doc::{CapabilitiesDoc, CapabilityMode};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::schema::SchemaStore;
use crate::stream_payload::{bytes_to_stream_payload, stream_payload_to_bytes};
use crate::wasmtime_limits::{classify_budget_exceeded, BudgetExceededKind};
use crate::web_ui::replay::CoreWasmRunner;

#[derive(Debug, Clone, Copy)]
pub struct HttpEffectLoopBudgets {
    pub max_effect_steps: u32,
    pub max_effect_results_bytes: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HttpEffectLoopNondeterminism {
    pub uses_network: bool,
    pub uses_os_time: bool,
}

pub struct HttpEffectState {
    pub kv: HashMap<String, Vec<u8>>,
    pub nondeterminism: HttpEffectLoopNondeterminism,
    client: Client<HttpConnector, Full<Bytes>>,
    fetch_allow_hosts: Vec<String>,
    caps: Option<CapabilitiesDoc>,
}

impl HttpEffectState {
    pub fn new(fetch_allow_hosts: Vec<String>, caps: Option<CapabilitiesDoc>) -> Self {
        let client =
            Client::builder(hyper_util::rt::TokioExecutor::new()).build(HttpConnector::new());
        Self {
            kv: HashMap::new(),
            nondeterminism: HttpEffectLoopNondeterminism::default(),
            client,
            fetch_allow_hosts,
            caps,
        }
    }

    fn is_fetch_allowed(&self, uri: &Uri) -> bool {
        if let Some(caps) = self.caps.as_ref() {
            let Some(host) = uri.host() else {
                return false;
            };
            let scheme = uri.scheme_str().unwrap_or("");
            let port = uri.port_u16().unwrap_or(80);
            return caps.network_allows(scheme, host, port);
        }
        let Some(host) = uri.host() else {
            return false;
        };
        self.fetch_allow_hosts.iter().any(|h| h == host)
    }
}

pub fn parse_http_fetch_allow_hosts_from_env() -> Vec<String> {
    let s = std::env::var("X07_WASM_HTTP_FETCH_ALLOW_HOSTS").unwrap_or_default();
    s.split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

pub async fn run_http_reducer_loop(
    store: &SchemaStore,
    core: &mut CoreWasmRunner,
    request_envelope: &Value,
    budgets: HttpEffectLoopBudgets,
    effect_state: &mut HttpEffectState,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Option<Value>, Value, HttpEffectLoopNondeterminism)> {
    let mut steps: Vec<Value> = Vec::new();

    let mut state: Value = Value::Null;
    let mut effect_results: Vec<Value> = Vec::new();

    for _ in 0..budgets.max_effect_steps {
        let dispatch = json!({
          "v": 1,
          "kind": "x07.http.dispatch",
          "state": state,
          "request": request_envelope,
          "effect_results": effect_results,
        });

        let dispatch_schema_diags = store.validate(
            "https://x07.io/spec/x07-http.dispatch.schema.json",
            &dispatch,
        )?;
        if dispatch_schema_diags
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_DISPATCH_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "dispatch schema invalid".to_string(),
            ));
            diagnostics.extend(dispatch_schema_diags);
            return Ok((None, trace_doc(steps), effect_state.nondeterminism));
        }

        let input_bytes = crate::report::canon::canonical_json_bytes(&dispatch)?;
        let output_bytes = match core.call(&input_bytes) {
            Ok(v) => v,
            Err(err) => {
                if let Some(kind) = classify_budget_exceeded(&err) {
                    let (code, msg) = match kind {
                        BudgetExceededKind::CpuFuel => (
                            "X07WASM_BUDGET_EXCEEDED_CPU_FUEL",
                            "execution exceeded Wasmtime fuel budget",
                        ),
                        BudgetExceededKind::WasmStack => (
                            "X07WASM_BUDGET_EXCEEDED_WASM_STACK",
                            "execution exceeded Wasmtime wasm stack budget",
                        ),
                        BudgetExceededKind::Memory => (
                            "X07WASM_BUDGET_EXCEEDED_MEMORY",
                            "execution exceeded Wasmtime memory budget",
                        ),
                        BudgetExceededKind::Table => (
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
                        "X07WASM_HTTP_REDUCER_CALL_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }
                return Ok((None, trace_doc(steps), effect_state.nondeterminism));
            }
        };

        let frame: Value = match serde_json::from_slice(&output_bytes) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_HTTP_FRAME_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("frame is not JSON: {err}"),
                ));
                return Ok((None, trace_doc(steps), effect_state.nondeterminism));
            }
        };

        let frame_schema_diags =
            store.validate("https://x07.io/spec/x07-http.frame.schema.json", &frame)?;
        if frame_schema_diags
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_FRAME_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                "frame schema invalid".to_string(),
            ));
            diagnostics.extend(frame_schema_diags);
            return Ok((None, trace_doc(steps), effect_state.nondeterminism));
        }

        steps.push(json!({ "env": dispatch, "frame": frame }));

        let frame = steps
            .last()
            .and_then(|s| s.get("frame"))
            .cloned()
            .unwrap_or(Value::Null);

        if let Some(resp) = frame.get("response") {
            if !resp.is_null() {
                return Ok((
                    Some(resp.clone()),
                    trace_doc(steps),
                    effect_state.nondeterminism,
                ));
            }
        }

        state = frame.get("state").cloned().unwrap_or(Value::Null);

        let effects = frame
            .get("effects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut next_results: Vec<Value> = Vec::new();
        let mut result_payload_bytes: u64 = 0;
        for eff in effects {
            let res = execute_effect(effect_state, &eff, diagnostics).await;
            let payload_bytes = effect_result_payload_bytes(&res);
            result_payload_bytes = result_payload_bytes.saturating_add(payload_bytes);
            if result_payload_bytes > budgets.max_effect_results_bytes {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_BUDGET_EXCEEDED_HTTP_EFFECT_RESULT_BYTES",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "effect results exceeded budget: bytes={} max_effect_results_bytes={}",
                        result_payload_bytes, budgets.max_effect_results_bytes
                    ),
                ));
                return Ok((None, trace_doc(steps), effect_state.nondeterminism));
            }
            next_results.push(res);
        }
        effect_results = next_results;
    }

    diagnostics.push(Diagnostic::new(
        "X07WASM_BUDGET_EXCEEDED_HTTP_EFFECTS_LOOPS",
        Severity::Error,
        Stage::Run,
        format!(
            "effect loop exceeded budget: max_effect_steps={}",
            budgets.max_effect_steps
        ),
    ));
    Ok((None, trace_doc(steps), effect_state.nondeterminism))
}

fn trace_doc(steps: Vec<Value>) -> Value {
    json!({
      "v": 1,
      "kind": "x07.http.trace",
      "steps": steps,
      "meta": {},
    })
}

fn effect_result_payload_bytes(result: &Value) -> u64 {
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return 0;
    }

    let typ = result.get("type").and_then(Value::as_str).unwrap_or("");
    match typ {
        "kv.get" => result
            .get("value")
            .and_then(|v| v.get("bytes_len"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        "http.fetch" => result
            .get("response")
            .and_then(|v| v.get("body"))
            .and_then(|v| v.get("bytes_len"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        _ => 0,
    }
}

async fn execute_effect(
    effect_state: &mut HttpEffectState,
    eff: &Value,
    diagnostics: &mut Vec<Diagnostic>,
) -> Value {
    let id = eff.get("id").and_then(Value::as_str).unwrap_or("eff0");
    let typ = eff.get("type").and_then(Value::as_str).unwrap_or("");

    match typ {
        "kv.get" => {
            let key = eff.get("key").and_then(Value::as_str).unwrap_or("");
            if let Some(v) = effect_state.kv.get(key) {
                json!({
                  "id": id,
                  "type": "kv.get",
                  "ok": true,
                  "found": true,
                  "value": bytes_to_stream_payload(v),
                })
            } else {
                json!({
                  "id": id,
                  "type": "kv.get",
                  "ok": true,
                  "found": false,
                  "value": null,
                })
            }
        }
        "kv.put" => {
            let key = eff
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let value = eff.get("value").cloned().unwrap_or(Value::Null);
            match stream_payload_to_bytes(&value) {
                Ok(bytes) => {
                    effect_state.kv.insert(key, bytes);
                    json!({
                      "id": id,
                      "type": "kv.put",
                      "ok": true,
                      "written": true,
                    })
                }
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_EFFECT_KV_IO_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("kv.put decode failed: {err:#}"),
                    ));
                    json!({
                      "id": id,
                      "type": "kv.put",
                      "ok": false,
                      "error": truncate_4096(&format!("{err:#}")),
                    })
                }
            }
        }
        "log.emit" => {
            let level = eff.get("level").and_then(Value::as_str).unwrap_or("info");
            let message = eff.get("message").and_then(Value::as_str).unwrap_or("");
            eprintln!("http.reducer log[{level}]: {message}");
            json!({
              "id": id,
              "type": "log.emit",
              "ok": true,
              "emitted": true,
            })
        }
        "time.now" => {
            if let Some(caps) = effect_state.caps.as_ref() {
                if caps.clocks.mode == CapabilityMode::Deny {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_CAPS_CLOCK_DENIED",
                        Severity::Error,
                        Stage::Run,
                        "time.now denied by capabilities".to_string(),
                    ));
                    return json!({
                      "id": id,
                      "type": "time.now",
                      "ok": false,
                      "error": "time.now denied by capabilities",
                    });
                }
            }
            effect_state.nondeterminism.uses_os_time = true;
            let unix_ms = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_millis() as u64,
                Err(_) => 0,
            };
            json!({
              "id": id,
              "type": "time.now",
              "ok": true,
              "unix_ms": unix_ms,
            })
        }
        "http.fetch" => {
            let method_s = eff.get("method").and_then(Value::as_str).unwrap_or("GET");
            let url_s = eff.get("url").and_then(Value::as_str).unwrap_or("");
            let uri: Uri = match url_s.parse() {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("http.fetch url parse failed: {err}"),
                    ));
                    return json!({
                      "id": id,
                      "type": "http.fetch",
                      "ok": false,
                      "error": truncate_4096(&format!("url parse failed: {err}")),
                    });
                }
            };
            if uri.scheme_str() != Some("http") {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                    Severity::Error,
                    Stage::Run,
                    "http.fetch supports only http:// URLs".to_string(),
                ));
                return json!({
                  "id": id,
                  "type": "http.fetch",
                  "ok": false,
                  "error": "unsupported scheme (only http:// allowed)",
                });
            }
            if !effect_state.is_fetch_allowed(&uri) {
                let (code, msg) = if effect_state.caps.is_some() {
                    (
                        "X07WASM_CAPS_NET_DENIED",
                        "http.fetch denied by capabilities",
                    )
                } else {
                    (
                        "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                        "http.fetch host not allowlisted (set X07_WASM_HTTP_FETCH_ALLOW_HOSTS)",
                    )
                };
                diagnostics.push(Diagnostic::new(
                    code,
                    Severity::Error,
                    Stage::Run,
                    format!("{msg}: {:?}", uri.host()),
                ));
                return json!({
                  "id": id,
                  "type": "http.fetch",
                  "ok": false,
                  "error": msg,
                });
            }

            effect_state.nondeterminism.uses_network = true;

            let method: Method = match method_s.parse() {
                Ok(v) => v,
                Err(_) => Method::GET,
            };
            let mut builder = Request::builder().method(method).uri(uri.clone());

            if let Some(hdrs) = eff.get("headers").and_then(Value::as_array) {
                for h in hdrs {
                    let k = h.get("k").and_then(Value::as_str).unwrap_or("");
                    let v = h.get("v").and_then(Value::as_str).unwrap_or("");
                    let name = match HeaderName::from_bytes(k.as_bytes()) {
                        Ok(n) => n,
                        Err(_) => continue,
                    };
                    let val = match HeaderValue::from_str(v) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if let Some(hm) = builder.headers_mut() {
                        hm.append(name, val);
                    }
                }
            }

            let body = eff.get("body").cloned().unwrap_or(Value::Null);
            let body_bytes = match stream_payload_to_bytes(&body) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("http.fetch body decode failed: {err:#}"),
                    ));
                    return json!({
                      "id": id,
                      "type": "http.fetch",
                      "ok": false,
                      "error": truncate_4096(&format!("{err:#}")),
                    });
                }
            };

            let req = match builder.body(Full::new(Bytes::from(body_bytes))) {
                Ok(v) => v,
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("http.fetch build request failed: {err}"),
                    ));
                    return json!({
                      "id": id,
                      "type": "http.fetch",
                      "ok": false,
                      "error": truncate_4096(&format!("{err}")),
                    });
                }
            };

            match effect_state.client.request(req).await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let mut headers: Vec<(String, String)> = Vec::new();
                    for (k, v) in resp.headers().iter() {
                        if let Ok(v) = v.to_str() {
                            headers.push((k.to_string(), v.to_string()));
                        }
                    }
                    let body_bytes = match crate::http_component_host::collect_body_with_limit(
                        resp.into_body(),
                        1024 * 1024,
                    )
                    .await
                    {
                        Ok(v) => v,
                        Err(err) => {
                            diagnostics.push(Diagnostic::new(
                                "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                                Severity::Error,
                                Stage::Run,
                                format!("http.fetch collect body failed: {err:#}"),
                            ));
                            return json!({
                              "id": id,
                              "type": "http.fetch",
                              "ok": false,
                              "error": truncate_4096(&format!("{err:#}")),
                            });
                        }
                    };

                    let response =
                        crate::serve::response_envelope_value(id, status, &headers, &body_bytes);
                    json!({
                      "id": id,
                      "type": "http.fetch",
                      "ok": true,
                      "response": response,
                    })
                }
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_HTTP_EFFECT_HTTP_FETCH_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("http.fetch failed: {err}"),
                    ));
                    json!({
                      "id": id,
                      "type": "http.fetch",
                      "ok": false,
                      "error": truncate_4096(&format!("{err}")),
                    })
                }
            }
        }
        other => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_HTTP_EFFECT_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("unknown effect type: {other:?}"),
            ));
            json!({
              "id": id,
              "type": other,
              "ok": false,
              "error": "unknown effect type",
            })
        }
    }
}

fn truncate_4096(s: &str) -> String {
    if s.len() <= 4096 {
        return s.to_string();
    }
    s.chars().take(4096).collect()
}

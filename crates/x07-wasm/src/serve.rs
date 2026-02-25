use std::convert::Infallible;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine as _;
use bytes::Bytes;
use serde_json::{json, Value};
use tokio::task::LocalSet;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::HeaderName;
use hyper::{Method, Request, Response, Uri};

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::blob;
use crate::cli::{MachineArgs, Scope, ServeArgs, ServeMode};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_serve(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ServeArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = matches!(args.mode, ServeMode::Listen);
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let component_digest = match util::file_digest(&args.component) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_COMPONENT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read component {}: {err:#}",
                    args.component.display()
                ),
            ));
            report::meta::FileDigest {
                path: args.component.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let budgets = ServeBudgets {
        max_request_bytes: args.max_request_bytes as usize,
        max_response_bytes: args.max_response_bytes as usize,
        max_wall_ms_per_request: args.max_wall_ms_per_request,
        max_concurrent: args.max_concurrent as usize,
    };

    let mut config = Config::new();
    config.async_support(true);
    let engine = match Engine::new(&config) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASMTIME_ENGINE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let report_doc = serve_report_doc(
                meta,
                diagnostics,
                &args,
                budgets,
                component_digest,
                None,
                Vec::new(),
                Vec::new(),
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(2) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };

    let component = match Component::from_file(&engine, &args.component) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_COMPONENT_COMPILE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let report_doc = serve_report_doc(
                meta,
                diagnostics,
                &args,
                budgets,
                component_digest,
                None,
                Vec::new(),
                Vec::new(),
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };

    let mut linker: Linker<ServeState> = Linker::new(&engine);
    if let Err(err) = wasmtime_wasi::p2::add_to_linker_async(&mut linker) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SERVE_LINKER_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        let report_doc = serve_report_doc(
            meta,
            diagnostics,
            &args,
            budgets,
            component_digest,
            None,
            Vec::new(),
            Vec::new(),
        );
        let exit_code = report_doc
            .get("exit_code")
            .and_then(Value::as_u64)
            .unwrap_or(2) as u8;
        store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
        return Ok(exit_code);
    }
    if let Err(err) = wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SERVE_LINKER_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        let report_doc = serve_report_doc(
            meta,
            diagnostics,
            &args,
            budgets,
            component_digest,
            None,
            Vec::new(),
            Vec::new(),
        );
        let exit_code = report_doc
            .get("exit_code")
            .and_then(Value::as_u64)
            .unwrap_or(2) as u8;
        store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
        return Ok(exit_code);
    }

    let proxy_pre = match linker.instantiate_pre(&component) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_PROXY_PRE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let report_doc = serve_report_doc(
                meta,
                diagnostics,
                &args,
                budgets,
                component_digest,
                None,
                Vec::new(),
                Vec::new(),
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(2) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };
    let proxy_pre = match ProxyPre::new(proxy_pre) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_PROXY_PRE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let report_doc = serve_report_doc(
                meta,
                diagnostics,
                &args,
                budgets,
                component_digest,
                None,
                Vec::new(),
                Vec::new(),
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(2) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };

    let local = LocalSet::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("build tokio runtime")?;

    let (mut meta, diagnostics, bound_addr, responses, incident_dirs) =
        rt.block_on(local.run_until(async {
            match args.mode {
                ServeMode::Canary => {
                    serve_canary(
                        &engine,
                        &proxy_pre,
                        &args,
                        &component_digest,
                        &mut meta,
                        &mut diagnostics,
                        &budgets,
                    )
                    .await
                }
                ServeMode::Listen => {
                    serve_listen(
                        &engine,
                        &proxy_pre,
                        &args,
                        &component_digest,
                        &mut meta,
                        &mut diagnostics,
                        &budgets,
                    )
                    .await
                }
            }
        }));

    let report_doc = serve_report_doc(
        meta.clone(),
        diagnostics.clone(),
        &args,
        budgets,
        component_digest.clone(),
        bound_addr.clone(),
        responses.clone(),
        incident_dirs.clone(),
    );
    let exit_code = report_doc
        .get("exit_code")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u8;

    if report_doc.get("ok").and_then(Value::as_bool) == Some(false) {
        if !incident_dirs.is_empty() {
            meta.nondeterminism.uses_os_time = true;
        }
        let report_bytes = report::canon::canonical_json_bytes(&report_doc)?;
        for d in incident_dirs.iter() {
            let _ = std::fs::write(d.join("serve.report.json"), &report_bytes);
        }
    }

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[derive(Debug, Clone, Copy)]
struct ServeBudgets {
    max_request_bytes: usize,
    max_response_bytes: usize,
    max_wall_ms_per_request: u64,
    max_concurrent: usize,
}

#[derive(Debug, Clone)]
struct ServeResponseSummary {
    ok: bool,
    status: u16,
    body: Value,
    wall_ms: u64,
}

struct ServeState {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
    outgoing_body_chunk_size: usize,
    outgoing_body_buffer_chunks: usize,
}

impl WasiView for ServeState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for ServeState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    fn outgoing_body_chunk_size(&mut self) -> usize {
        self.outgoing_body_chunk_size
    }

    fn outgoing_body_buffer_chunks(&mut self) -> usize {
        self.outgoing_body_buffer_chunks
    }
}

async fn serve_canary(
    engine: &Engine,
    proxy: &ProxyPre<ServeState>,
    args: &ServeArgs,
    component: &report::meta::FileDigest,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    budgets: &ServeBudgets,
) -> (
    report::meta::ReportMeta,
    Vec<Diagnostic>,
    Option<String>,
    Vec<ServeResponseSummary>,
    Vec<PathBuf>,
) {
    let mut responses: Vec<ServeResponseSummary> = Vec::new();
    let mut incident_dirs: Vec<PathBuf> = Vec::new();

    let body_loaded = match blob::load_bytes_spec(&args.request_body, meta) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_REQUEST_BODY_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            return (
                meta.clone(),
                diagnostics.clone(),
                None,
                responses,
                incident_dirs,
            );
        }
    };

    let method = match Method::from_bytes(args.method.as_bytes()) {
        Ok(m) => m,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_METHOD_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("invalid method {:?}: {err}", args.method),
            ));
            return (
                meta.clone(),
                diagnostics.clone(),
                None,
                responses,
                incident_dirs,
            );
        }
    };

    let needs_default_authority = !args.path.contains("://");
    let uri_str = if !needs_default_authority {
        args.path.clone()
    } else {
        let mut p = args.path.clone();
        if !p.starts_with('/') {
            p.insert(0, '/');
        }
        format!("http://localhost{p}")
    };

    let uri: Uri = match uri_str.parse() {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_PATH_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("invalid path {:?}: {err}", args.path),
            ));
            return (
                meta.clone(),
                diagnostics.clone(),
                None,
                responses,
                incident_dirs,
            );
        }
    };

    for _ in 0..args.stop_after {
        let mut b = Request::builder().method(method.clone()).uri(uri.clone());
        if needs_default_authority {
            b = b.header(hyper::header::HOST, "localhost");
        }
        let req = b
            .body(full_body(body_loaded.bytes.clone()))
            .expect("request build");
        let headers = req.headers().clone();

        let started = std::time::Instant::now();
        match handle_one_request(engine, proxy, req, budgets).await {
            Ok(buf) => {
                let wall_ms = started.elapsed().as_millis() as u64;
                responses.push(ServeResponseSummary {
                    ok: true,
                    status: buf.status,
                    body: blob_ref_for_bytes(&buf.body),
                    wall_ms,
                });
            }
            Err(err) => {
                let wall_ms = started.elapsed().as_millis() as u64;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SERVE_REQUEST_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                let (req_env_bytes, req_env_doc, req_sha) =
                    request_envelope_bytes(&method, &uri, &headers, &body_loaded.bytes);
                let incident = match write_http_incident(
                    &args.incidents_dir,
                    component,
                    &req_env_bytes,
                    &req_env_doc,
                    &body_loaded.bytes,
                    None,
                    None,
                    diagnostics,
                    err.to_string(),
                    budgets,
                ) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INCIDENT_BUNDLE_WRITE_FAILED",
                            Severity::Warning,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        None
                    }
                };
                if let Some(dir) = incident {
                    meta.nondeterminism.uses_os_time = true;
                    incident_dirs.push(dir);
                }
                responses.push(ServeResponseSummary {
                    ok: false,
                    status: 0,
                    body: json!({ "bytes_len": 0, "sha256": "0".repeat(64) }),
                    wall_ms,
                });
                let _ = (req_sha, wall_ms);
            }
        }
    }

    (
        meta.clone(),
        diagnostics.clone(),
        None,
        responses,
        incident_dirs,
    )
}

async fn serve_listen(
    engine: &Engine,
    proxy: &ProxyPre<ServeState>,
    args: &ServeArgs,
    component: &report::meta::FileDigest,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    budgets: &ServeBudgets,
) -> (
    report::meta::ReportMeta,
    Vec<Diagnostic>,
    Option<String>,
    Vec<ServeResponseSummary>,
    Vec<PathBuf>,
) {
    let responses_acc: std::sync::Arc<std::sync::Mutex<Vec<ServeResponseSummary>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let incident_dirs_acc: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let addr: SocketAddr = match args.addr.parse() {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_ADDR_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("invalid --addr {:?}: {err}", args.addr),
            ));
            return (
                meta.clone(),
                diagnostics.clone(),
                None,
                responses_acc.lock().unwrap().clone(),
                incident_dirs_acc.lock().unwrap().clone(),
            );
        }
    };

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_LISTEN_FAILED",
                Severity::Error,
                Stage::Run,
                format!("listen failed on {addr}: {err}"),
            ));
            return (
                meta.clone(),
                diagnostics.clone(),
                None,
                responses_acc.lock().unwrap().clone(),
                incident_dirs_acc.lock().unwrap().clone(),
            );
        }
    };
    let bound_addr = match listener.local_addr() {
        Ok(v) => v,
        Err(_) => addr,
    };

    let stop_after = args.stop_after;
    let mut handled: u32 = 0;

    while stop_after == 0 || handled < stop_after {
        let (stream, _peer) = match listener.accept().await {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SERVE_ACCEPT_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("accept failed: {err}"),
                ));
                break;
            }
        };

        handled = handled.saturating_add(1);

        let io = hyper_util::rt::TokioIo::new(stream);
        let proxy = proxy.clone();
        let component = component.clone();
        let budgets = *budgets;
        let incidents_dir = args.incidents_dir.clone();
        let responses_acc = responses_acc.clone();
        let incident_dirs_acc = incident_dirs_acc.clone();

        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
            let proxy = proxy.clone();
            let component = component.clone();
            let budgets = budgets;
            let incidents_dir = incidents_dir.clone();
            let responses_acc = responses_acc.clone();
            let incident_dirs_acc = incident_dirs_acc.clone();
            async move {
                let (parts, body) = req.into_parts();
                let body_bytes =
                    match collect_body_with_limit(body, budgets.max_request_bytes).await {
                        Ok(v) => v,
                        Err(err) => {
                            let wall_ms = 0u64;
                            responses_acc.lock().unwrap().push(ServeResponseSummary {
                                ok: false,
                                status: 413,
                                body: json!({ "bytes_len": 0, "sha256": "0".repeat(64) }),
                                wall_ms,
                            });
                            let _ = err;
                            return Ok::<_, hyper::Error>(
                                Response::builder()
                                    .status(413)
                                    .body(Full::new(Bytes::from_static(b"request too large")))
                                    .unwrap(),
                            );
                        }
                    };
                let req2 = Request::from_parts(parts, full_body(body_bytes.clone()));
                let method = req2.method().clone();
                let uri = req2.uri().clone();
                let headers = req2.headers().clone();
                let started = std::time::Instant::now();
                match handle_one_request(engine, &proxy, req2, &budgets).await {
                    Ok(buf) => {
                        let wall_ms = started.elapsed().as_millis() as u64;
                        responses_acc.lock().unwrap().push(ServeResponseSummary {
                            ok: true,
                            status: buf.status,
                            body: blob_ref_for_bytes(&buf.body),
                            wall_ms,
                        });

                        let mut resp = Response::builder().status(buf.status);
                        for (k, v) in buf.headers {
                            if let (Ok(k), Ok(v)) = (
                                HeaderName::from_bytes(k.as_bytes()),
                                v.parse::<hyper::header::HeaderValue>(),
                            ) {
                                resp = resp.header(k, v);
                            }
                        }
                        let body = Full::new(Bytes::from(buf.body));
                        let resp = resp.body(body).unwrap();
                        Ok::<_, hyper::Error>(resp)
                    }
                    Err(err) => {
                        let wall_ms = started.elapsed().as_millis() as u64;
                        let (_req_env_bytes, req_env_doc, _req_sha) =
                            request_envelope_bytes(&method, &uri, &headers, &body_bytes);
                        let incident = write_http_incident(
                            &incidents_dir,
                            &component,
                            &_req_env_bytes,
                            &req_env_doc,
                            &body_bytes,
                            Some(500),
                            None,
                            &mut Vec::new(),
                            err.to_string(),
                            &budgets,
                        );
                        if let Ok(dir) = incident {
                            incident_dirs_acc.lock().unwrap().push(dir);
                        }
                        responses_acc.lock().unwrap().push(ServeResponseSummary {
                            ok: false,
                            status: 500,
                            body: json!({ "bytes_len": 0, "sha256": "0".repeat(64) }),
                            wall_ms,
                        });
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(500)
                                .body(Full::new(Bytes::from_static(b"internal error")))
                                .unwrap(),
                        )
                    }
                }
            }
        });

        if let Err(err) = hyper::server::conn::http1::Builder::new()
            .keep_alive(false)
            .serve_connection(io, svc)
            .await
        {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SERVE_HTTP1_CONN_FAILED",
                Severity::Warning,
                Stage::Run,
                format!("serve_connection failed: {err}"),
            ));
        }
    }

    let responses = responses_acc.lock().unwrap().clone();
    let incident_dirs = incident_dirs_acc.lock().unwrap().clone();

    if !incident_dirs.is_empty() {
        meta.nondeterminism.uses_os_time = true;
    }

    (
        meta.clone(),
        diagnostics.clone(),
        Some(bound_addr.to_string()),
        responses,
        incident_dirs,
    )
}

struct BufferedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn handle_one_request<B>(
    engine: &Engine,
    proxy: &ProxyPre<ServeState>,
    req: Request<B>,
    budgets: &ServeBudgets,
) -> Result<BufferedResponse>
where
    B: http_body::Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
{
    let outgoing_body_chunk_size = budgets.max_response_bytes.clamp(1, 16 * 1024);
    let outgoing_body_buffer_chunks = budgets
        .max_response_bytes
        .div_ceil(outgoing_body_chunk_size)
        .max(1);

    let wasi = WasiCtxBuilder::new().build();
    let state = ServeState {
        table: ResourceTable::new(),
        wasi,
        http: WasiHttpCtx::new(),
        outgoing_body_chunk_size,
        outgoing_body_buffer_chunks,
    };
    let mut store = Store::new(engine, state);
    store.data_mut().table.set_max_capacity(1024);

    let proxy = proxy.instantiate_async(&mut store).await?;

    let scheme = wasmtime_wasi_http::bindings::http::types::Scheme::Http;
    let req = store.data_mut().new_incoming_request(scheme, req)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let out = store.data_mut().new_response_outparam(sender)?;

    let fut = proxy
        .wasi_http_incoming_handler()
        .call_handle(&mut store, req, out);

    let res = tokio::time::timeout(
        std::time::Duration::from_millis(budgets.max_wall_ms_per_request),
        fut,
    )
    .await;

    match res {
        Ok(Ok(())) => {}
        Ok(Err(err)) => anyhow::bail!("{err:#}"),
        Err(_) => anyhow::bail!("timeout"),
    }

    let resp = receiver.await.context("response_outparam recv")??;
    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let mut headers = Vec::new();
    for (k, v) in parts.headers.iter() {
        if let Ok(v) = v.to_str() {
            headers.push((k.to_string(), v.to_string()));
        }
    }

    let body_bytes = collect_body_with_limit(body, budgets.max_response_bytes).await?;

    Ok(BufferedResponse {
        status,
        headers,
        body: body_bytes,
    })
}

fn full_body(bytes: Vec<u8>) -> impl http_body::Body<Data = Bytes, Error = hyper::Error> {
    Full::new(Bytes::from(bytes)).map_err(|never: Infallible| match never {})
}

async fn collect_body_with_limit<B>(body: B, max_bytes: usize) -> Result<Vec<u8>>
where
    B: http_body::Body<Data = Bytes> + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let collected = Limited::new(body, max_bytes)
        .collect()
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(collected.to_bytes().to_vec())
}

fn blob_ref_for_bytes(bytes: &[u8]) -> Value {
    json!({
      "bytes_len": bytes.len() as u64,
      "sha256": util::sha256_hex(bytes),
      "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn request_envelope_bytes(
    method: &Method,
    uri: &Uri,
    headers: &hyper::HeaderMap,
    body: &[u8],
) -> (Vec<u8>, Value, String) {
    let env = request_envelope_value(method, uri, headers, body);
    let bytes = report::canon::canonical_json_bytes(&env).unwrap_or_else(|_| b"{}\n".to_vec());
    let sha = util::sha256_hex(&bytes);
    (bytes, env, sha)
}

fn request_envelope_value(
    method: &Method,
    uri: &Uri,
    headers: &hyper::HeaderMap,
    body: &[u8],
) -> Value {
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

    let mut hdrs: Vec<(String, String)> = Vec::new();
    for (k, v) in headers.iter() {
        let val = v.to_str().unwrap_or("").to_string();
        hdrs.push((k.to_string(), val));
    }
    hdrs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let headers_json = hdrs
        .into_iter()
        .map(|(k, v)| json!([k, v]))
        .collect::<Vec<_>>();

    json!({
      "v": 1,
      "kind": "x07.http.request",
      "method": method.as_str(),
      "path": path,
      "query": query,
      "headers": headers_json,
      "body_b64": base64::engine::general_purpose::STANDARD.encode(body),
    })
}

fn response_envelope_value(status: u16, headers: &[(String, String)], body: &[u8]) -> Value {
    let hdrs = headers
        .iter()
        .map(|(k, v)| json!([k, v]))
        .collect::<Vec<_>>();
    json!({
      "v": 1,
      "kind": "x07.http.response",
      "status": status,
      "headers": hdrs,
      "body_b64": base64::engine::general_purpose::STANDARD.encode(body),
    })
}

#[allow(clippy::too_many_arguments)]
fn write_http_incident(
    incidents_dir: &Path,
    component: &report::meta::FileDigest,
    request_envelope_bytes: &[u8],
    request_envelope_doc: &Value,
    request_body: &[u8],
    response_status: Option<u16>,
    response: Option<&BufferedResponse>,
    diagnostics: &mut Vec<Diagnostic>,
    error: String,
    budgets: &ServeBudgets,
) -> Result<PathBuf> {
    let req_sha = util::sha256_hex(request_envelope_bytes);
    let incident_id = util::sha256_hex(format!("{}:{}", component.sha256, req_sha).as_bytes());
    let incident_id = incident_id[..32].to_string();
    let date = crate::wasm::incident::utc_date_yyyy_mm_dd();
    let dir = incidents_dir.join(date).join(incident_id);
    std::fs::create_dir_all(&dir).with_context(|| format!("create dir: {}", dir.display()))?;

    let request_body_sha = util::sha256_hex(request_body);
    std::fs::write(dir.join("request.envelope.json"), request_envelope_bytes)
        .with_context(|| format!("write: {}", dir.join("request.envelope.json").display()))?;
    std::fs::write(dir.join("request.body.bin"), request_body)
        .with_context(|| format!("write: {}", dir.join("request.body.bin").display()))?;
    std::fs::write(
        dir.join("request.body.sha256"),
        format!("{request_body_sha}\n"),
    )
    .with_context(|| format!("write: {}", dir.join("request.body.sha256").display()))?;

    if let Some(resp) = response {
        let env = response_envelope_value(resp.status, &resp.headers, &resp.body);
        let bytes = report::canon::canonical_json_bytes(&env)?;
        std::fs::write(dir.join("response.envelope.json"), bytes)
            .with_context(|| format!("write: {}", dir.join("response.envelope.json").display()))?;
    } else if let Some(status) = response_status {
        let env = response_envelope_value(status, &[], &[]);
        let bytes = report::canon::canonical_json_bytes(&env)?;
        let _ = std::fs::write(dir.join("response.envelope.json"), bytes);
    }

    let component_manifest_path =
        PathBuf::from(&component.path).with_extension("wasm.manifest.json");
    if component_manifest_path.is_file() {
        if let Ok(bytes) = std::fs::read(&component_manifest_path) {
            let _ = std::fs::write(dir.join("component.manifest.json"), bytes);
        }
    }

    let incident_manifest = json!({
      "schema_version": "x07.wasm.incident.manifest@0.1.0",
      "kind": "serve",
      "component": component,
      "request": {
        "envelope": request_envelope_doc,
        "body_sha256": request_body_sha,
      },
      "budgets": {
        "max_request_bytes": budgets.max_request_bytes,
        "max_response_bytes": budgets.max_response_bytes,
        "max_wall_ms_per_request": budgets.max_wall_ms_per_request,
      },
      "error": error,
    });
    let bytes = report::canon::canonical_json_bytes(&incident_manifest)?;
    std::fs::write(dir.join("incident.manifest.json"), bytes)
        .with_context(|| format!("write: {}", dir.join("incident.manifest.json").display()))?;

    let _ = diagnostics;
    Ok(dir)
}

#[allow(clippy::too_many_arguments)]
fn serve_report_doc(
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    args: &ServeArgs,
    budgets: ServeBudgets,
    component: report::meta::FileDigest,
    bound_addr: Option<String>,
    responses: Vec<ServeResponseSummary>,
    incident_dirs: Vec<PathBuf>,
) -> Value {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    json!({
      "schema_version": "x07.wasm.serve.report@0.1.0",
      "command": "x07-wasm.serve",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": match args.mode { ServeMode::Canary => "canary", ServeMode::Listen => "listen" },
        "component": component,
        "addr": { "requested": args.addr, "bound": bound_addr },
        "stop_after": args.stop_after,
        "budgets": {
          "max_request_bytes": budgets.max_request_bytes,
          "max_response_bytes": budgets.max_response_bytes,
          "max_wall_ms_per_request": budgets.max_wall_ms_per_request,
          "max_concurrent": budgets.max_concurrent
        },
        "responses": responses.iter().map(|r| json!({
          "ok": r.ok,
          "status": r.status,
          "body": r.body,
          "wall_ms": r.wall_ms
        })).collect::<Vec<_>>(),
        "incidents": incident_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
      }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_schema_accepts_placeholder() {
        let store = SchemaStore::new().unwrap();
        let raw_argv: Vec<OsString> = Vec::new();
        let meta = report::meta::tool_meta(&raw_argv, std::time::Instant::now());
        let doc = json!({
          "schema_version": "x07.wasm.serve.report@0.1.0",
          "command": "x07-wasm.serve",
          "ok": false,
          "exit_code": 1,
          "diagnostics": [],
          "meta": meta,
          "result": {
            "mode": "canary",
            "component": { "path": "a.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "addr": { "requested": "127.0.0.1:0", "bound": "127.0.0.1:0" },
            "stop_after": 1,
            "budgets": {
              "max_request_bytes": 1024,
              "max_response_bytes": 1024,
              "max_wall_ms_per_request": 1000,
              "max_concurrent": 1
            },
            "responses": [],
            "incidents": []
          }
        });
        let diags = store
            .validate(
                "https://x07.io/spec/x07-wasm.serve.report.schema.json",
                &doc,
            )
            .unwrap();
        assert!(diags.is_empty(), "schema diags: {diags:?}");
    }
}

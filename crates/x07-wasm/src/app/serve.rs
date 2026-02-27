use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Value};
use tokio::task::LocalSet;

use crate::app::bundle::LoadedAppBundle;
use crate::caps::doc::CapabilitiesDoc;
use crate::cli::{AppServeArgs, AppServeMode, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::http_component_host::{self, HttpComponentBudgets, HttpComponentHost};
use crate::ops::load_ops_profile_with_refs;
use crate::report;
use crate::schema::SchemaStore;
use crate::slo::eval::evaluate_slo_docs;

pub fn cmd_app_serve(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppServeArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = true;
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
            &args.addr,
            &args.api_prefix,
            false,
            None,
            args.strict_mime,
            None,
        );
    };
    let bundle_json = bundle.doc_json.clone();

    let frontend_dir = args.dir.join(&bundle.doc.frontend.dir_rel);
    if !frontend_dir.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_SERVE_FRONTEND_DIR_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("frontend dir not found: {}", frontend_dir.display()),
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
            &args.addr,
            &args.api_prefix,
            false,
            None,
            args.strict_mime,
            None,
        );
    }

    let backend_component_path = args.dir.join(&bundle.doc.backend.artifact.path);
    if !backend_component_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_SERVE_BACKEND_COMPONENT_MISSING",
            Severity::Error,
            Stage::Parse,
            format!(
                "backend component not found: {}",
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
            &args.addr,
            &args.api_prefix,
            false,
            None,
            args.strict_mime,
            None,
        );
    }

    let mut caps: Option<Arc<CapabilitiesDoc>> = None;
    let mut slo_profile_doc_json: Option<Value> = None;
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
                &args.addr,
                &args.api_prefix,
                false,
                None,
                args.strict_mime,
                None,
            );
        };

        match serde_json::from_value::<CapabilitiesDoc>(loaded_ops.capabilities.doc_json.clone()) {
            Ok(v) => caps = Some(Arc::new(v)),
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_CAPS_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse capabilities doc: {err}"),
            )),
        }
        slo_profile_doc_json = loaded_ops.slo_profile.as_ref().map(|s| s.doc_json.clone());

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
                &args.addr,
                &args.api_prefix,
                false,
                None,
                args.strict_mime,
                None,
            );
        }
    }

    let host = match HttpComponentHost::from_component_file(&backend_component_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_SERVE_BACKEND_HOST_INIT_FAILED",
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
                &args.addr,
                &args.api_prefix,
                false,
                None,
                args.strict_mime,
                None,
            );
        }
    };

    let (budgets, max_concurrency, profile_strict_mime, profile_api_prefix, profile_addr) =
        load_app_serve_settings(&store, &bundle, &mut meta, &mut diagnostics);
    let effective_strict_mime = args.strict_mime || profile_strict_mime;
    let effective_api_prefix = if args.api_prefix == "/api" {
        profile_api_prefix
    } else {
        args.api_prefix.clone()
    };
    let effective_addr_str = if args.addr == "127.0.0.1:0" {
        profile_addr
    } else {
        args.addr.clone()
    };

    // For `port=0`, report the actual bound addr string.
    let addr: SocketAddr = match effective_addr_str.parse() {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_SERVE_ADDR_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("invalid addr {:?}: {err}", effective_addr_str),
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
                &effective_addr_str,
                &effective_api_prefix,
                false,
                None,
                effective_strict_mime,
                None,
            );
        }
    };

    let diag_acc: Arc<Mutex<Vec<Diagnostic>>> = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(AppServeState {
        bundle_doc_json: bundle_json,
        frontend_dir,
        api_prefix: effective_api_prefix.clone(),
        host: Arc::new(host),
        budgets,
        caps,
        wasi_base_dir: args.dir.clone(),
        diag_acc: diag_acc.clone(),
        max_concurrency: Arc::new(tokio::sync::Semaphore::new(max_concurrency)),
    });
    let local = LocalSet::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("build tokio runtime")?;

    let (bound_addr, wasm_mime_ok, canary_statuses) = rt.block_on(local.run_until(async {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(v) => v,
            Err(err) => {
                diag_acc.lock().unwrap().push(Diagnostic::new(
                    "X07WASM_APP_SERVE_BIND_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("bind failed: {err}"),
                ));
                return (None, false, None);
            }
        };
        let bound_addr = listener.local_addr().ok().map(|a| a.to_string());
        let bound_sock = listener.local_addr().ok();

        match args.mode {
            AppServeMode::Listen => {
                let _ = serve_listener(listener, state.clone(), None, diag_acc.clone()).await;
                // Listen mode runs until an accept/connection error; no smoke check.
                (bound_addr, true, None)
            }
            AppServeMode::Smoke => {
                let stop_after = 1u32;
                let server = tokio::spawn(serve_listener(
                    listener,
                    state.clone(),
                    Some(stop_after),
                    diag_acc.clone(),
                ));

                let wasm_mime_ok = if let Some(sock) = bound_sock {
                    match tokio::task::spawn_blocking(move || smoke_check_wasm_mime(sock)).await {
                        Ok(Ok(ok)) => ok,
                        _ => false,
                    }
                } else {
                    false
                };

                let _ = server.await;
                (bound_addr, wasm_mime_ok, None)
            }
            AppServeMode::Canary => {
                // GET /, GET /app.bundle.json, GET /app.wasm, GET {api_prefix}/.
                let stop_after = 4u32;
                let server = tokio::spawn(serve_listener(
                    listener,
                    state.clone(),
                    Some(stop_after),
                    diag_acc.clone(),
                ));

                let mut wasm_mime_ok = false;
                let mut statuses: Vec<u16> = Vec::new();
                if let Some(sock) = bound_sock {
                    let api_prefix = state.api_prefix.clone();
                    let canary = tokio::task::spawn_blocking(move || {
                        let r0 = simple_http_request(sock, "GET", "/", None)?;
                        let r1 = simple_http_request(sock, "GET", "/app.bundle.json", None)?;
                        let (_mime, mime_ok, wasm_status) =
                            simple_http_get_mime(sock, "/app.wasm")?;
                        let mut api_path = api_prefix;
                        if !api_path.ends_with('/') {
                            api_path.push('/');
                        }
                        let api = simple_http_request(sock, "GET", &api_path, None)?;
                        Ok::<_, anyhow::Error>((
                            mime_ok,
                            vec![r0.status, r1.status, wasm_status, api.status],
                        ))
                    });
                    match canary.await {
                        Ok(Ok((ok, canary_statuses))) => {
                            wasm_mime_ok = ok;
                            statuses = canary_statuses;
                        }
                        Ok(Err(err)) => {
                            diag_acc.lock().unwrap().push(Diagnostic::new(
                                "X07WASM_APP_SERVE_CANARY_FAILED",
                                Severity::Error,
                                Stage::Run,
                                format!("{err:#}"),
                            ));
                        }
                        Err(err) => {
                            diag_acc.lock().unwrap().push(Diagnostic::new(
                                "X07WASM_APP_SERVE_CANARY_FAILED",
                                Severity::Error,
                                Stage::Run,
                                format!("canary task failed: {err}"),
                            ));
                        }
                    }
                }

                let _ = server.await;
                (bound_addr, wasm_mime_ok, Some(statuses))
            }
        }
    }));

    diagnostics.extend(diag_acc.lock().unwrap().iter().cloned());

    let mut canary_result: Option<Value> = None;
    if matches!(args.mode, AppServeMode::Canary) {
        if let (Some(slo_profile), Some(statuses)) = (
            slo_profile_doc_json.as_ref(),
            canary_statuses.as_deref().filter(|s| !s.is_empty()),
        ) {
            let metrics_snapshot = metrics_snapshot_for_canary(slo_profile, statuses);
            let metrics_schema_diags = store.validate(
                "https://x07.io/spec/x07-metrics.snapshot.schema.json",
                &metrics_snapshot,
            )?;
            if !metrics_schema_diags.is_empty() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_METRICS_SNAPSHOT_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    "generated metrics snapshot schema invalid".to_string(),
                ));
                diagnostics.extend(metrics_schema_diags);
            }

            let outcome = evaluate_slo_docs(slo_profile, &metrics_snapshot, &mut diagnostics);
            canary_result = Some(json!({
              "metrics_snapshot": metrics_snapshot,
              "slo_decision": outcome.decision,
              "slo_violations": outcome.violations,
            }));
        }
    }

    if effective_strict_mime && !wasm_mime_ok {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_SERVE_WASM_MIME_INVALID",
            Severity::Error,
            Stage::Run,
            "expected application/wasm (exact) for /app.wasm".to_string(),
        ));
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
        &effective_addr_str,
        &effective_api_prefix,
        wasm_mime_ok,
        bound_addr,
        effective_strict_mime,
        canary_result,
    )
}

struct AppServeState {
    bundle_doc_json: Value,
    frontend_dir: PathBuf,
    api_prefix: String,
    host: Arc<HttpComponentHost>,
    budgets: HttpComponentBudgets,
    caps: Option<Arc<CapabilitiesDoc>>,
    wasi_base_dir: PathBuf,
    diag_acc: Arc<Mutex<Vec<Diagnostic>>>,
    max_concurrency: Arc<tokio::sync::Semaphore>,
}

async fn serve_listener(
    listener: tokio::net::TcpListener,
    state: Arc<AppServeState>,
    stop_after: Option<u32>,
    diag_acc: Arc<Mutex<Vec<Diagnostic>>>,
) -> Result<()> {
    let mut handled: u32 = 0;
    let mut tasks: Option<Vec<tokio::task::JoinHandle<()>>> = stop_after.map(|_| Vec::new());

    while stop_after.is_none() || handled < stop_after.unwrap_or(0) {
        let (stream, _peer) = match listener.accept().await {
            Ok(v) => v,
            Err(err) => {
                diag_acc.lock().unwrap().push(Diagnostic::new(
                    "X07WASM_APP_SERVE_ACCEPT_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("accept failed: {err}"),
                ));
                break;
            }
        };
        handled = handled.saturating_add(1);

        let io = hyper_util::rt::TokioIo::new(stream);
        let state2 = state.clone();
        let diag_acc2 = diag_acc.clone();
        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
            let state3 = state2.clone();
            async move { handle_one_request(req, state3).await }
        });

        let task = tokio::spawn(async move {
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .keep_alive(false)
                .serve_connection(io, svc)
                .await
            {
                diag_acc2.lock().unwrap().push(Diagnostic::new(
                    "X07WASM_APP_SERVE_HTTP1_CONN_FAILED",
                    Severity::Warning,
                    Stage::Run,
                    format!("serve_connection failed: {err}"),
                ));
            }
        });
        if let Some(v) = tasks.as_mut() {
            v.push(task);
        }
    }

    if let Some(tasks) = tasks {
        for t in tasks {
            let _ = t.await;
        }
    }
    Ok(())
}

async fn handle_one_request(
    req: Request<Incoming>,
    state: Arc<AppServeState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Serve the bundle manifest at /app.bundle.json (bundle root).
    if path == "/app.bundle.json" {
        return serve_bytes(
            method,
            &state.bundle_doc_json,
            "application/json; charset=utf-8",
        );
    }

    if is_api_path(&state.api_prefix, &path) {
        return proxy_api_request(req, state).await;
    }

    serve_static_request(method, &path, &state.frontend_dir)
}

fn is_api_path(api_prefix: &str, path: &str) -> bool {
    if api_prefix.is_empty() || api_prefix == "/" {
        return true;
    }
    if path == api_prefix {
        return true;
    }
    let p = if api_prefix.ends_with('/') {
        api_prefix.to_string()
    } else {
        format!("{api_prefix}/")
    };
    path.starts_with(&p)
}

async fn proxy_api_request(
    req: Request<Incoming>,
    state: Arc<AppServeState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let _permit = match state.max_concurrency.acquire().await {
        Ok(p) => p,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Full::new(Bytes::from_static(b"server shutting down")))
                .unwrap());
        }
    };

    let (parts, body) = req.into_parts();
    let body_bytes =
        match http_component_host::collect_body_with_limit(body, state.budgets.max_request_bytes)
            .await
        {
            Ok(v) => v,
            Err(_err) => {
                return Ok(Response::builder()
                    .status(StatusCode::PAYLOAD_TOO_LARGE)
                    .body(Full::new(Bytes::from_static(b"request too large")))
                    .unwrap());
            }
        };

    let req2 = Request::from_parts(parts, http_component_host::full_body(body_bytes.clone()));
    let mut request_diags: Vec<Diagnostic> = Vec::new();
    match state
        .host
        .handle_request(
            req2,
            &state.budgets,
            state.caps.clone(),
            &state.wasi_base_dir,
            &mut request_diags,
        )
        .await
    {
        Ok(buf) => {
            if !request_diags.is_empty() {
                state.diag_acc.lock().unwrap().extend(request_diags);
            }
            let mut resp = Response::builder().status(buf.status);
            for (k, v) in buf.headers {
                if let (Ok(k), Ok(v)) = (
                    HeaderName::from_bytes(k.as_bytes()),
                    HeaderValue::from_str(&v),
                ) {
                    resp = resp.header(k, v);
                }
            }
            Ok(resp.body(Full::new(Bytes::from(buf.body))).unwrap())
        }
        Err(err) => {
            let _ = err;
            if !request_diags.is_empty() {
                state.diag_acc.lock().unwrap().extend(request_diags);
            }
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from_static(b"internal error")))
                .unwrap())
        }
    }
}

fn serve_static_request(
    method: Method,
    path: &str,
    dir: &Path,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    if method != Method::GET && method != Method::HEAD {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::new(Bytes::from_static(b"method not allowed")))
            .unwrap());
    }

    let rel = sanitize_path(path);
    let rel = if rel.is_empty() {
        "index.html".to_string()
    } else {
        rel
    };
    let full = dir.join(&rel);
    if !full.is_file() {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from_static(b"not found")))
            .unwrap());
    }

    let mime = mime_for_path(&full);
    if method == Method::HEAD {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::CONTENT_TYPE, mime)
            .body(Full::new(Bytes::new()))
            .unwrap());
    }
    let body = match std::fs::read(&full) {
        Ok(v) => v,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from_static(b"read failed")))
                .unwrap());
        }
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::CONTENT_TYPE, mime)
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "wasm" => "application/wasm",
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn sanitize_path(raw: &str) -> String {
    let mut s = raw.split('?').next().unwrap_or("").to_string();
    if s.starts_with('/') {
        s = s[1..].to_string();
    }
    let mut parts = Vec::new();
    for p in s.split('/') {
        if p.is_empty() || p == "." || p == ".." {
            continue;
        }
        parts.push(p);
    }
    parts.join("/")
}

fn serve_bytes(
    method: Method,
    doc: &Value,
    content_type: &'static str,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    if method != Method::GET && method != Method::HEAD {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::new(Bytes::from_static(b"method not allowed")))
            .unwrap());
    }
    let bytes =
        report::canon::canonical_pretty_json_bytes(doc).unwrap_or_else(|_| b"{}\n".to_vec());
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        Bytes::from(bytes)
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::CONTENT_TYPE, content_type)
        .body(Full::new(body))
        .unwrap())
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
    args: &AppServeArgs,
    effective_addr: &str,
    effective_api_prefix: &str,
    wasm_mime_ok: bool,
    bound_addr: Option<String>,
    effective_strict_mime: bool,
    canary: Option<Value>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let mode = match args.mode {
        AppServeMode::Listen => "listen",
        AppServeMode::Smoke => "smoke",
        AppServeMode::Canary => "canary",
    };
    let mut stdout_json = json!({
      "dir": args.dir.display().to_string(),
      "addr": bound_addr.unwrap_or_else(|| effective_addr.to_string()),
      "mode": mode,
      "api_prefix": effective_api_prefix,
      "strict_wasm_mime": effective_strict_mime,
      "wasm_mime_ok": wasm_mime_ok
    });
    if let Some(canary) = canary {
        if let Some(obj) = stdout_json.as_object_mut() {
            obj.insert("canary".to_string(), canary);
        }
    }

    let report_doc = json!({
      "schema_version": "x07.wasm.app.serve.report@0.1.0",
      "command": "x07-wasm.app.serve",
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

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

// --- smoke/canary client helpers (blocking; used via spawn_blocking) ---

struct SimpleHttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
}

fn simple_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> Result<SimpleHttpResponse> {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let mut stream = TcpStream::connect(addr).context("connect")?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();

    let body = body.unwrap_or(&[]);
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes()).context("write request")?;
    if !body.is_empty() {
        stream.write_all(body).context("write body")?;
    }
    stream.flush().ok();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).context("read response")?;

    parse_simple_http_response(&buf)
}

fn parse_simple_http_response(buf: &[u8]) -> Result<SimpleHttpResponse> {
    let text = String::from_utf8_lossy(buf);
    let mut lines = text.lines();
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse::<u16>()
        .unwrap_or(0);

    let mut headers = Vec::new();
    let mut header_bytes_len = 0usize;
    for line in text.lines() {
        header_bytes_len += line.len() + 1;
        if line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    // Best-effort split (header_bytes_len is approximate due to \r\n).
    let _ = header_bytes_len;
    Ok(SimpleHttpResponse { status, headers })
}

fn simple_http_get_mime(addr: SocketAddr, path: &str) -> Result<(String, bool, u16)> {
    let resp = simple_http_request(addr, "GET", path, None)?;
    let mut mime = String::new();
    for (k, v) in resp.headers {
        if k.eq_ignore_ascii_case("content-type") {
            mime = v;
            break;
        }
    }
    let ok = mime.eq_ignore_ascii_case("application/wasm");
    Ok((mime, ok, resp.status))
}

fn smoke_check_wasm_mime(addr: SocketAddr) -> Result<bool> {
    let (_mime, ok, _status) = simple_http_get_mime(addr, "/app.wasm")?;
    Ok(ok)
}

fn metrics_snapshot_for_canary(slo_profile_doc: &Value, statuses: &[u16]) -> Value {
    let total = statuses.len().max(1) as f64;
    let errors = statuses.iter().filter(|&&s| s >= 400).count() as f64;
    let error_rate = errors / total;
    let availability = 1.0 - error_rate;

    let service = slo_profile_doc
        .get("service")
        .and_then(Value::as_str)
        .unwrap_or("app");

    let mut metrics: Vec<Value> = Vec::new();
    if let Some(arr) = slo_profile_doc.get("indicators").and_then(Value::as_array) {
        for ind in arr {
            let kind = ind.get("kind").and_then(Value::as_str).unwrap_or("");
            let metric = ind.get("metric").and_then(Value::as_str).unwrap_or("");
            if metric.trim().is_empty() {
                continue;
            }
            match kind {
                "error_rate" => {
                    metrics.push(json!({ "name": metric, "value": error_rate, "unit": "ratio" }))
                }
                "availability" => {
                    metrics.push(json!({ "name": metric, "value": availability, "unit": "ratio" }))
                }
                "latency_p95_ms" => {
                    metrics.push(json!({ "name": metric, "value": 0, "unit": "ms" }))
                }
                _ => {}
            }
        }
    }

    json!({
      "schema_version": "x07.metrics.snapshot@0.1.0",
      "v": 1,
      "taken_at_utc": "1970-01-01T00:00:00Z",
      "service": service,
      "metrics": metrics,
      "labels": {},
    })
}

fn load_app_serve_settings(
    store: &SchemaStore,
    bundle: &LoadedAppBundle,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> (HttpComponentBudgets, usize, bool, String, String) {
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
                "X07WASM_APP_SERVE_PROFILE_LOAD_FAILED",
                Severity::Warning,
                Stage::Parse,
                format!(
                    "failed to load app profile {:?}: {err:#}",
                    bundle.doc.profile_id
                ),
            ));
            return (
                HttpComponentBudgets {
                    max_request_bytes: 1024 * 1024,
                    max_response_bytes: 1024 * 1024,
                    max_wall_ms: 2_000,
                },
                16,
                false,
                "/api".to_string(),
                "127.0.0.1:0".to_string(),
            );
        }
    };

    meta.inputs.push(loaded.digest.clone());
    if let Some(d) = loaded.index_digest.as_ref() {
        meta.inputs.push(d.clone());
    }

    let max_http = usize::try_from(loaded.doc.budgets.max_http_body_bytes).unwrap_or(1024 * 1024);
    let max_wall_ms = loaded.doc.budgets.max_request_wall_ms;
    let max_concurrency = usize::try_from(loaded.doc.budgets.max_concurrency).unwrap_or(16);
    let strict_wasm_mime = loaded.doc.devserver.strict_wasm_mime;
    let api_prefix = loaded.doc.routing.api_prefix.clone();
    let addr = loaded.doc.devserver.addr.clone();

    (
        HttpComponentBudgets {
            max_request_bytes: max_http,
            max_response_bytes: max_http,
            max_wall_ms: max_wall_ms.max(1),
        },
        max_concurrency.max(1),
        strict_wasm_mime,
        api_prefix,
        addr,
    )
}

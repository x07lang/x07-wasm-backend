use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{MachineArgs, Scope, WebUiServeArgs, WebUiServeMode};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;

pub fn cmd_web_ui_serve(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WebUiServeArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = true;
    meta.nondeterminism.uses_os_time = false;

    let incident_dir = args.incidents_dir.display().to_string();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    if !args.dir.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_SERVE_DIR_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("dir not found: {}", args.dir.display()),
        ));
        return emit_serve_report(
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
            None,
            Some(incident_dir.clone()),
        );
    }

    let addr: SocketAddr = match args.addr.parse() {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_SERVE_ADDR_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("invalid addr {:?}: {err}", args.addr),
            ));
            return emit_serve_report(
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
                None,
                Some(incident_dir.clone()),
            );
        }
    };

    let listener = match TcpListener::bind(addr) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_SERVE_BIND_FAILED",
                Severity::Error,
                Stage::Run,
                format!("bind failed: {err}"),
            ));
            return emit_serve_report(
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
                None,
                Some(incident_dir.clone()),
            );
        }
    };
    let bound = listener.local_addr().ok().map(|a| a.to_string());

    match args.mode {
        WebUiServeMode::Listen => {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        let _ = handle_one(&args.dir, stream);
                    }
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_WEB_UI_SERVE_ACCEPT_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("accept failed: {err}"),
                        ));
                        break;
                    }
                }
            }
            emit_serve_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args,
                bound,
                None,
                None,
                Some(incident_dir.clone()),
            )
        }
        WebUiServeMode::Smoke => {
            // In smoke mode, serve exactly one request in a background thread, and request /app.wasm.
            let dir = args.dir.clone();
            let bound_addr = listener.local_addr().context("local_addr")?;
            let handle = std::thread::spawn(move || {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = handle_one(&dir, stream);
                }
            });

            let (wasm_mime, wasm_mime_ok) = smoke_check_wasm_mime(bound_addr, args.strict_mime)
                .unwrap_or_else(|_| ("".to_string(), false));

            let _ = handle.join();

            if args.strict_mime && !wasm_mime_ok {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_SERVE_WASM_MIME_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!("expected application/wasm; got: {:?}", wasm_mime),
                ));
            }

            emit_serve_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args,
                Some(bound_addr.to_string()),
                Some(wasm_mime),
                Some(wasm_mime_ok),
                Some(incident_dir.clone()),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_serve_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    args: &WebUiServeArgs,
    bound_addr: Option<String>,
    wasm_mime: Option<String>,
    wasm_mime_ok: Option<bool>,
    incident_dir: Option<String>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.web_ui.serve.report@0.1.0",
      "command": "x07-wasm.web-ui.serve",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": match args.mode { WebUiServeMode::Listen => "listen", WebUiServeMode::Smoke => "smoke" },
        "dir": args.dir.display().to_string(),
        "bound_addr": bound_addr,
        "wasm_mime": wasm_mime,
        "wasm_mime_ok": wasm_mime_ok,
        "incident_dir": incident_dir,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn smoke_check_wasm_mime(addr: SocketAddr, strict: bool) -> Result<(String, bool)> {
    let mut stream = TcpStream::connect(addr).context("connect")?;
    let req = b"GET /app.wasm HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(req).context("write request")?;
    stream.flush().ok();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).context("read response")?;

    let text = String::from_utf8_lossy(&buf);
    let mut mime = String::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("content-type:") {
            mime = line
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
            break;
        }
        if line.trim().is_empty() {
            break;
        }
    }
    let ok = mime.eq_ignore_ascii_case("application/wasm");
    if strict {
        Ok((mime, ok))
    } else {
        Ok((mime, true))
    }
}

fn handle_one(dir: &Path, mut stream: TcpStream) -> Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let line = req.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
        )?;
        return Ok(());
    }

    let rel = sanitize_path(path);
    let rel = if rel.is_empty() {
        "index.html".to_string()
    } else {
        rel
    };
    let full = dir.join(&rel);
    if !full.exists() || !full.is_file() {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", b"not found")?;
        return Ok(());
    }

    let mime = mime_for_path(&full);
    if method == "HEAD" {
        write_response(&mut stream, 200, &mime, b"")?;
        return Ok(());
    }
    let body = std::fs::read(&full).with_context(|| format!("read: {}", full.display()))?;
    write_response(&mut stream, 200, &mime, &body)?;
    Ok(())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut header = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n");
    header.push_str("X-Content-Type-Options: nosniff\r\n");
    if content_type.starts_with("text/html") {
        header.push_str("Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; connect-src 'self' https: http:; img-src 'self' data:; style-src 'self' 'unsafe-inline'\r\n");
    }
    header.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream
        .write_all(header.as_bytes())
        .context("write header")?;
    stream.write_all(body).context("write body")?;
    Ok(())
}

fn mime_for_path(path: &Path) -> String {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "wasm" => "application/wasm".to_string(),
        "html" => "text/html; charset=utf-8".to_string(),
        "js" | "mjs" => "text/javascript; charset=utf-8".to_string(),
        "json" => "application/json; charset=utf-8".to_string(),
        "css" => "text/css; charset=utf-8".to_string(),
        _ => "application/octet-stream".to_string(),
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

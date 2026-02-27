use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{AppPackArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_app_pack(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppPackArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let bundle_digest = match util::file_digest(&args.bundle_manifest) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to digest bundle manifest {}: {err:#}",
                    args.bundle_manifest.display()
                ),
            ));
            report::meta::FileDigest {
                path: args.bundle_manifest.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let bundle_dir = args
        .bundle_manifest
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let bundle_bytes = match std::fs::read(&args.bundle_manifest) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read bundle manifest {}: {err}",
                    args.bundle_manifest.display()
                ),
            ));
            Vec::new()
        }
    };

    let bundle_doc_json: Value = match serde_json::from_slice(&bundle_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("bundle manifest is not JSON: {err}"),
            ));
            Value::Null
        }
    };

    if bundle_doc_json != Value::Null {
        let diags = store
            .validate(
                "https://x07.io/spec/x07-app.bundle.schema.json",
                &bundle_doc_json,
            )
            .unwrap_or_default();
        if !diags.is_empty() {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                "bundle manifest schema invalid".to_string(),
            ));
            diagnostics.extend(diags);
        }
    }

    let bundle_doc: Option<crate::app::bundle::AppBundleDoc> =
        serde_json::from_value(bundle_doc_json.clone()).ok();

    if diagnostics.iter().any(|d| d.severity == Severity::Error) || bundle_doc.is_none() {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            bundle_digest,
            0,
            0,
            None,
        );
    }

    let bundle_doc = bundle_doc.unwrap();

    if let Err(err) = std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create dir: {}", args.out_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_PACK_WRITE_FAILED",
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
            bundle_digest,
            0,
            0,
            None,
        );
    }

    let assets_dir = args.out_dir.join("assets");
    if let Err(err) = std::fs::create_dir_all(&assets_dir)
        .with_context(|| format!("create dir: {}", assets_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_PACK_WRITE_FAILED",
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
            bundle_digest,
            0,
            0,
            None,
        );
    }

    let mut packaged_files: Vec<report::meta::FileDigest> = Vec::new();

    // Copy the bundle manifest itself into the pack.
    let packaged_bundle = match write_content_addressed_file(
        &assets_dir,
        &args.bundle_manifest,
        &bundle_bytes,
        "json",
    ) {
        Ok(d) => d,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_WRITE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            report::meta::FileDigest {
                path: "assets/unknown.json".to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };
    packaged_files.push(packaged_bundle.clone());
    meta.outputs.push(packaged_bundle.clone());

    // Copy backend component into the pack.
    let backend_src = bundle_dir.join(&bundle_doc.backend.artifact.path);
    let backend_bytes = match std::fs::read(&backend_src) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read backend component {}: {err}",
                    backend_src.display()
                ),
            ));
            Vec::new()
        }
    };
    let backend_ext = file_ext_or(&backend_src, "wasm");
    let packaged_backend =
        match write_content_addressed_file(&assets_dir, &backend_src, &backend_bytes, &backend_ext)
        {
            Ok(d) => d,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PACK_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                report::meta::FileDigest {
                    path: "assets/unknown.wasm".to_string(),
                    sha256: "0".repeat(64),
                    bytes_len: 0,
                }
            }
        };
    packaged_files.push(packaged_backend.clone());
    meta.outputs.push(packaged_backend.clone());

    // Copy all frontend artifacts into the pack.
    let mut assets: Vec<Value> = Vec::new();
    for a in &bundle_doc.frontend.artifacts {
        let src = bundle_dir.join(&a.path);
        let bytes = match std::fs::read(&src) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PACK_BUNDLE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to read frontend artifact {}: {err}", src.display()),
                ));
                continue;
            }
        };
        let ext = file_ext_or(&src, "bin");
        let dig = match write_content_addressed_file(&assets_dir, &src, &bytes, &ext) {
            Ok(d) => d,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PACK_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                continue;
            }
        };
        packaged_files.push(dig.clone());
        meta.outputs.push(dig.clone());

        let serve_path = serve_path_for_frontend_artifact(&bundle_doc.frontend.dir_rel, &a.path);
        let headers = recommended_headers_for_serve_path(&serve_path);

        assets.push(json!({
          "serve_path": serve_path,
          "file": dig,
          "headers": headers,
          "content_type": null,
        }));
    }
    assets.sort_by_key(|a| {
        a.get("serve_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });

    let pack_manifest_path = args.out_dir.join("app.pack.json");
    let pack_doc = json!({
      "schema_version": "x07.app.pack@0.1.0",
      "profile_id": args.profile_id,
      "routing": { "api_prefix": "/api" },
      "bundle_manifest": bundled_digest_obj(&packaged_bundle),
      "frontend": { "index_path": "/index.html" },
      "backend": {
        "adapter": bundle_doc.backend.adapter,
        "component": bundled_digest_obj(&packaged_backend),
      },
      "assets": assets,
    });

    let pack_bytes = report::canon::canonical_pretty_json_bytes(&pack_doc)?;
    if let Err(err) = std::fs::write(&pack_manifest_path, &pack_bytes)
        .with_context(|| format!("write: {}", pack_manifest_path.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_PACK_WRITE_FAILED",
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
            bundle_digest,
            packaged_files.len() as u64,
            packaged_files.iter().map(|d| d.bytes_len).sum(),
            None,
        );
    }

    let pack_digest = util::file_digest(&pack_manifest_path)?;
    meta.outputs.push(pack_digest.clone());

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args,
        bundle_digest,
        packaged_files.len() as u64,
        packaged_files.iter().map(|d| d.bytes_len).sum(),
        Some(pack_digest),
    )
}

fn file_ext_or(path: &Path, fallback: &str) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn serve_path_for_frontend_artifact(frontend_dir_rel: &str, artifact_path: &str) -> String {
    let prefix = format!("{}/", frontend_dir_rel.trim_end_matches('/'));
    if let Some(rest) = artifact_path.strip_prefix(&prefix) {
        format!("/{}", rest)
    } else {
        format!("/{}", artifact_path.trim_start_matches('/'))
    }
}

fn recommended_headers_for_serve_path(serve_path: &str) -> Vec<Value> {
    if serve_path.ends_with(".wasm") {
        return vec![json!({"k":"content-type","v":"application/wasm"})];
    }
    Vec::new()
}

fn bundled_digest_obj(d: &report::meta::FileDigest) -> Value {
    json!({
      "path": d.path,
      "sha256": d.sha256,
      "bytes_len": d.bytes_len,
    })
}

fn write_content_addressed_file(
    assets_dir: &Path,
    _src_path: &Path,
    bytes: &[u8],
    ext: &str,
) -> Result<report::meta::FileDigest> {
    let sha256 = util::sha256_hex(bytes);
    let file_name = format!("{}.{}", sha256, ext);
    let rel_path = Path::new("assets").join(&file_name);
    let dst_path = assets_dir.join(&file_name);
    std::fs::write(&dst_path, bytes).with_context(|| format!("write: {}", dst_path.display()))?;
    Ok(report::meta::FileDigest {
        path: rel_path.to_string_lossy().to_string(),
        sha256,
        bytes_len: bytes.len() as u64,
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
    args: &AppPackArgs,
    bundle_manifest: report::meta::FileDigest,
    assets_count: u64,
    assets_bytes_total: u64,
    pack_manifest: Option<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let pack_manifest = pack_manifest.unwrap_or(report::meta::FileDigest {
        path: args.out_dir.join("app.pack.json").display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.app.pack.report@0.1.0",
      "command": "x07-wasm.app.pack",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "stdout": { "bytes_len": 0 },
        "stderr": { "bytes_len": 0 },
        "stdout_json": {
          "profile_id": args.profile_id,
          "out_dir": args.out_dir.display().to_string(),
          "pack_manifest": pack_manifest,
          "assets_count": assets_count,
          "assets_bytes_total": assets_bytes_total,
        }
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    let _ = bundle_manifest;
    Ok(exit_code)
}

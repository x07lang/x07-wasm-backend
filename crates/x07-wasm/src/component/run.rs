use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::task::LocalSet;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::blob;
use crate::cli::{ComponentRunArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::guest_diag;
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_component_run(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ComponentRunArgs,
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
                "X07WASM_COMPONENT_RUN_COMPONENT_READ_FAILED",
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

    let run_args = match parse_args_json(&args.args_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_ARGS_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            Vec::new()
        }
    };

    let stdin_loaded = load_stdin(&args, &mut meta, &mut diagnostics)?;

    let output_cap: usize = args.max_output_bytes.try_into().unwrap_or(usize::MAX);
    let stdout_pipe = MemoryOutputPipe::new(output_cap);
    let stderr_pipe = MemoryOutputPipe::new(output_cap);

    let mut wasi_builder = WasiCtxBuilder::new();
    if !run_args.is_empty() {
        wasi_builder.args(&run_args);
    }
    let wasi = wasi_builder
        .stdin(MemoryInputPipe::new(stdin_loaded.bytes.clone()))
        .stdout(stdout_pipe.clone())
        .stderr(stderr_pipe.clone())
        .build();

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
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                component_digest,
                &run_args,
                stdin_loaded.blob_ref,
                &[],
                &[],
                json!({ "outcome": "engine_failed" }),
                None,
            );
        }
    };

    let component = match Component::from_file(&engine, &args.component) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_COMPONENT_COMPILE_FAILED",
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
                &run_args,
                stdin_loaded.blob_ref,
                &[],
                &[],
                json!({ "outcome": "component_compile_failed" }),
                None,
            );
        }
    };

    let mut linker: Linker<RunState> = Linker::new(&engine);
    if let Err(err) = wasmtime_wasi::p2::add_to_linker_async(&mut linker) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_COMPONENT_RUN_LINKER_FAILED",
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
            &run_args,
            stdin_loaded.blob_ref,
            &[],
            &[],
            json!({ "outcome": "linker_failed" }),
            None,
        );
    }

    let local = LocalSet::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .context("build tokio runtime")?;

    let run_outcome = rt.block_on(local.run_until(async {
        let mut store = Store::new(
            &engine,
            RunState {
                table: ResourceTable::new(),
                wasi,
            },
        );
        store.data_mut().table.set_max_capacity(1024);

        let command = match wasmtime_wasi::p2::bindings::Command::instantiate_async(
            &mut store, &component, &linker,
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                return RunOutcome::Trap {
                    trap: format!("{err:#}"),
                };
            }
        };

        let run_future = command.wasi_cli_run().call_run(&mut store);
        let res = if let Some(ms) = args.max_wall_ms {
            match tokio::time::timeout(std::time::Duration::from_millis(ms), run_future).await {
                Ok(v) => v,
                Err(_) => return RunOutcome::TimedOut { max_wall_ms: ms },
            }
        } else {
            run_future.await
        };

        match res {
            Ok(Ok(())) => RunOutcome::Ok,
            Ok(Err(())) => RunOutcome::Err,
            Err(err) => RunOutcome::Trap {
                trap: format!("{err:#}"),
            },
        }
    }));

    let run_doc = match run_outcome.clone() {
        RunOutcome::Ok => json!({ "outcome": "ok" }),
        RunOutcome::Err => json!({ "outcome": "err" }),
        RunOutcome::TimedOut { max_wall_ms } => {
            json!({ "outcome": "timeout", "max_wall_ms": max_wall_ms })
        }
        RunOutcome::Trap { trap } => json!({ "outcome": "trap", "trap": trap }),
    };

    let stdout_bytes = stdout_pipe.contents().to_vec();
    let stderr_bytes = stderr_pipe.contents().to_vec();
    let output_at_cap = output_cap != usize::MAX
        && output_cap > 0
        && (stdout_bytes.len() == output_cap || stderr_bytes.len() == output_cap);

    match run_outcome {
        RunOutcome::Ok => {}
        RunOutcome::Err => diagnostics.push(Diagnostic::new(
            "X07WASM_COMPONENT_RUN_EXIT_FAILURE",
            Severity::Error,
            Stage::Run,
            "component run returned Err(())".to_string(),
        )),
        RunOutcome::TimedOut { .. } => diagnostics.push(Diagnostic::new(
            "X07WASM_BUDGET_EXCEEDED_WALL_TIME",
            Severity::Error,
            Stage::Run,
            "component run exceeded wall-time budget".to_string(),
        )),
        RunOutcome::Trap { trap } => {
            if trap.contains("write beyond capacity of MemoryOutputPipe") || output_at_cap {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_BUDGET_EXCEEDED_OUTPUT",
                    Severity::Error,
                    Stage::Run,
                    "stdout/stderr exceeded max_output_bytes budget".to_string(),
                ));
            }
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_TRAP",
                Severity::Error,
                Stage::Run,
                "component trapped during run".to_string(),
            ));
        }
    }

    match guest_diag::extract_guest_diag_from_stderr(&stderr_bytes) {
        Ok(Some(gd)) => {
            let mut d = Diagnostic::new(
                gd.code,
                Severity::Error,
                Stage::Run,
                "guest diagnostic via stderr sentinel".to_string(),
            );
            if let Some(Value::Object(map)) = gd.data_obj {
                for (k, v) in map {
                    d.data.insert(k, v);
                }
            }
            diagnostics.push(d);
        }
        Ok(None) => {}
        Err(err) => diagnostics.push(err.into_diagnostic()),
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let mut incident_dir: Option<PathBuf> = None;
    if !ok {
        meta.nondeterminism.uses_os_time = true;
        match write_incident_bundle(
            &args.incidents_dir,
            &component_digest.sha256,
            &stdin_loaded.bytes,
            &stdin_loaded.blob_ref,
            &run_args,
            &stdout_bytes,
            &stderr_bytes,
            &args.component,
        ) {
            Ok(dir) => incident_dir = Some(dir),
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_INCIDENT_BUNDLE_WRITE_FAILED",
                Severity::Warning,
                Stage::Run,
                format!("{err:#}"),
            )),
        }
    }

    if let Some(path) = args.stdout_out.as_ref() {
        if let Err(err) = write_output_file(path, &stdout_bytes) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_STDOUT_WRITE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to write stdout to {}: {err:#}", path.display()),
            ));
        } else if let Ok(d) = util::file_digest(path) {
            meta.outputs.push(d);
        }
    }
    if let Some(path) = args.stderr_out.as_ref() {
        if let Err(err) = write_output_file(path, &stderr_bytes) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_STDERR_WRITE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to write stderr to {}: {err:#}", path.display()),
            ));
        } else if let Ok(d) = util::file_digest(path) {
            meta.outputs.push(d);
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
        component_digest,
        &run_args,
        stdin_loaded.blob_ref,
        &stdout_bytes,
        &stderr_bytes,
        run_doc,
        incident_dir,
    )
}

struct RunState {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for RunState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

#[derive(Debug, Clone)]
enum RunOutcome {
    Ok,
    Err,
    TimedOut { max_wall_ms: u64 },
    Trap { trap: String },
}

fn parse_args_json(args_json: &str) -> Result<Vec<String>> {
    let v: Value = serde_json::from_str(args_json).context("parse --args-json")?;
    let Some(arr) = v.as_array() else {
        anyhow::bail!("--args-json must be a JSON array");
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(s) = item.as_str() else {
            anyhow::bail!("--args-json items must be strings");
        };
        out.push(s.to_string());
    }
    Ok(out)
}

fn load_stdin(
    args: &ComponentRunArgs,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<blob::LoadedBytes> {
    if let Some(path) = args.stdin.as_ref() {
        return blob::load_file(path, meta).map_err(|err| {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_STDIN_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read stdin {}: {err:#}", path.display()),
            ));
            anyhow::anyhow!("stdin read failed")
        });
    }
    if let Some(b64) = args.stdin_b64.as_deref() {
        return blob::load_base64(b64).map_err(|err| {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_RUN_STDIN_BASE64_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to decode --stdin-b64: {err:#}"),
            ));
            anyhow::anyhow!("stdin base64 invalid")
        });
    }
    Ok(blob::LoadedBytes {
        bytes: Vec::new(),
        blob_ref: json!({ "bytes_len": 0, "sha256": util::sha256_hex(&[]) }),
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
    component: report::meta::FileDigest,
    args: &[String],
    stdin: Value,
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    run: Value,
    incident_dir: Option<PathBuf>,
) -> Result<u8> {
    let stdout_ref = json!({
      "bytes_len": stdout_bytes.len() as u64,
      "sha256": util::sha256_hex(stdout_bytes),
    });
    let stderr_ref = json!({
      "bytes_len": stderr_bytes.len() as u64,
      "sha256": util::sha256_hex(stderr_bytes),
    });

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.component.run.report@0.1.0",
      "command": "x07-wasm.component.run",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "component": component,
        "args": args,
        "stdin": stdin,
        "stdout": stdout_ref,
        "stderr": stderr_ref,
        "run": run,
        "incident_dir": incident_dir.as_ref().map(|p| p.display().to_string()),
      }
    });

    if let Some(dir) = incident_dir.as_ref() {
        let bytes = report::canon::canonical_json_bytes(&report_doc)?;
        let _ = std::fs::write(dir.join("component.run.report.json"), bytes);
    }

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[allow(clippy::too_many_arguments)]
fn write_incident_bundle(
    incidents_dir: &Path,
    component_sha256: &str,
    stdin_bytes: &[u8],
    stdin_ref: &Value,
    args: &[String],
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    component_path: &Path,
) -> Result<PathBuf> {
    let stdin_sha = util::sha256_hex(stdin_bytes);
    let run_id = util::sha256_hex(format!("{component_sha256}:{stdin_sha}").as_bytes());
    let run_id = run_id[..32].to_string();
    let date = crate::wasm::incident::utc_date_yyyy_mm_dd();
    let dir = incidents_dir.join(date).join(run_id);
    std::fs::create_dir_all(&dir).with_context(|| format!("create dir: {}", dir.display()))?;

    let manifest = json!({
      "schema_version": "x07.wasm.incident.manifest@0.1.0",
      "kind": "component-run",
      "component": { "path": component_path.display().to_string(), "sha256": component_sha256 },
      "stdin": stdin_ref,
      "args": args,
    });
    let manifest_bytes = report::canon::canonical_json_bytes(&manifest)?;
    std::fs::write(dir.join("incident.manifest.json"), &manifest_bytes)
        .with_context(|| format!("write: {}", dir.join("incident.manifest.json").display()))?;

    std::fs::write(dir.join("stdin.bin"), stdin_bytes)
        .with_context(|| format!("write: {}", dir.join("stdin.bin").display()))?;

    if !stdout_bytes.is_empty() {
        std::fs::write(dir.join("stdout.bin"), stdout_bytes)
            .with_context(|| format!("write: {}", dir.join("stdout.bin").display()))?;
    }
    if !stderr_bytes.is_empty() {
        std::fs::write(dir.join("stderr.bin"), stderr_bytes)
            .with_context(|| format!("write: {}", dir.join("stderr.bin").display()))?;
    }

    let component_manifest_path = component_path.with_extension("wasm.manifest.json");
    if component_manifest_path.is_file() {
        if let Ok(bytes) = std::fs::read(&component_manifest_path) {
            let _ = std::fs::write(dir.join("component.manifest.json"), bytes);
        }
    }

    Ok(dir)
}

fn write_output_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("write: {}", path.display()))?;
    Ok(())
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
          "schema_version": "x07.wasm.component.run.report@0.1.0",
          "command": "x07-wasm.component.run",
          "ok": false,
          "exit_code": 1,
          "diagnostics": [],
          "meta": meta,
          "result": {
            "component": { "path": "a.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "args": [],
            "stdin": { "bytes_len": 0, "sha256": "0".repeat(64) },
            "stdout": { "bytes_len": 0, "sha256": "0".repeat(64) },
            "stderr": { "bytes_len": 0, "sha256": "0".repeat(64) },
            "run": { "outcome": "ok" },
            "incident_dir": null
          }
        });
        let diags = store
            .validate(
                "https://x07.io/spec/x07-wasm.component.run.report.schema.json",
                &doc,
            )
            .unwrap();
        assert!(diags.is_empty(), "schema diags: {diags:?}");
    }
}

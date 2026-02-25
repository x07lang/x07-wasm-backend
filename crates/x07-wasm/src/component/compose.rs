use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{ComponentComposeAdapterKind, ComponentComposeArgs, MachineArgs, Scope};
use crate::cmdutil;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::wit;

pub fn cmd_component_compose(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ComponentComposeArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let adapter_kind = args.adapter;
    let solve_path = args.solve;
    let adapter_component_path = args
        .adapter_component
        .unwrap_or_else(|| default_adapter_component_path(adapter_kind));
    let out_path = args.out;
    let artifact_out_path = args
        .artifact_out
        .unwrap_or_else(|| default_artifact_out_path(&out_path));
    let targets_check = args.targets_check;

    let solve_digest = file_digest_or_zero(&solve_path, &mut meta, &mut diagnostics);
    let adapter_digest = file_digest_or_zero(&adapter_component_path, &mut meta, &mut diagnostics);

    if let Some(parent) = out_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))
        {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_COMPONENT_COMPOSE_OUTDIR_CREATE_FAILED",
                Stage::Run,
                format!("failed to create out dir: {}", parent.display()),
                &err,
            ));
        }
    }

    let mut out_digest = report::meta::FileDigest {
        path: out_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    let mut targets_check_out = json!({
      "attempted": false,
      "ok": true,
      "exit_code": 0,
      "stdout": "",
      "stderr": ""
    });

    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        let wac_args = vec![
            "plug".to_string(),
            "--plug".to_string(),
            solve_path.display().to_string(),
            adapter_component_path.display().to_string(),
            "-o".to_string(),
            out_path.display().to_string(),
        ];

        let wac_out = match cmdutil::run_cmd_capture("wac", &wac_args) {
            Ok(v) => Some(v),
            Err(err) => {
                diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                    "X07WASM_WAC_PLUG_SPAWN_FAILED",
                    Stage::Run,
                    "wac plug",
                    &err,
                ));
                None
            }
        };

        if let Some(out) = wac_out.as_ref() {
            if !out.status.success() {
                diagnostics.push(cmdutil::diag_cmd_failed(
                    "X07WASM_WAC_PLUG_FAILED",
                    Stage::Run,
                    "wac plug",
                    out.code,
                    &out.stderr,
                ));
            }
        }

        if wac_out.as_ref().is_some_and(|o| o.status.success()) {
            match util::file_digest(&out_path) {
                Ok(d) => {
                    out_digest = d.clone();
                    meta.outputs.push(d);
                }
                Err(err) => diagnostics.push(Diagnostic::new(
                    "X07WASM_COMPONENT_COMPOSE_OUTPUT_DIGEST_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to digest output {}: {err:#}", out_path.display()),
                )),
            }

            let (wit_package, wit_world, kind) = match adapter_kind {
                ComponentComposeAdapterKind::Http => ("wasi:http@0.2.8", "proxy", "http"),
                ComponentComposeAdapterKind::Cli => ("wasi:cli@0.2.8", "command", "cli"),
            };

            let artifact_doc = json!({
              "schema_version": "x07.wasm.component.artifact@0.1.0",
              "artifact_id": format!("{kind}-{}", &out_digest.sha256[..16]),
              "kind": kind,
              "component": out_digest,
              "wit": { "package": wit_package, "world": wit_world },
              "profiles": {},
              "toolchain": {
                "x07_wasm": env!("CARGO_PKG_VERSION"),
              }
            });

            let diags = store.validate(
                "https://x07.io/spec/x07-wasm.component.artifact.schema.json",
                &artifact_doc,
            )?;
            if diags.iter().any(|d| d.severity == Severity::Error) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_COMPONENT_ARTIFACT_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "internal error: component artifact failed schema validation: {diags:?}"
                    ),
                ));
            } else {
                let bytes = report::canon::canonical_json_bytes(&artifact_doc)?;
                match std::fs::write(&artifact_out_path, &bytes)
                    .with_context(|| format!("write: {}", artifact_out_path.display()))
                {
                    Ok(()) => match util::file_digest(&artifact_out_path) {
                        Ok(d) => meta.outputs.push(d),
                        Err(err) => diagnostics.push(Diagnostic::new(
                            "X07WASM_COMPONENT_ARTIFACT_DIGEST_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!(
                                "failed to digest artifact {}: {err:#}",
                                artifact_out_path.display()
                            ),
                        )),
                    },
                    Err(err) => diagnostics.push(cmdutil::diag_io_failed(
                        "X07WASM_COMPONENT_ARTIFACT_WRITE_FAILED",
                        Stage::Run,
                        format!("failed to write artifact: {}", artifact_out_path.display()),
                        &err,
                    )),
                }
            }

            if targets_check {
                let (wit_path, world) = match adapter_kind {
                    ComponentComposeAdapterKind::Http => (
                        Path::new("wit/deps/wasi/http/0.2.8/proxy.wit").to_path_buf(),
                        "proxy".to_string(),
                    ),
                    ComponentComposeAdapterKind::Cli => (
                        Path::new("wit/deps/wasi/cli/0.2.8/command.wit").to_path_buf(),
                        "command".to_string(),
                    ),
                };

                let mut wit_arg = wit_path.display().to_string();
                match wit::bundle::bundle_for_wit_path(
                    &store,
                    Path::new("arch/wit/index.x07wit.json"),
                    &wit_path,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(Some(bundle)) => wit_arg = bundle.dir.display().to_string(),
                    Ok(None) => {}
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_WIT_BUNDLE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    )),
                };

                let wac_args = vec![
                    "targets".to_string(),
                    "--wit".to_string(),
                    wit_arg,
                    "--world".to_string(),
                    world.clone(),
                    out_path.display().to_string(),
                ];
                let out = match cmdutil::run_cmd_capture("wac", &wac_args) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                            "X07WASM_WAC_TARGETS_SPAWN_FAILED",
                            Stage::Run,
                            "wac targets",
                            &err,
                        ));
                        None
                    }
                };

                if let Some(out) = out {
                    let ok = out.status.success();
                    targets_check_out = json!({
                      "attempted": true,
                      "ok": ok,
                      "exit_code": u8::try_from(out.code).unwrap_or(1),
                      "stdout": String::from_utf8_lossy(&out.stdout).to_string(),
                      "stderr": String::from_utf8_lossy(&out.stderr).to_string(),
                    });
                    if !ok {
                        diagnostics.push(cmdutil::diag_cmd_failed(
                            "X07WASM_WAC_TARGETS_FAILED",
                            Stage::Run,
                            "wac targets",
                            out.code,
                            &out.stderr,
                        ));
                    }
                } else {
                    targets_check_out = json!({
                      "attempted": true,
                      "ok": false,
                      "exit_code": 1,
                      "stdout": "",
                      "stderr": ""
                    });
                }
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let (kind, wit_package, wit_world) = match adapter_kind {
        ComponentComposeAdapterKind::Http => ("http", "wasi:http@0.2.8", "proxy"),
        ComponentComposeAdapterKind::Cli => ("cli", "wasi:cli@0.2.8", "command"),
    };

    let report_doc = json!({
      "schema_version": "x07.wasm.component.compose.report@0.1.0",
      "command": "x07-wasm.component.compose",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "adapter_kind": kind,
        "solve_component": solve_digest,
        "adapter_component": adapter_digest,
        "out_component": out_digest,
        "artifact": {
          "schema_version": "x07.wasm.component.artifact@0.1.0",
          "artifact_id": format!("{kind}-{}", &out_digest.sha256[..16]),
          "kind": kind,
          "component": out_digest,
          "wit": { "package": wit_package, "world": wit_world },
          "profiles": {},
          "toolchain": { "x07_wasm": env!("CARGO_PKG_VERSION") }
        },
        "targets_check": targets_check_out
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn file_digest_or_zero(
    path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> report::meta::FileDigest {
    match util::file_digest(path) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_COMPOSE_INPUT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read {}: {err:#}", path.display()),
            ));
            report::meta::FileDigest {
                path: path.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    }
}

fn default_adapter_component_path(kind: ComponentComposeAdapterKind) -> PathBuf {
    match kind {
        ComponentComposeAdapterKind::Http => {
            Path::new("target/x07-wasm/component/http-adapter.component.wasm").to_path_buf()
        }
        ComponentComposeAdapterKind::Cli => {
            Path::new("target/x07-wasm/component/cli-adapter.component.wasm").to_path_buf()
        }
    }
}

fn default_artifact_out_path(out: &Path) -> PathBuf {
    if out.extension().and_then(|s| s.to_str()) == Some("wasm") {
        return out.with_extension("wasm.manifest.json");
    }
    Path::new(&format!("{}.manifest.json", out.to_string_lossy())).to_path_buf()
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
          "schema_version": "x07.wasm.component.compose.report@0.1.0",
          "command": "x07-wasm.component.compose",
          "ok": true,
          "exit_code": 0,
          "diagnostics": [],
          "meta": meta,
          "result": {
            "adapter_kind": "http",
            "solve_component": { "path": "a.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "adapter_component": { "path": "b.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "out_component": { "path": "c.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "artifact": {
              "schema_version": "x07.wasm.component.artifact@0.1.0",
              "artifact_id": "http-0000000000000000",
              "kind": "http",
              "component": { "path": "c.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
              "wit": { "package": "wasi:http@0.2.8", "world": "proxy" },
              "profiles": {},
              "toolchain": { "x07_wasm": "0.1.0" }
            },
            "targets_check": {
              "attempted": false,
              "ok": true,
              "exit_code": 0,
              "stdout": "",
              "stderr": ""
            }
          }
        });
        let diags = store
            .validate(
                "https://x07.io/spec/x07-wasm.component.compose.report.schema.json",
                &doc,
            )
            .unwrap();
        assert!(diags.is_empty(), "schema diags: {diags:?}");
    }
}

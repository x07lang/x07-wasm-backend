use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::cli::{ComponentTargetsArgs, MachineArgs, Scope};
use crate::cmdutil;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::wit;

pub fn cmd_component_targets(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ComponentTargetsArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
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
                "X07WASM_COMPONENT_TARGETS_COMPONENT_READ_FAILED",
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

    let wit_digest = match util::file_digest(&args.wit) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_TARGETS_WIT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read wit {}: {err:#}", args.wit.display()),
            ));
            report::meta::FileDigest {
                path: args.wit.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let mut wac_exit_code: u8 = 1;
    let mut stdout = String::new();
    let mut stderr = String::new();

    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        let mut wit_arg = args.wit.display().to_string();
        match wit::bundle::bundle_for_wit_path(
            &store,
            Path::new("arch/wit/index.x07wit.json"),
            &args.wit,
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
            args.world.clone(),
            args.component.display().to_string(),
        ];

        let out = match cmdutil::run_cmd_capture("wac", &wac_args) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                    "X07WASM_WAC_TARGETS_SPAWN_FAILED",
                    Stage::Run,
                    "wac targets",
                    &err,
                ));
                wac_exit_code = 1;
                stdout.clear();
                stderr.clear();
                let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
                let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
                let report_doc = json!({
                  "schema_version": "x07.wasm.component.targets.report@0.1.0",
                  "command": "x07-wasm.component.targets",
                  "ok": ok,
                  "exit_code": exit_code,
                  "diagnostics": diagnostics,
                  "meta": meta,
                  "result": {
                    "component": component_digest,
                    "wit": wit_digest,
                    "world": args.world,
                    "wac_exit_code": wac_exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                  }
                });
                store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
                return Ok(exit_code);
            }
        };

        wac_exit_code = u8::try_from(out.code).unwrap_or(1);
        stdout = String::from_utf8_lossy(&out.stdout).to_string();
        stderr = String::from_utf8_lossy(&out.stderr).to_string();

        if !out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_WAC_TARGETS_FAILED",
                Stage::Run,
                "wac targets",
                out.code,
                &out.stderr,
            ));
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.component.targets.report@0.1.0",
      "command": "x07-wasm.component.targets",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "component": component_digest,
        "wit": wit_digest,
        "world": args.world,
        "wac_exit_code": wac_exit_code,
        "stdout": stdout,
        "stderr": stderr,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_schema_accepts_placeholder() {
        let store = SchemaStore::new().unwrap();
        let raw_argv: Vec<OsString> = Vec::new();
        let doc = json!({
          "schema_version": "x07.wasm.component.targets.report@0.1.0",
          "command": "x07-wasm.component.targets",
          "ok": false,
          "exit_code": 1,
          "diagnostics": [],
          "meta": report::meta::tool_meta(&raw_argv, std::time::Instant::now()),
          "result": {
            "component": { "path": "a.wasm", "sha256": "0".repeat(64), "bytes_len": 0 },
            "wit": { "path": "a.wit", "sha256": "0".repeat(64), "bytes_len": 0 },
            "world": "proxy",
            "wac_exit_code": 1,
            "stdout": "",
            "stderr": ""
          }
        });
        let diags = store
            .validate(
                "https://x07.io/spec/x07-wasm.component.targets.report.schema.json",
                &doc,
            )
            .unwrap();
        assert!(diags.is_empty(), "schema diags: {diags:?}");
    }
}

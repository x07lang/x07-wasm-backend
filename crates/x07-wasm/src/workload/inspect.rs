use std::ffi::OsString;
use std::fs;

use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::workload::cli::WorkloadInspectArgs;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_workload_inspect(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WorkloadInspectArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_os_time = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_process = false;

    let mut diagnostics = Vec::new();
    if let Ok(digest) = crate::util::file_digest(&args.pack_manifest) {
        meta.inputs.push(digest);
    }

    let mut stdout_json = json!({
        "pack_manifest": args.pack_manifest.display().to_string(),
        "view": args.view,
    });

    match surface::load_source_from_pack(&args.pack_manifest) {
        Ok(source) => match surface::build_artifacts(&source, &args.view, None) {
            Ok(artifacts) => {
                let runtime_pack = load_runtime_pack(&args.pack_manifest).unwrap_or(Value::Null);
                stdout_json = json!({
                    "pack_manifest_path": args.pack_manifest.display().to_string(),
                    "view": args.view,
                    "pack_manifest": artifacts.pack_manifest,
                    "runtime_pack": runtime_pack,
                    "workload": artifacts.workload_doc,
                    "bindings": artifacts.binding_doc,
                    "topology": artifacts
                        .topology_docs
                        .into_iter()
                        .map(|(_, doc)| doc)
                        .collect::<Vec<_>>(),
                });
            }
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_WORKLOAD_INSPECT_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to inspect workload pack: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_WORKLOAD_INSPECT_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("failed to load workload pack sources: {err}"),
        )),
    }

    surface::emit_report(
        raw_argv,
        scope,
        machine,
        started,
        "x07-wasm.workload.inspect",
        meta,
        diagnostics,
        stdout_json,
        None,
        CopyStats::default(),
        Vec::new(),
    )
}

fn load_runtime_pack(pack_manifest: &std::path::Path) -> Result<Value> {
    let Some(pack_dir) = pack_manifest.parent() else {
        return Ok(Value::Null);
    };
    let path = pack_dir.join("x07.workload.pack.json");
    if !path.is_file() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

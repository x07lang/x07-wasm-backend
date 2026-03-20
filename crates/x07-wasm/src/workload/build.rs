use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::workload::cli::WorkloadBuildArgs;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_workload_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WorkloadBuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_os_time = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_process = false;

    let mut diagnostics = Vec::new();
    push_input_digest(&mut meta, &args.project);
    push_input_digest(&mut meta, &args.manifest);

    let mut stdout_json = json!({
        "mode": "build",
        "out_dir": args.out_dir.display().to_string(),
    });

    match surface::load_source(&args.project, &args.manifest) {
        Ok(source) => match surface::write_build_outputs(&source, &args.out_dir, None) {
            Ok((outputs, artifacts)) => {
                meta.outputs.extend(outputs);
                stdout_json = json!({
                    "mode": "build",
                    "out_dir": args.out_dir.display().to_string(),
                    "pack_manifest": artifacts.pack_manifest,
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
                "X07WASM_WORKLOAD_BUILD_WRITE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to write workload build outputs: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_WORKLOAD_BUILD_INPUT_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("failed to load service source: {err}"),
        )),
    }

    surface::emit_report(
        raw_argv,
        scope,
        machine,
        started,
        "x07-wasm.workload.build",
        meta,
        surface::SurfaceReportPayload {
            diagnostics,
            stdout_json,
            output_path: Some(args.out_dir),
            copy_stats: CopyStats::default(),
            checked_schema_ids: Vec::new(),
        },
    )
}

fn push_input_digest(meta: &mut report::meta::ReportMeta, path: &Path) {
    if let Ok(digest) = crate::util::file_digest(path) {
        meta.inputs.push(digest);
    }
}

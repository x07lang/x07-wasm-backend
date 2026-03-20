use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::binding::cli::BindingResolveArgs;
use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_binding_resolve(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: BindingResolveArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_os_time = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_process = false;

    let mut diagnostics = Vec::new();
    push_input_digest(&mut meta, &args.project);
    push_input_digest(&mut meta, &args.manifest);
    if let Some(pack_manifest) = &args.pack_manifest {
        push_input_digest(&mut meta, pack_manifest);
    }

    let source = match &args.pack_manifest {
        Some(pack_manifest) => surface::load_source_from_pack(pack_manifest),
        None => surface::load_source(&args.project, &args.manifest),
    };

    let mut stdout_json = json!({});
    match source {
        Ok(source) => match surface::binding_requirements_doc(&source) {
            Ok(doc) => stdout_json = doc,
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_BINDING_RESOLVE_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to resolve binding requirements: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_BINDING_RESOLVE_INVALID",
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
        "x07-wasm.binding.resolve",
        meta,
        surface::SurfaceReportPayload {
            diagnostics,
            stdout_json,
            output_path: None,
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

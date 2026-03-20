use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::topology::cli::TopologyPreviewArgs;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_topology_preview(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: TopologyPreviewArgs,
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

    let mut stdout_json = json!({
        "profile": args.profile,
    });
    match source {
        Ok(source) => match surface::topology_docs(&source, args.profile.as_deref()) {
            Ok(docs) => {
                stdout_json = if docs.len() == 1 {
                    docs.into_iter()
                        .next()
                        .map(|(_, doc)| doc)
                        .unwrap_or(Value::Null)
                } else {
                    Value::Array(docs.into_iter().map(|(_, doc)| doc).collect())
                };
            }
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_TOPOLOGY_PREVIEW_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to build topology preview: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_TOPOLOGY_PREVIEW_INVALID",
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
        "x07-wasm.topology.preview",
        meta,
        diagnostics,
        stdout_json,
        None,
        CopyStats::default(),
        Vec::new(),
    )
}

fn push_input_digest(meta: &mut report::meta::ReportMeta, path: &Path) {
    if let Ok(digest) = crate::util::file_digest(path) {
        meta.inputs.push(digest);
    }
}

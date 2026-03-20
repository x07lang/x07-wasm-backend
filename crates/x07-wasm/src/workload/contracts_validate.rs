use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::workload::cli::WorkloadContractsValidateArgs;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_workload_contracts_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WorkloadContractsValidateArgs,
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

    let mut stdout_json = json!({
        "profile": args.profile,
    });
    let mut checked_schema_ids = Vec::new();

    let source = match &args.pack_manifest {
        Some(pack_manifest) => surface::load_source_from_pack(pack_manifest),
        None => surface::load_source(&args.project, &args.manifest),
    };

    match source {
        Ok(source) => match surface::build_artifacts(&source, "full", args.profile.as_deref()) {
            Ok(artifacts) => match surface::resolve_schema_dir(args.schema_dir.as_deref()) {
                Ok(schema_dir) => {
                    let mut docs = vec![
                        ("lp.workload.pack.manifest.schema.json", &artifacts.pack_manifest),
                        ("lp.workload.describe.result.schema.json", &artifacts.workload_doc),
                        ("lp.binding.requirements.result.schema.json", &artifacts.binding_doc),
                    ];
                    for (_, doc) in &artifacts.topology_docs {
                        docs.push(("lp.topology.preview.result.schema.json", doc));
                    }
                    checked_schema_ids = vec![
                        "lp.workload.pack.manifest@0.1.0".to_string(),
                        "lp.workload.describe.result@0.1.0".to_string(),
                        "lp.binding.requirements.result@0.1.0".to_string(),
                    ];
                    checked_schema_ids.extend(
                        artifacts
                            .topology_docs
                            .iter()
                            .map(|_| "lp.topology.preview.result@0.1.0".to_string()),
                    );
                    match surface::validate_contract_docs(&schema_dir, &docs) {
                        Ok(mut schema_diags) => {
                            diagnostics.append(&mut schema_diags);
                            stdout_json = json!({
                                "schema_dir": schema_dir.display().to_string(),
                                "documents_checked": docs.len(),
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
                            "X07WASM_WORKLOAD_CONTRACTS_VALIDATE_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("failed to validate workload contracts: {err}"),
                        )),
                    }
                }
                Err(err) => diagnostics.push(Diagnostic::new(
                    "X07WASM_WORKLOAD_CONTRACTS_SCHEMA_DIR_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to resolve contract schema directory: {err}"),
                )),
            },
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_WORKLOAD_CONTRACTS_INPUT_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to build workload documents: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_WORKLOAD_CONTRACTS_INPUT_INVALID",
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
        "x07-wasm.workload.contracts-validate",
        meta,
        diagnostics,
        stdout_json,
        None,
        CopyStats::default(),
        checked_schema_ids,
    )
}

fn push_input_digest(meta: &mut report::meta::ReportMeta, path: &Path) {
    if let Ok(digest) = crate::util::file_digest(path) {
        meta.inputs.push(digest);
    }
}

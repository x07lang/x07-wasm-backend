use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::meta::FileDigest;
use crate::workload::cli::WorkloadPackArgs;
use crate::workload::surface::{self, CopyStats};

pub fn cmd_workload_pack(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WorkloadPackArgs,
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
        "mode": "pack",
        "out_dir": args.out_dir.display().to_string(),
    });
    let mut copy_stats = CopyStats::default();

    match surface::load_source(&args.project, &args.manifest) {
        Ok(source) => match surface::write_build_outputs(&source, &args.out_dir, None) {
            Ok((outputs, artifacts)) => {
                meta.outputs.extend(outputs);
                match surface::write_pack_sources(&source, &args.out_dir) {
                    Ok(stats) => {
                        copy_stats = stats;
                        match build_runtime_pack_doc(
                            &args.out_dir,
                            &artifacts.workload_doc,
                            args.runtime_image.as_deref(),
                            args.container_port,
                        )
                        .and_then(|doc| {
                            let digest = surface::write_json_doc(
                                &args.out_dir.join("x07.workload.pack.json"),
                                &doc,
                            )?;
                            Ok((doc, digest))
                        }) {
                            Ok((runtime_pack, digest)) => {
                                meta.outputs.push(digest);
                                stdout_json = json!({
                                    "mode": "pack",
                                    "out_dir": args.out_dir.display().to_string(),
                                    "pack_manifest": artifacts.pack_manifest,
                                    "runtime_pack": runtime_pack,
                                    "workload": artifacts.workload_doc,
                                    "bindings": artifacts.binding_doc,
                                    "topology": artifacts
                                        .topology_docs
                                        .into_iter()
                                        .map(|(_, doc)| doc)
                                        .collect::<Vec<_>>(),
                                    "sources_snapshot": {
                                        "root": args.out_dir.join("sources").display().to_string(),
                                        "files_copied": copy_stats.files_copied,
                                        "bytes_copied": copy_stats.bytes_copied,
                                    }
                                });
                            }
                            Err(err) => diagnostics.push(Diagnostic::new(
                                "X07WASM_WORKLOAD_PACK_WRITE_FAILED",
                                Severity::Error,
                                Stage::Run,
                                format!("failed to write workload runtime pack: {err}"),
                            )),
                        }
                    }
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_WORKLOAD_PACK_WRITE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("failed to copy workload sources: {err}"),
                    )),
                }
            }
            Err(err) => diagnostics.push(Diagnostic::new(
                "X07WASM_WORKLOAD_PACK_WRITE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to write workload pack outputs: {err}"),
            )),
        },
        Err(err) => diagnostics.push(Diagnostic::new(
            "X07WASM_WORKLOAD_PACK_INPUT_INVALID",
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
        "x07-wasm.workload.pack",
        meta,
        surface::SurfaceReportPayload {
            diagnostics,
            stdout_json,
            output_path: Some(args.out_dir),
            copy_stats,
            checked_schema_ids: Vec::new(),
        },
    )
}

fn push_input_digest(meta: &mut report::meta::ReportMeta, path: &Path) {
    if let Ok(digest) = crate::util::file_digest(path) {
        meta.inputs.push(digest);
    }
}

fn build_runtime_pack_doc(
    out_dir: &Path,
    workload_doc: &Value,
    runtime_image: Option<&str>,
    container_port: u16,
) -> Result<Value> {
    let workload_id = workload_doc
        .get("workload_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cells = workload_doc
        .get("cells")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|cell| {
            let runtime_class = cell
                .get("runtime_class")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let ingress_kind = cell
                .get("ingress_kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut doc = json!({
                "cell_key": cell.get("cell_key").cloned().unwrap_or(Value::Null),
                "runtime_class": runtime_class,
                "ingress_kind": ingress_kind,
            });
            if runtime_class == "native-http" && ingress_kind == "http" {
                if let Some(image) = runtime_image.filter(|value| !value.trim().is_empty()) {
                    doc["executable"] = json!({
                        "kind": "oci_image",
                        "image": image,
                        "container_port": container_port,
                        "health_path": "/",
                    });
                }
            }
            doc
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema_version": "x07.workload.pack@0.1.0",
        "workload_id": workload_id,
        "public_manifest": relative_digest(out_dir, &out_dir.join("workload.pack.json"))?,
        "workload": relative_digest(out_dir, &out_dir.join("workload.describe.json"))?,
        "binding_requirements": relative_digest(out_dir, &out_dir.join("binding.requirements.json"))?,
        "topology": collect_matching_digests(out_dir, "topology.preview.", ".json")?,
        "sources": collect_relative_digests(&out_dir.join("sources"), out_dir)?,
        "cells": cells,
    }))
}

fn collect_matching_digests(out_dir: &Path, prefix: &str, suffix: &str) -> Result<Vec<Value>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(out_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if path.is_file() && name.starts_with(prefix) && name.ends_with(suffix) {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| relative_digest(out_dir, &path))
        .collect()
}

fn collect_relative_digests(root: &Path, pack_root: &Path) -> Result<Vec<Value>> {
    let mut files = Vec::new();
    collect_files_recursive(root, &mut files)?;
    files.sort();
    files
        .into_iter()
        .map(|path| relative_digest(pack_root, &path))
        .collect()
}

fn collect_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn relative_digest(root: &Path, path: &Path) -> Result<Value> {
    let digest = crate::util::file_digest(path)?;
    Ok(json!(relative_file_digest(root, digest)?))
}

fn relative_file_digest(root: &Path, digest: FileDigest) -> Result<FileDigest> {
    let rel_path = Path::new(&digest.path)
        .strip_prefix(root)?
        .to_string_lossy()
        .to_string();
    Ok(FileDigest {
        path: rel_path,
        sha256: digest.sha256,
        bytes_len: digest.bytes_len,
    })
}

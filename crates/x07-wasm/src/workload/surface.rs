use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jsonschema::{Draft, Resource};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::meta::{FileDigest, ReportMeta};
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
struct ProjectManifest {
    schema_version: String,
    entry: String,
    #[serde(default)]
    module_roots: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DomainPackRef {
    id: String,
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceCell {
    cell_key: String,
    cell_kind: String,
    entry_symbol: String,
    ingress_kind: String,
    runtime_class: String,
    scale_class: String,
    #[serde(default)]
    binding_refs: Vec<String>,
    topology_group: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceTopologyProfile {
    id: String,
    #[serde(default)]
    target_kind: Option<String>,
    placement: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceBinding {
    name: String,
    kind: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceManifest {
    schema_version: String,
    service_id: String,
    display_name: String,
    domain_pack: DomainPackRef,
    cells: Vec<ServiceCell>,
    #[serde(default)]
    topology_profiles: Vec<ServiceTopologyProfile>,
    #[serde(default)]
    resource_bindings: Vec<ServiceBinding>,
    #[serde(default)]
    default_trust_profile: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkloadSource {
    pub project_path: PathBuf,
    pub manifest_path: PathBuf,
    project: ProjectManifest,
    manifest: ServiceManifest,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CopyStats {
    pub files_copied: u64,
    pub bytes_copied: u64,
}

pub(crate) struct SurfaceArtifacts {
    pub pack_manifest: Value,
    pub workload_doc: Value,
    pub binding_doc: Value,
    pub topology_docs: Vec<(String, Value)>,
}

pub(crate) fn load_source(project_path: &Path, manifest_path: &Path) -> Result<WorkloadSource> {
    let project_bytes =
        fs::read(project_path).with_context(|| format!("read project {}", project_path.display()))?;
    let manifest_bytes = fs::read(manifest_path)
        .with_context(|| format!("read service manifest {}", manifest_path.display()))?;
    let project: ProjectManifest = serde_json::from_slice(&project_bytes)
        .with_context(|| format!("parse project {}", project_path.display()))?;
    let manifest: ServiceManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("parse service manifest {}", manifest_path.display()))?;
    validate_source_docs(&project, &manifest)?;
    Ok(WorkloadSource {
        project_path: project_path.to_path_buf(),
        manifest_path: manifest_path.to_path_buf(),
        project,
        manifest,
    })
}

pub(crate) fn load_source_from_pack(pack_manifest_path: &Path) -> Result<WorkloadSource> {
    let pack_dir = pack_manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("pack manifest must have a parent directory"))?;
    let project_path = pack_dir.join("sources").join("x07.json");
    let manifest_path = pack_dir
        .join("sources")
        .join("arch")
        .join("service")
        .join("index.x07service.json");
    load_source(&project_path, &manifest_path)
}

pub(crate) fn build_artifacts(
    source: &WorkloadSource,
    inspect_view: &str,
    preferred_profile: Option<&str>,
) -> Result<SurfaceArtifacts> {
    let workload_doc = workload_describe_doc(source, inspect_view)?;
    let binding_doc = binding_requirements_doc(source)?;
    let pack_manifest = pack_manifest_doc(source)?;
    let topology_docs = topology_docs(source, preferred_profile)?;
    Ok(SurfaceArtifacts {
        pack_manifest,
        workload_doc,
        binding_doc,
        topology_docs,
    })
}

pub(crate) fn write_build_outputs(
    source: &WorkloadSource,
    out_dir: &Path,
    preferred_profile: Option<&str>,
) -> Result<(Vec<FileDigest>, SurfaceArtifacts)> {
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let artifacts = build_artifacts(source, "full", preferred_profile)?;
    let mut outputs = Vec::new();
    outputs.push(write_json_doc(
        &out_dir.join("workload.pack.json"),
        &artifacts.pack_manifest,
    )?);
    outputs.push(write_json_doc(
        &out_dir.join("workload.describe.json"),
        &artifacts.workload_doc,
    )?);
    outputs.push(write_json_doc(
        &out_dir.join("binding.requirements.json"),
        &artifacts.binding_doc,
    )?);
    for (profile_id, doc) in &artifacts.topology_docs {
        outputs.push(write_json_doc(
            &out_dir.join(format!("topology.preview.{profile_id}.json")),
            doc,
        )?);
    }
    Ok((outputs, artifacts))
}

pub(crate) fn write_pack_sources(source: &WorkloadSource, out_dir: &Path) -> Result<CopyStats> {
    let project_root = project_root(&source.project_path);
    let sources_dir = out_dir.join("sources");
    fs::create_dir_all(&sources_dir).with_context(|| format!("create {}", sources_dir.display()))?;
    let mut seen = BTreeSet::new();
    let mut stats = CopyStats::default();
    copy_relative_file(
        &project_root,
        &relative_to(&project_root, &source.project_path)?,
        &sources_dir,
        &mut seen,
        &mut stats,
    )?;
    copy_relative_file(
        &project_root,
        &relative_to(&project_root, &source.manifest_path)?,
        &sources_dir,
        &mut seen,
        &mut stats,
    )?;
    for module_root in &source.project.module_roots {
        copy_relative_path(
            &project_root,
            Path::new(module_root),
            &sources_dir,
            &mut seen,
            &mut stats,
        )?;
    }
    let entry_path = project_root.join(&source.project.entry);
    if entry_path.exists() {
        copy_relative_file(
            &project_root,
            &relative_to(&project_root, &entry_path)?,
            &sources_dir,
            &mut seen,
            &mut stats,
        )?;
    }
    Ok(stats)
}

pub(crate) fn pack_manifest_doc(source: &WorkloadSource) -> Result<Value> {
    let cells = manifest_cells(source);
    let topology_profiles = source
        .manifest
        .topology_profiles
        .iter()
        .map(|profile| {
            json!({
                "id": profile.id,
                "placement": profile.placement,
            })
        })
        .collect::<Vec<_>>();
    let bindings_required = source
        .manifest
        .resource_bindings
        .iter()
        .map(|binding| binding.name.clone())
        .collect::<Vec<_>>();
    Ok(json!({
        "schema_version": "lp.workload.pack.manifest@0.1.0",
        "workload_id": source.manifest.service_id,
        "project": {
            "entry": source.project.entry,
            "module_roots": source.project.module_roots,
        },
        "cells": cells,
        "bindings_required": bindings_required,
        "topology_profiles": topology_profiles,
    }))
}

pub(crate) fn workload_describe_doc(source: &WorkloadSource, view: &str) -> Result<Value> {
    if !matches!(view, "summary" | "full") {
        anyhow::bail!("unsupported inspect view: {view}");
    }
    Ok(json!({
        "schema_version": "lp.workload.describe.result@0.1.0",
        "view": view,
        "workload_id": source.manifest.service_id,
        "display_name": source.manifest.display_name,
        "scope": {
            "org_id": "local",
            "project_id": source.manifest.domain_pack.id,
            "environment_id": Value::Null,
        },
        "default_target_kind": default_target_kind(source),
        "default_trust_profile": source.manifest.default_trust_profile,
        "cells": manifest_cells(source),
        "generated_unix_ms": 0,
    }))
}

pub(crate) fn binding_requirements_doc(source: &WorkloadSource) -> Result<Value> {
    let mut required_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for cell in &source.manifest.cells {
        for binding_ref in &cell.binding_refs {
            required_by
                .entry(binding_ref.clone())
                .or_default()
                .push(cell.cell_key.clone());
        }
    }
    let bindings = source
        .manifest
        .resource_bindings
        .iter()
        .map(|binding| {
            let mut cells = required_by.get(&binding.name).cloned().unwrap_or_default();
            cells.sort();
            let mut binding_doc = json!({
                "name": binding.name,
                "kind": binding.kind,
                "required_by_cells": cells,
                "required": binding.required,
            });
            if let Some(notes) = binding.notes.as_deref() {
                binding_doc["notes"] = json!(notes);
            }
            binding_doc
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema_version": "lp.binding.requirements.result@0.1.0",
        "workload_id": source.manifest.service_id,
        "bindings": bindings,
        "generated_unix_ms": 0,
    }))
}

pub(crate) fn topology_docs(
    source: &WorkloadSource,
    preferred_profile: Option<&str>,
) -> Result<Vec<(String, Value)>> {
    if source.manifest.topology_profiles.is_empty() {
        return Ok(vec![(
            "default".to_string(),
            topology_preview_doc(source, "default", Some("hosted"), "co-located")?,
        )]);
    }
    let mut docs = Vec::new();
    if let Some(profile_id) = preferred_profile {
        let profile = source
            .manifest
            .topology_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("unknown topology profile: {profile_id}"))?;
        docs.push((
            profile.id.clone(),
            topology_preview_doc(
                source,
                &profile.id,
                profile.target_kind.as_deref(),
                &profile.placement,
            )?,
        ));
        return Ok(docs);
    }
    for profile in &source.manifest.topology_profiles {
        docs.push((
            profile.id.clone(),
            topology_preview_doc(
                source,
                &profile.id,
                profile.target_kind.as_deref(),
                &profile.placement,
            )?,
        ));
    }
    Ok(docs)
}

fn topology_preview_doc(
    source: &WorkloadSource,
    profile_id: &str,
    target_kind: Option<&str>,
    placement: &str,
) -> Result<Value> {
    let mut groups: BTreeMap<String, Vec<&ServiceCell>> = BTreeMap::new();
    for cell in &source.manifest.cells {
        let group_key = if placement == "split-by-cell" {
            cell.cell_key.clone()
        } else {
            cell.topology_group.clone()
        };
        groups.entry(group_key).or_default().push(cell);
    }

    let groups = groups
        .into_iter()
        .map(|(group_key, cells)| {
            let mut cell_keys = cells
                .iter()
                .map(|cell| cell.cell_key.clone())
                .collect::<Vec<_>>();
            cell_keys.sort();
            let runtime_class = collapse_or_first(cells.iter().map(|cell| cell.runtime_class.as_str()));
            let scale_class = collapse_or_first(cells.iter().map(|cell| cell.scale_class.as_str()));
            let exposure = if cells.iter().any(|cell| matches!(cell.ingress_kind.as_str(), "http" | "mcp")) {
                "public"
            } else {
                "none"
            };
            let mut notes = Vec::new();
            if cells.iter().any(|cell| cell.ingress_kind == "http") {
                notes.push("ingress required".to_string());
            }
            if cells.iter().any(|cell| cell.ingress_kind == "event") {
                notes.push("queue-driven".to_string());
            }
            if cells.iter().any(|cell| cell.ingress_kind == "schedule") {
                notes.push("scheduler required".to_string());
            }
            if cells.len() > 1 {
                notes.push("co-located by topology group".to_string());
            }
            json!({
                "group_key": group_key,
                "cell_keys": cell_keys,
                "runtime_class": runtime_class,
                "scale_class": scale_class,
                "exposure": exposure,
                "notes": notes,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "schema_version": "lp.topology.preview.result@0.1.0",
        "workload_id": source.manifest.service_id,
        "profile_id": profile_id,
        "target_kind": target_kind.unwrap_or("hosted"),
        "groups": groups,
        "generated_unix_ms": 0,
    }))
}

fn manifest_cells(source: &WorkloadSource) -> Vec<Value> {
    source
        .manifest
        .cells
        .iter()
        .map(|cell| {
            json!({
                "cell_id": format!("{}.{}", source.manifest.service_id, cell.cell_key),
                "cell_key": cell.cell_key,
                "cell_kind": cell.cell_kind,
                "ingress_kind": cell.ingress_kind,
                "runtime_class": cell.runtime_class,
                "scale_class": cell.scale_class,
                "topology_group": cell.topology_group,
                "binding_refs": cell.binding_refs,
            })
        })
        .collect()
}

fn validate_source_docs(project: &ProjectManifest, manifest: &ServiceManifest) -> Result<()> {
    if project.schema_version.trim().is_empty() {
        anyhow::bail!("project schema_version must not be empty");
    }
    if project.entry.trim().is_empty() {
        anyhow::bail!("project entry must not be empty");
    }
    if project.module_roots.is_empty() {
        anyhow::bail!("project module_roots must not be empty");
    }
    if manifest.schema_version != "x07.service.manifest@0.1.0" {
        anyhow::bail!("unsupported service manifest schema_version: {}", manifest.schema_version);
    }
    if !workload_id_valid(&manifest.service_id) {
        anyhow::bail!("invalid workload_id/service_id: {}", manifest.service_id);
    }
    if manifest.display_name.trim().is_empty() {
        anyhow::bail!("service display_name must not be empty");
    }
    if manifest.domain_pack.id.trim().is_empty() || manifest.domain_pack.display_name.trim().is_empty() {
        anyhow::bail!("domain_pack id/display_name must not be empty");
    }
    if manifest.cells.is_empty() {
        anyhow::bail!("service manifest must define at least one cell");
    }
    let mut binding_names = BTreeSet::new();
    for binding in &manifest.resource_bindings {
        if binding.name.trim().is_empty() {
            anyhow::bail!("binding name must not be empty");
        }
        if !binding_names.insert(binding.name.clone()) {
            anyhow::bail!("duplicate binding name: {}", binding.name);
        }
        validate_binding_kind(&binding.kind)?;
    }
    let mut cell_keys = BTreeSet::new();
    for cell in &manifest.cells {
        if cell.cell_key.trim().is_empty() {
            anyhow::bail!("cell_key must not be empty");
        }
        if !cell_keys.insert(cell.cell_key.clone()) {
            anyhow::bail!("duplicate cell_key: {}", cell.cell_key);
        }
        if cell.entry_symbol.trim().is_empty() {
            anyhow::bail!("entry_symbol must not be empty");
        }
        if cell.topology_group.trim().is_empty() {
            anyhow::bail!("topology_group must not be empty");
        }
        validate_cell_value("cell_kind", &cell.cell_kind, &["api-cell","event-consumer","scheduled-job","policy-service","workflow-service","mcp-tool"])?;
        validate_cell_value("ingress_kind", &cell.ingress_kind, &["http","event","schedule","workflow","mcp"])?;
        validate_cell_value("runtime_class", &cell.runtime_class, &["wasm-component","native-http","native-worker","embedded-kernel"])?;
        validate_cell_value("scale_class", &cell.scale_class, &["replicated-http","partitioned-consumer","singleton-orchestrator","leased-worker","burst-batch","embedded-kernel"])?;
        for binding_ref in &cell.binding_refs {
            if !binding_names.contains(binding_ref) {
                anyhow::bail!("cell {} references unknown binding {}", cell.cell_key, binding_ref);
            }
        }
    }
    let mut topology_ids = BTreeSet::new();
    for profile in &manifest.topology_profiles {
        if profile.id.trim().is_empty() {
            anyhow::bail!("topology profile id must not be empty");
        }
        if !topology_ids.insert(profile.id.clone()) {
            anyhow::bail!("duplicate topology profile id: {}", profile.id);
        }
        validate_cell_value("placement", &profile.placement, &["co-located","split-by-cell","embedded-kernel"])?;
        if let Some(target_kind) = profile.target_kind.as_deref() {
            validate_cell_value("target_kind", target_kind, &["hosted","k8s","wasmcloud"])?;
        }
    }
    Ok(())
}

fn validate_binding_kind(kind: &str) -> Result<()> {
    validate_cell_value(
        "binding kind",
        kind,
        &["postgres", "mysql", "sqlite", "redis", "kafka", "amqp", "s3", "secret", "otlp"],
    )
}

fn validate_cell_value(label: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.iter().any(|allowed_value| *allowed_value == value) {
        return Ok(());
    }
    anyhow::bail!("{label} has unsupported value: {value}");
}

fn workload_id_valid(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn default_target_kind(source: &WorkloadSource) -> &str {
    source
        .manifest
        .topology_profiles
        .iter()
        .find_map(|profile| profile.target_kind.as_deref())
        .unwrap_or("hosted")
}

fn collapse_or_first<'a>(values: impl Iterator<Item = &'a str>) -> &'a str {
    let mut seen = BTreeSet::new();
    let mut first = "mixed";
    for value in values {
        if seen.is_empty() {
            first = value;
        }
        seen.insert(value);
    }
    if seen.len() == 1 { first } else { "mixed" }
}

pub(crate) fn resolve_schema_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_dir() {
            return Ok(path.to_path_buf());
        }
        anyhow::bail!("schema_dir does not exist: {}", path.display());
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for ancestor in cwd.ancestors() {
        let direct = ancestor.join("contracts").join("spec").join("schemas");
        if direct.is_dir() {
            return Ok(direct);
        }
        let sibling = ancestor.join("x07-platform-contracts").join("spec").join("schemas");
        if sibling.is_dir() {
            return Ok(sibling);
        }
    }
    anyhow::bail!("unable to locate x07-platform-contracts schema dir; pass --schema-dir")
}

pub(crate) fn validate_contract_docs(
    schema_dir: &Path,
    docs: &[(&str, &Value)],
) -> Result<Vec<Diagnostic>> {
    let mut by_id = BTreeMap::new();
    let mut schemas_by_name = BTreeMap::new();
    for entry in fs::read_dir(schema_dir).with_context(|| format!("read {}", schema_dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("lp.") || !name.ends_with(".schema.json") {
            continue;
        }
        let bytes = fs::read(&path).with_context(|| format!("read schema {}", path.display()))?;
        let doc: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse schema {}", path.display()))?;
        let schema_id = doc
            .get("$id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("schema missing $id: {}", path.display()))?;
        by_id.insert(schema_id.to_string(), doc.clone());
        schemas_by_name.insert(name.to_string(), doc);
    }
    let resources = by_id
        .iter()
        .map(|(id, doc)| (id.clone(), Resource::from_contents(doc.clone())));
    let mut diagnostics = Vec::new();
    for (schema_name, instance) in docs {
        let schema = schemas_by_name
            .get(*schema_name)
            .ok_or_else(|| anyhow::anyhow!("missing schema {} in {}", schema_name, schema_dir.display()))?
            .clone();
        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .with_resources(resources.clone())
            .build(&schema)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        for err in validator.iter_errors(instance) {
            let mut diagnostic = Diagnostic::new(
                "X07WASM_WORKLOAD_CONTRACT_INVALID",
                Severity::Error,
                Stage::Lint,
                format!("{schema_name}: {err}"),
            );
            diagnostic
                .data
                .insert("instance_path".to_string(), json!(err.instance_path().to_string()));
            diagnostic
                .data
                .insert("schema_path".to_string(), json!(err.schema_path().to_string()));
            diagnostics.push(diagnostic);
        }
    }
    Ok(diagnostics)
}

pub(crate) fn emit_report(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    command: &str,
    mut meta: ReportMeta,
    diagnostics: Vec<Diagnostic>,
    stdout_json: Value,
    output_path: Option<&Path>,
    copy_stats: CopyStats,
    checked_schema_ids: Vec<String>,
) -> Result<u8> {
    let store = SchemaStore::new()?;
    let ok = diagnostics.iter().all(|diagnostic| diagnostic.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    meta.elapsed_ms = started.elapsed().as_millis() as u64;
    let stdout_bytes_len = report::canon::canonical_json_bytes(&stdout_json)?.len() as u64;
    let report_doc = json!({
        "schema_version": "x07.wasm.workload.surface.report@0.1.0",
        "command": command,
        "ok": ok,
        "exit_code": exit_code,
        "diagnostics": diagnostics,
        "meta": meta,
        "result": {
            "stdout": { "bytes_len": stdout_bytes_len },
            "stderr": { "bytes_len": 0 },
            "stdout_json": stdout_json,
            "output_path": output_path.map(|path| path.display().to_string()),
            "files_copied": copy_stats.files_copied,
            "bytes_copied": copy_stats.bytes_copied,
            "checked_schema_ids": checked_schema_ids,
        }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

pub(crate) fn write_json_doc(path: &Path, doc: &Value) -> Result<FileDigest> {
    let bytes = report::canon::canonical_pretty_json_bytes(doc)?;
    util::write_file_atomic(path, &bytes).with_context(|| format!("write {}", path.display()))?;
    util::file_digest(path)
}

fn copy_relative_path(
    root: &Path,
    rel_path: &Path,
    dst_root: &Path,
    seen: &mut BTreeSet<PathBuf>,
    stats: &mut CopyStats,
) -> Result<()> {
    let src_path = root.join(rel_path);
    if src_path.is_file() {
        return copy_relative_file(root, rel_path, dst_root, seen, stats);
    }
    if src_path.is_dir() {
        for entry in fs::read_dir(&src_path).with_context(|| format!("read {}", src_path.display()))? {
            let entry = entry?;
            let rel = rel_path.join(entry.file_name());
            copy_relative_path(root, &rel, dst_root, seen, stats)?;
        }
    }
    Ok(())
}

fn copy_relative_file(
    root: &Path,
    rel_path: &Path,
    dst_root: &Path,
    seen: &mut BTreeSet<PathBuf>,
    stats: &mut CopyStats,
) -> Result<()> {
    if !seen.insert(rel_path.to_path_buf()) {
        return Ok(());
    }
    let src_path = root.join(rel_path);
    if !src_path.is_file() {
        return Ok(());
    }
    let dst_path = dst_root.join(rel_path);
    if let Some(parent) = dst_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let bytes = fs::read(&src_path).with_context(|| format!("read {}", src_path.display()))?;
    util::write_file_atomic(&dst_path, &bytes).with_context(|| format!("write {}", dst_path.display()))?;
    stats.files_copied += 1;
    stats.bytes_copied += bytes.len() as u64;
    Ok(())
}

fn relative_to(root: &Path, path: &Path) -> Result<PathBuf> {
    path.strip_prefix(root)
        .map(|path| path.to_path_buf())
        .with_context(|| format!("path {} is outside {}", path.display(), root.display()))
}

fn project_root(project_path: &Path) -> PathBuf {
    project_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("x07-wasm-{label}-{}-{ts}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn write_fixture_project(root: &Path) {
        fs::create_dir_all(root.join("src/service")).expect("mkdir src/service");
        fs::create_dir_all(root.join("src/domain")).expect("mkdir src/domain");
        fs::create_dir_all(root.join("arch/service")).expect("mkdir arch/service");
        fs::write(
            root.join("x07.json"),
            r#"{
  "schema_version": "x07.project@0.4.0",
  "entry": "src/main.x07.json",
  "module_roots": ["src", "tests"]
}"#,
        )
        .expect("write x07.json");
        fs::write(root.join("src/main.x07.json"), "{}").expect("write main");
        fs::write(root.join("src/service/api.x07.json"), "{}").expect("write api");
        fs::write(root.join("src/service/worker.x07.json"), "{}").expect("write worker");
        fs::write(root.join("src/domain/orders.x07.json"), "{}").expect("write domain");
        fs::write(
            root.join("arch/service/index.x07service.json"),
            r#"{
  "schema_version": "x07.service.manifest@0.1.0",
  "service_id": "orders",
  "display_name": "Orders",
  "domain_pack": { "id": "orders", "display_name": "Orders" },
  "cells": [
    {
      "cell_key": "api",
      "cell_kind": "api-cell",
      "entry_symbol": "orders.api.main",
      "ingress_kind": "http",
      "runtime_class": "native-http",
      "scale_class": "replicated-http",
      "binding_refs": ["db.primary", "obj.documents"],
      "topology_group": "frontdoor"
    },
    {
      "cell_key": "events",
      "cell_kind": "event-consumer",
      "entry_symbol": "orders.events.main",
      "ingress_kind": "event",
      "runtime_class": "native-worker",
      "scale_class": "partitioned-consumer",
      "binding_refs": ["db.primary", "msg.orders"],
      "topology_group": "async"
    },
    {
      "cell_key": "settlement",
      "cell_kind": "scheduled-job",
      "entry_symbol": "orders.settlement.main",
      "ingress_kind": "schedule",
      "runtime_class": "native-worker",
      "scale_class": "burst-batch",
      "binding_refs": ["db.primary"],
      "topology_group": "async"
    }
  ],
  "topology_profiles": [
    { "id": "dev", "target_kind": "hosted", "placement": "co-located" },
    { "id": "prod", "target_kind": "k8s", "placement": "split-by-cell" }
  ],
  "resource_bindings": [
    { "name": "db.primary", "kind": "postgres", "required": true },
    { "name": "msg.orders", "kind": "amqp", "required": true },
    { "name": "obj.documents", "kind": "s3", "required": false }
  ],
  "default_trust_profile": "sandboxed_service_v1"
}"#,
        )
        .expect("write service manifest");
    }

    #[test]
    fn builds_pack_manifest_and_topology_docs_for_multi_cell_service() {
        let root = temp_dir("surface-build");
        write_fixture_project(&root);
        let source = load_source(
            &root.join("x07.json"),
            &root.join("arch/service/index.x07service.json"),
        )
        .expect("load source");
        let artifacts = build_artifacts(&source, "full", None).expect("build artifacts");
        assert_eq!(artifacts.pack_manifest["workload_id"], "orders");
        assert_eq!(artifacts.pack_manifest["cells"].as_array().expect("cells").len(), 3);
        assert_eq!(artifacts.binding_doc["bindings"].as_array().expect("bindings").len(), 3);
        assert_eq!(artifacts.topology_docs.len(), 2);
        let prod = artifacts
            .topology_docs
            .iter()
            .find(|(profile_id, _)| profile_id == "prod")
            .expect("prod topology");
        assert_eq!(prod.1["target_kind"], "k8s");
        assert_eq!(prod.1["groups"].as_array().expect("groups").len(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pack_sources_snapshot_includes_project_and_manifest() {
        let root = temp_dir("surface-pack");
        write_fixture_project(&root);
        let out_dir = root.join("dist/workload");
        let source = load_source(
            &root.join("x07.json"),
            &root.join("arch/service/index.x07service.json"),
        )
        .expect("load source");
        let stats = write_pack_sources(&source, &out_dir).expect("write pack sources");
        assert!(stats.files_copied >= 4);
        assert!(out_dir.join("sources/x07.json").is_file());
        assert!(out_dir.join("sources/arch/service/index.x07service.json").is_file());
        let _ = fs::remove_dir_all(root);
    }
}

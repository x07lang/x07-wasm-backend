use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::cli::{MachineArgs, ProfileValidateArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_PROFILE_ID: &str = "wasm_release";

#[derive(Debug, Clone, Deserialize)]
struct WasmIndexDoc {
    profiles: Vec<WasmIndexProfileRef>,
    #[serde(default)]
    defaults: Option<WasmIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
struct WasmIndexDefaults {
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WasmIndexProfileRef {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileDoc {
    pub id: String,
    pub v: u64,
    pub target: WasmProfileTarget,
    pub x07_build: WasmProfileX07Build,
    pub clang: WasmProfileClang,
    pub wasm_ld: WasmProfileWasmLd,
    pub defaults: WasmProfileDefaults,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileTarget {
    pub triple: String,
    pub wasm_c_abi: String,
    pub entry_export: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileX07Build {
    pub freestanding: bool,
    #[serde(default)]
    pub emit_c_header: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileClang {
    #[serde(default)]
    pub cc: Option<String>,
    pub cflags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileWasmLd {
    #[serde(default)]
    pub linker: Option<String>,
    pub ldflags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmProfileDefaults {
    pub arena_cap_bytes: u64,
    pub max_output_bytes: u64,
}

pub struct LoadedProfile {
    pub digest: report::meta::FileDigest,
    pub doc: WasmProfileDoc,
    pub index_digest: Option<report::meta::FileDigest>,
}

pub fn cmd_profile_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ProfileValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut profiles_status: Vec<serde_json::Value> = Vec::new();

    if let Some(file) = &args.profile_file {
        let profile_status = validate_profile_file(&store, file, None, &mut meta, &mut diagnostics);
        profiles_status.push(profile_status);

        let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
        let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
        let report_doc = json!({
          "schema_version": "x07.wasm.profile.validate.report@0.1.0",
          "command": "x07-wasm.profile.validate",
          "ok": ok,
          "exit_code": exit_code,
          "diagnostics": diagnostics,
          "meta": meta,
          "result": {
            "mode": "profile_file_v1",
            "index": null,
            "profiles": profiles_status,
          }
        });
        store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
        return Ok(exit_code);
    }

    let index_path = args.index.clone();
    let mut index_digest = report::meta::FileDigest {
        path: index_path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let index_bytes = match std::fs::read(&index_path) {
        Ok(b) => {
            index_digest.sha256 = util::sha256_hex(&b);
            index_digest.bytes_len = b.len() as u64;
            b
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read index {}: {err}", index_path.display()),
            ));
            Vec::new()
        }
    };
    meta.inputs.push(index_digest.clone());

    let mut index_ok = true;
    let mut index_doc_json_ok = true;
    let index_doc: serde_json::Value = match serde_json::from_slice(&index_bytes) {
        Ok(v) => v,
        Err(err) => {
            index_doc_json_ok = false;
            index_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_INDEX_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index JSON: {err}"),
            ));
            json!(null)
        }
    };

    if index_doc_json_ok {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-arch.wasm.index.schema.json",
            &index_doc,
        )?);

        if diagnostics.iter().any(|d| d.severity == Severity::Error) {
            index_ok = false;
        }
    }

    let index_parsed: Option<WasmIndexDoc> = if index_doc_json_ok {
        match serde_json::from_value(index_doc.clone()) {
            Ok(v) => Some(v),
            Err(err) => {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WASM_INDEX_PARSE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse index doc: {err}"),
                ));
                None
            }
        }
    } else {
        None
    };

    if let Some(idx) = index_parsed.as_ref() {
        let mut seen = std::collections::BTreeSet::new();
        for p in &idx.profiles {
            if !seen.insert(p.id.clone()) {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WASM_INDEX_DUPLICATE_PROFILE_ID",
                    Severity::Error,
                    Stage::Parse,
                    format!("duplicate profile id in index: {:?}", p.id),
                ));
            }
        }
    }

    let mut default_profile_id: Option<String> = None;
    let mut default_profile_found = false;
    let mut profiles_count: u64 = 0;

    if let Some(idx) = index_parsed.as_ref() {
        profiles_count = idx.profiles.len() as u64;
        default_profile_id = idx
            .defaults
            .as_ref()
            .and_then(|d| d.default_profile_id.clone())
            .or_else(|| Some(DEFAULT_PROFILE_ID.to_string()));
        if let Some(def) = default_profile_id.as_deref() {
            default_profile_found = idx.profiles.iter().any(|p| p.id == def);
            if !default_profile_found {
                index_ok = false;
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WASM_INDEX_DEFAULT_PROFILE_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("default profile id not found in profiles list: {def:?}"),
                ));
            }
        }
    }

    let mode = if args.profile.is_some() {
        "profile_id_v1"
    } else {
        "index_v1"
    };

    if let Some(idx) = index_parsed.as_ref() {
        let wanted = args.profile.as_deref();
        for p in &idx.profiles {
            if let Some(w) = wanted {
                if p.id != w {
                    continue;
                }
            }
            let profile_path = PathBuf::from(&p.path);
            let status =
                validate_profile_file(&store, &profile_path, Some(p), &mut meta, &mut diagnostics);
            profiles_status.push(status);
        }
        if wanted.is_some() && profiles_status.is_empty() {
            index_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_INDEX_PROFILE_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!("profile id not found: {:?}", wanted.unwrap()),
            ));
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = json!({
      "schema_version": "x07.wasm.profile.validate.report@0.1.0",
      "command": "x07-wasm.profile.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "index": {
          "path": index_digest.path,
          "ok": index_ok,
          "profiles_count": profiles_count,
          "default_profile_id": default_profile_id,
          "default_profile_found": default_profile_found,
        },
        "profiles": profiles_status,
      }
    });
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn validate_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_ref: Option<&WasmIndexProfileRef>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> serde_json::Value {
    let mut digest = report::meta::FileDigest {
        path: path.display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => {
            digest.sha256 = util::sha256_hex(&b);
            digest.bytes_len = b.len() as u64;
            b
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read profile {}: {err}", path.display()),
            ));
            meta.inputs.push(digest.clone());
            let ref_id = sanitize_profile_id(index_ref, None);
            return json!({
              "ref": { "id": ref_id, "v": 1 },
              "path": digest.path,
              "ok": false,
              "schema_version": null,
              "schema_valid": false,
              "id_matches_index": index_ref.is_none(),
            });
        }
    };
    meta.inputs.push(digest.clone());

    let doc: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROFILE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse profile JSON {}: {err}", path.display()),
            ));
            let ref_id = sanitize_profile_id(index_ref, None);
            return json!({
              "ref": { "id": ref_id, "v": 1 },
              "path": digest.path,
              "ok": false,
              "schema_version": null,
              "schema_valid": false,
              "id_matches_index": index_ref.is_none(),
            });
        }
    };

    let schema_diags =
        match store.validate("https://x07.io/spec/x07-wasm.profile.schema.json", &doc) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_INTERNAL_SCHEMA_VALIDATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                Vec::new()
            }
        };
    let schema_valid = schema_diags.is_empty();
    diagnostics.extend(schema_diags);

    let schema_version = doc
        .get("schema_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let id = doc
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let v = doc.get("v").and_then(|v| v.as_i64()).unwrap_or(1);
    let v = if v < 1 { 1 } else { v } as u64;

    let mut id_matches_index = true;
    if let Some(idx) = index_ref {
        if id.as_deref() != Some(idx.id.as_str()) {
            id_matches_index = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROFILE_ID_MISMATCH",
                Severity::Error,
                Stage::Parse,
                format!(
                    "profile id mismatch: index.id={:?} profile.id={:?} path={}",
                    idx.id,
                    id,
                    path.display()
                ),
            ));
        }
    }

    let ref_id = sanitize_profile_id(index_ref, id.as_deref());
    let ok = schema_valid && id_matches_index;
    json!({
      "ref": { "id": ref_id, "v": v },
      "path": digest.path,
      "ok": ok,
      "schema_version": schema_version,
      "schema_valid": schema_valid,
      "id_matches_index": id_matches_index,
    })
}

fn sanitize_profile_id(index_ref: Option<&WasmIndexProfileRef>, raw_id: Option<&str>) -> String {
    if let Some(idx) = index_ref {
        return idx.id.clone();
    }
    if let Some(id) = raw_id {
        if is_valid_profile_id(id) {
            return id.to_string();
        }
    }
    "unknown".to_string()
}

fn is_valid_profile_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    let mut it = s.chars();
    let Some(first) = it.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    for ch in it {
        if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-') {
            return false;
        }
    }
    true
}

pub fn load_profile(
    store: &SchemaStore,
    index_path: &PathBuf,
    profile_id: Option<&str>,
    profile_file: Option<&PathBuf>,
) -> Result<LoadedProfile> {
    if let Some(path) = profile_file {
        return load_profile_file(store, path, None);
    }

    let index_digest = util::file_digest(index_path)?;
    let index_bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let index_doc: serde_json::Value = serde_json::from_slice(&index_bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-arch.wasm.index.schema.json",
        &index_doc,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid wasm index: {}", index_digest.path);
    }

    let idx: WasmIndexDoc = serde_json::from_value(index_doc)
        .with_context(|| format!("parse index doc: {}", index_path.display()))?;
    let default_id = idx
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
        .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
    let wanted = profile_id.unwrap_or(&default_id);
    let entry = idx
        .profiles
        .iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| anyhow::anyhow!("profile id not found in index: {wanted:?}"))?;
    let profile_path = PathBuf::from(&entry.path);
    load_profile_file(store, &profile_path, Some(index_digest))
}

fn load_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_digest: Option<report::meta::FileDigest>,
) -> Result<LoadedProfile> {
    let digest = util::file_digest(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-wasm.profile.schema.json",
        &doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid wasm profile: {}", digest.path);
    }
    let doc: WasmProfileDoc = serde_json::from_value(doc_json)
        .with_context(|| format!("parse wasm profile doc: {}", path.display()))?;
    validate_profile_contract(&doc)?;
    Ok(LoadedProfile {
        digest,
        doc,
        index_digest,
    })
}

fn validate_profile_contract(doc: &WasmProfileDoc) -> Result<()> {
    if doc.target.wasm_c_abi.trim() != "basic_c_abi@1" {
        anyhow::bail!("unsupported wasm_c_abi: {:?}", doc.target.wasm_c_abi);
    }
    if doc.target.entry_export.trim() != "x07_solve_v2" {
        anyhow::bail!("unsupported entry_export: {:?}", doc.target.entry_export);
    }
    if !doc.x07_build.freestanding {
        anyhow::bail!("profile must set x07_build.freestanding=true");
    }
    if !doc.x07_build.emit_c_header {
        anyhow::bail!("profile must set x07_build.emit_c_header=true");
    }
    Ok(())
}

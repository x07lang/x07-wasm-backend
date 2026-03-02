use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::Regex;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, ToolchainValidateArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
struct ToolchainIndexDoc {
    #[serde(default)]
    defaults: Option<ToolchainIndexDefaults>,
    profiles: Vec<ToolchainIndexProfileRef>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolchainIndexDefaults {
    #[serde(default)]
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolchainIndexProfileRef {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolchainProfileDoc {
    id: String,
    v: u64,
    tools: ToolchainProfileTools,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolchainProfileTools {
    x07: ToolReq,
    clang: ToolReq,
    wasm_ld: ToolReq,
    wasmtime: ToolReq,
    wasm_tools: ToolReq,
    wit_bindgen: ToolReq,
    wac: ToolReq,
    jco: ToolReq,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolReq {
    #[serde(default)]
    cmd: Option<String>,
    version: VersionProbe,
    constraint: String,
    required: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct VersionProbe {
    argv: Vec<String>,
    regex: String,
    group: u32,
    source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolId {
    X07,
    Clang,
    WasmLd,
    Wasmtime,
    WasmTools,
    WitBindgen,
    Wac,
    Jco,
}

impl ToolId {
    fn as_str(self) -> &'static str {
        match self {
            ToolId::X07 => "x07",
            ToolId::Clang => "clang",
            ToolId::WasmLd => "wasm_ld",
            ToolId::Wasmtime => "wasmtime",
            ToolId::WasmTools => "wasm_tools",
            ToolId::WitBindgen => "wit_bindgen",
            ToolId::Wac => "wac",
            ToolId::Jco => "jco",
        }
    }

    fn default_cmd(self) -> &'static str {
        match self {
            ToolId::X07 => "x07",
            ToolId::Clang => "clang",
            ToolId::WasmLd => "wasm-ld",
            ToolId::Wasmtime => "wasmtime",
            ToolId::WasmTools => "wasm-tools",
            ToolId::WitBindgen => "wit-bindgen",
            ToolId::Wac => "wac",
            ToolId::Jco => "jco",
        }
    }
}

pub fn cmd_toolchain_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ToolchainValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut index_summary: Option<Value> = None;

    let (mode, resolved_profile_path, expected_profile_id) = if let Some(profile) =
        args.profile.as_ref()
    {
        ("profile_file_v1", profile.clone(), None)
    } else {
        let (idx_doc, idx_ok) = load_index_doc(&store, &args.index, &mut meta, &mut diagnostics)?;
        let idx = idx_doc.unwrap_or(ToolchainIndexDoc {
            defaults: None,
            profiles: Vec::new(),
        });

        let default_profile_id = idx
            .defaults
            .as_ref()
            .and_then(|d| d.default_profile_id.clone());

        let default_profile_found = default_profile_id
            .as_ref()
            .is_some_and(|id| idx.profiles.iter().any(|p| &p.id == id));

        index_summary = Some(json!({
          "path": args.index.display().to_string(),
          "ok": idx_ok,
          "profiles_count": idx.profiles.len() as u64,
          "default_profile_id": default_profile_id,
          "default_profile_found": default_profile_found,
        }));

        let profile_id = if let Some(id) = args.profile_id.as_ref() {
            id.clone()
        } else if let Some(id) = idx
            .defaults
            .as_ref()
            .and_then(|d| d.default_profile_id.clone())
        {
            id
        } else {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                "no --profile provided and toolchain index has no defaults.default_profile_id"
                    .to_string(),
            ));
            "unknown".to_string()
        };

        let resolved = idx
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .map(|p| PathBuf::from(p.path.clone()))
            .unwrap_or_else(|| {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_TOOLCHAIN_PROFILE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("profile id not found in index: {profile_id:?}"),
                ));
                PathBuf::from("arch/wasm/toolchain/profiles/missing.json")
            });

        let mode = if args.profile_id.is_some() {
            "profile_id_v1"
        } else {
            "index_v1"
        };
        (mode, resolved, Some(profile_id))
    };

    let profile_status = load_profile_status(
        &store,
        &resolved_profile_path,
        expected_profile_id.as_deref(),
        &mut meta,
        &mut diagnostics,
    )?;

    let mut tools: Vec<Value> = Vec::new();
    let mut compatibility_items: Vec<Value> = Vec::new();

    let profile_doc_opt = profile_status
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| {
            read_profile_doc(&store, &resolved_profile_path)
                .ok()
                .and_then(|v| serde_json::from_value::<ToolchainProfileDoc>(v).ok())
        })
        .flatten();

    if let Some(profile_doc) = profile_doc_opt.as_ref() {
        for tool_id in [
            ToolId::X07,
            ToolId::Clang,
            ToolId::WasmLd,
            ToolId::Wasmtime,
            ToolId::WasmTools,
            ToolId::WitBindgen,
            ToolId::Wac,
            ToolId::Jco,
        ] {
            let req = tool_req(profile_doc, tool_id);
            let status = check_tool(tool_id, req, &mut diagnostics);
            let found = status.get("found_version").cloned().unwrap_or(Value::Null);
            compatibility_items.push(json!({
              "tool_id": tool_id.as_str(),
              "required_constraint": req.constraint.clone(),
              "found_version": found,
            }));
            tools.push(status);
        }
    }

    let compatibility_hash = {
        let doc = json!({
          "profile": {
            "id": profile_status.get("ref").and_then(|r| r.get("id")).cloned().unwrap_or(json!("unknown")),
            "v": profile_status.get("ref").and_then(|r| r.get("v")).cloned().unwrap_or(json!(1)),
          },
          "tools": compatibility_items,
        });
        let bytes = report::canon::canonical_json_bytes(&doc)?;
        util::sha256_hex(&bytes)
    };

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.toolchain.validate.report@0.1.0",
      "command": "x07-wasm.toolchain.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "index": index_summary,
        "profile": profile_status,
        "tools": tools,
        "compatibility_hash": compatibility_hash,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn tool_req(doc: &ToolchainProfileDoc, tool_id: ToolId) -> &ToolReq {
    match tool_id {
        ToolId::X07 => &doc.tools.x07,
        ToolId::Clang => &doc.tools.clang,
        ToolId::WasmLd => &doc.tools.wasm_ld,
        ToolId::Wasmtime => &doc.tools.wasmtime,
        ToolId::WasmTools => &doc.tools.wasm_tools,
        ToolId::WitBindgen => &doc.tools.wit_bindgen,
        ToolId::Wac => &doc.tools.wac,
        ToolId::Jco => &doc.tools.jco,
    }
}

fn check_tool(tool_id: ToolId, req: &ToolReq, diagnostics: &mut Vec<Diagnostic>) -> Value {
    let cmd = req
        .cmd
        .clone()
        .unwrap_or_else(|| tool_id.default_cmd().to_string());
    let argv = req.version.argv.clone();

    let severity = if req.required {
        Severity::Error
    } else {
        Severity::Warning
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut found_version: Option<String> = None;
    let mut ok = false;

    let out = std::process::Command::new(&cmd).args(&argv).output();
    match out {
        Ok(o) => {
            stdout = truncate_4096(&String::from_utf8_lossy(&o.stdout));
            stderr = truncate_4096(&String::from_utf8_lossy(&o.stderr));

            let version_str = match extract_version(&req.version, &stdout, &stderr) {
                Ok(v) => v,
                Err(why) => {
                    let mut d = Diagnostic::new(
                        "X07WASM_TOOLCHAIN_VERSION_PARSE_FAILED",
                        severity,
                        Stage::Run,
                        format!("failed to parse tool version: {why}"),
                    );
                    d.data
                        .insert("tool_id".to_string(), json!(tool_id.as_str()));
                    d.data.insert("stdout".to_string(), json!(stdout.clone()));
                    d.data.insert("stderr".to_string(), json!(stderr.clone()));
                    diagnostics.push(d);
                    String::new()
                }
            };

            if !version_str.is_empty() {
                found_version = Some(version_str.clone());
                match (
                    Version::parse(&version_str),
                    VersionReq::parse(&req.constraint),
                ) {
                    (Ok(v), Ok(want)) => {
                        if want.matches(&v) {
                            ok = true;
                        } else {
                            let mut d = Diagnostic::new(
                                "X07WASM_TOOLCHAIN_VERSION_CONSTRAINT_UNSATISFIED",
                                severity,
                                Stage::Run,
                                "tool version constraint unsatisfied".to_string(),
                            );
                            d.data
                                .insert("tool_id".to_string(), json!(tool_id.as_str()));
                            d.data
                                .insert("expected".to_string(), json!(req.constraint.clone()));
                            d.data.insert("found".to_string(), json!(version_str));
                            diagnostics.push(d);
                        }
                    }
                    (Err(err), _) => {
                        let mut d = Diagnostic::new(
                            "X07WASM_TOOLCHAIN_VERSION_PARSE_FAILED",
                            severity,
                            Stage::Run,
                            format!("failed to parse SemVer: {err}"),
                        );
                        d.data
                            .insert("tool_id".to_string(), json!(tool_id.as_str()));
                        d.data.insert("stdout".to_string(), json!(stdout.clone()));
                        d.data.insert("stderr".to_string(), json!(stderr.clone()));
                        diagnostics.push(d);
                    }
                    (_, Err(err)) => {
                        let mut d = Diagnostic::new(
                            "X07WASM_TOOLCHAIN_PROFILE_SCHEMA_INVALID",
                            Severity::Error,
                            Stage::Parse,
                            format!("invalid SemVer constraint for tool: {err}"),
                        );
                        d.data
                            .insert("tool_id".to_string(), json!(tool_id.as_str()));
                        d.data
                            .insert("constraint".to_string(), json!(req.constraint.clone()));
                        diagnostics.push(d);
                    }
                }
            }
        }
        Err(err) => {
            let mut d = Diagnostic::new(
                "X07WASM_TOOLCHAIN_TOOL_SPAWN_FAILED",
                severity,
                Stage::Run,
                format!("failed to spawn tool: {cmd}"),
            );
            d.data
                .insert("tool_id".to_string(), json!(tool_id.as_str()));
            d.data.insert("argv".to_string(), json!(argv.clone()));
            d.data
                .insert("os_error".to_string(), json!(err.to_string()));
            diagnostics.push(d);
        }
    }

    json!({
      "tool_id": tool_id.as_str(),
      "ok": ok,
      "required_constraint": req.constraint.clone(),
      "found_version": found_version,
      "cmd": cmd,
      "argv": argv,
      "stdout": stdout,
      "stderr": stderr,
    })
}

fn extract_version(probe: &VersionProbe, stdout: &str, stderr: &str) -> Result<String> {
    let re = Regex::new(&probe.regex).context("compile regex")?;
    let group = usize::try_from(probe.group).unwrap_or(1);

    let mut texts: Vec<&str> = Vec::new();
    match probe.source.as_str() {
        "stdout" => texts.push(stdout),
        "stderr" => texts.push(stderr),
        "either" => {
            texts.push(stdout);
            texts.push(stderr);
        }
        other => anyhow::bail!("unknown probe.source: {other:?}"),
    }

    for text in texts {
        if let Some(caps) = re.captures(text) {
            if let Some(m) = caps.get(group) {
                return Ok(m.as_str().to_string());
            }
        }
    }

    anyhow::bail!("regex did not match output")
}

fn load_index_doc(
    store: &SchemaStore,
    index_path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Option<ToolchainIndexDoc>, bool)> {
    let bytes = match std::fs::read(index_path) {
        Ok(v) => {
            meta.inputs.push(report::meta::FileDigest {
                path: index_path.display().to_string(),
                sha256: util::sha256_hex(&v),
                bytes_len: v.len() as u64,
            });
            v
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read toolchain index {}: {err}",
                    index_path.display()
                ),
            ));
            return Ok((None, false));
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("index JSON invalid: {err}"),
            ));
            return Ok((None, false));
        }
    };

    let diags = store.validate(
        "https://x07.io/spec/x07-arch.wasm.toolchain.index.schema.json",
        &doc_json,
    )?;
    if !diags.is_empty() {
        for d in diags {
            let mut dd = Diagnostic::new(
                "X07WASM_TOOLCHAIN_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                d.message,
            );
            dd.data = d.data;
            diagnostics.push(dd);
        }
        return Ok((None, false));
    }

    let parsed: ToolchainIndexDoc = match serde_json::from_value(doc_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_INDEX_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index doc: {err}"),
            ));
            return Ok((None, false));
        }
    };

    Ok((Some(parsed), true))
}

fn read_profile_doc(store: &SchemaStore, path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-wasm.toolchain.profile.schema.json",
        &doc_json,
    )?;
    if !diags.is_empty() {
        anyhow::bail!("profile schema invalid: {diags:?}");
    }
    Ok(doc_json)
}

fn load_profile_status(
    store: &SchemaStore,
    profile_path: &Path,
    expected_id: Option<&str>,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Value> {
    let mut id_matches_index = expected_id.is_none();

    let path_s = profile_path.display().to_string();

    let bytes = match std::fs::read(profile_path) {
        Ok(v) => {
            meta.inputs.push(report::meta::FileDigest {
                path: path_s.clone(),
                sha256: util::sha256_hex(&v),
                bytes_len: v.len() as u64,
            });
            v
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read toolchain profile {}: {err}",
                    profile_path.display()
                ),
            ));
            return Ok(json!({
              "ref": { "id": expected_id.unwrap_or("unknown"), "v": 1 },
              "path": path_s,
              "ok": false,
              "schema_version": null,
              "schema_valid": false,
              "id_matches_index": id_matches_index,
            }));
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_PROFILE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("profile JSON invalid: {err}"),
            ));
            return Ok(json!({
              "ref": { "id": expected_id.unwrap_or("unknown"), "v": 1 },
              "path": path_s,
              "ok": false,
              "schema_version": null,
              "schema_valid": false,
              "id_matches_index": id_matches_index,
            }));
        }
    };

    let schema_version = doc_json
        .get("schema_version")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let diags = store.validate(
        "https://x07.io/spec/x07-wasm.toolchain.profile.schema.json",
        &doc_json,
    )?;
    let schema_valid = diags.is_empty();
    if !schema_valid {
        for d in diags {
            let mut dd = Diagnostic::new(
                "X07WASM_TOOLCHAIN_PROFILE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                d.message,
            );
            dd.data = d.data;
            diagnostics.push(dd);
        }
    }

    let parsed: Option<ToolchainProfileDoc> = serde_json::from_value(doc_json)
        .ok()
        .filter(|_| schema_valid);

    let (id, v) = if let Some(p) = parsed.as_ref() {
        (p.id.clone(), p.v)
    } else {
        (expected_id.unwrap_or("unknown").to_string(), 1)
    };

    if let Some(exp) = expected_id {
        id_matches_index = exp == id;
        if !id_matches_index {
            diagnostics.push(Diagnostic::new(
                "X07WASM_TOOLCHAIN_PROFILE_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("profile.id mismatch: expected={exp:?} found={id:?}"),
            ));
        }
    }

    let ok = schema_valid && id_matches_index;

    Ok(json!({
      "ref": { "id": id, "v": v },
      "path": path_s,
      "ok": ok,
      "schema_version": schema_version,
      "schema_valid": schema_valid,
      "id_matches_index": id_matches_index,
    }))
}

fn truncate_4096(s: &str) -> String {
    if s.len() <= 4096 {
        return s.to_string();
    }
    s.chars().take(4096).collect()
}

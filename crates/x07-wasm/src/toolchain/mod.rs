use std::ffi::OsString;

use anyhow::Result;
use serde_json::json;

use crate::cli::{DoctorArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;

pub fn cmd_doctor(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    _args: DoctorArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let x07 = tool_first_line("x07", &["--version"]).ok();
    let clang = tool_first_line("clang", &["--version"]).ok();
    let wasm_ld = tool_first_line("wasm-ld", &["--version"]).ok();
    let wasmtime = tool_first_line("wasmtime", &["--version"]).ok();
    let wasm_tools = tool_first_line("wasm-tools", &["--version"]).ok();
    let wit_bindgen = tool_first_line("wit-bindgen", &["--version"]).ok();
    let wac = tool_first_line("wac", &["--version"]).ok();

    if x07.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_X07",
            Severity::Error,
            Stage::Run,
            "x07 not found on PATH".to_string(),
        ));
    }
    if clang.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_CLANG",
            Severity::Error,
            Stage::Run,
            "clang not found on PATH".to_string(),
        ));
    }
    if wasm_ld.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_WASM_LD",
            Severity::Error,
            Stage::Run,
            "wasm-ld not found on PATH".to_string(),
        ));
    }
    if wasmtime.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_WASMTIME",
            Severity::Error,
            Stage::Run,
            "wasmtime not found on PATH".to_string(),
        ));
    }
    if wasm_tools.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_WASM_TOOLS",
            Severity::Error,
            Stage::Run,
            "wasm-tools not found on PATH".to_string(),
        ));
    }
    if wit_bindgen.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_WIT_BINDGEN",
            Severity::Error,
            Stage::Run,
            "wit-bindgen not found on PATH".to_string(),
        ));
    }
    if wac.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_WAC",
            Severity::Error,
            Stage::Run,
            "wac not found on PATH".to_string(),
        ));
    }

    meta.tool.clang = clang.clone();
    meta.tool.wasm_ld = wasm_ld.clone();
    meta.tool.wasmtime = wasmtime.clone();

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code: u8 = if ok { 0 } else { 1 };

    let report_doc = json!({
      "schema_version": "x07.wasm.doctor.report@0.1.0",
      "command": "x07-wasm.doctor",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "x07": x07,
        "clang": clang,
        "wasm_ld": wasm_ld,
        "wasmtime": wasmtime,
        "wasm_tools": wasm_tools,
        "wit_bindgen": wit_bindgen,
        "wac": wac,
      }
    });

    let store = SchemaStore::new()?;
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

pub fn tool_first_line(name: &str, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(name).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("{name} failed");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("").trim().to_string())
}

pub fn x07_semver() -> Result<String> {
    let line = tool_first_line("x07", &["--version"])?;
    let mut it = line.split_whitespace();
    let _name = it.next();
    let v = it
        .next()
        .ok_or_else(|| anyhow::anyhow!("unable to parse x07 version: {line:?}"))?;
    Ok(v.to_string())
}

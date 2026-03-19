use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::diag::{Diagnostic, Severity, Stage};

pub struct CmdCapture {
    pub status: std::process::ExitStatus,
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn run_cmd_capture(program: &str, args: &[String]) -> Result<CmdCapture> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run: {program}"))?;
    let code = out.status.code().unwrap_or(1);
    Ok(CmdCapture {
        status: out.status,
        code,
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

fn project_has_dependencies(project_path: &Path) -> Result<bool> {
    let bytes = std::fs::read(project_path)
        .with_context(|| format!("read project manifest: {}", project_path.display()))?;
    let doc: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse project manifest JSON: {}", project_path.display()))?;
    Ok(doc
        .get("dependencies")
        .and_then(Value::as_array)
        .is_some_and(|deps| !deps.is_empty()))
}

pub fn ensure_x07_project_deps_hydrated(project_path: &Path) -> Result<bool> {
    if !project_has_dependencies(project_path)? {
        return Ok(false);
    }

    let project_arg = project_path.display().to_string();
    let check_args = vec![
        "pkg".to_string(),
        "lock".to_string(),
        "--project".to_string(),
        project_arg.clone(),
        "--check".to_string(),
        "--offline".to_string(),
    ];
    let check_out = run_cmd_capture("x07", &check_args)?;
    if check_out.status.success() {
        return Ok(false);
    }

    let sync_args = vec![
        "pkg".to_string(),
        "lock".to_string(),
        "--project".to_string(),
        project_arg,
    ];
    let sync_out = run_cmd_capture("x07", &sync_args)?;
    if sync_out.status.success() {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&sync_out.stderr);
    let stdout = String::from_utf8_lossy(&sync_out.stdout);
    anyhow::bail!(
        "x07 pkg lock failed (code={}): stderr={:?} stdout={:?}",
        sync_out.code,
        stderr,
        stdout
    );
}

pub fn diag_cmd_failed(
    code: &str,
    stage: Stage,
    cmd: &str,
    exit: i32,
    stderr: &[u8],
) -> Diagnostic {
    let mut d = Diagnostic::new(
        code,
        Severity::Error,
        stage,
        format!("{cmd} failed (code={exit})"),
    );
    d.data.insert(
        "stderr".to_string(),
        json!(String::from_utf8_lossy(stderr).to_string()),
    );
    d
}

pub fn diag_cmd_spawn_failed(
    code: &str,
    stage: Stage,
    cmd: &str,
    err: &anyhow::Error,
) -> Diagnostic {
    let mut d = Diagnostic::new(code, Severity::Error, stage, format!("{cmd} spawn failed"));
    d.data
        .insert("error".to_string(), json!(format!("{err:#}")));
    d
}

pub fn diag_io_failed(code: &str, stage: Stage, msg: String, err: &anyhow::Error) -> Diagnostic {
    let mut d = Diagnostic::new(code, Severity::Error, stage, msg);
    d.data
        .insert("error".to_string(), json!(format!("{err:#}")));
    d
}

pub fn emit_scaffold_report(command: &str, message: &str) -> Result<u8> {
    let report = json!({
        "schema_version": "x07.wasm.scaffold.report@0.1.0",
        "ok": true,
        "command": command,
        "status": "scaffolded",
        "message": message
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

use anyhow::{Context, Result};
use serde_json::json;

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

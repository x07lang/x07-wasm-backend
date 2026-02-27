use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;

const CLI_PARSE_REPORT_SCHEMA_ID: &str =
    "https://x07.io/spec/x07-wasm.cli.parse.report.schema.json";

#[derive(Debug, Clone, Default)]
pub struct MachineHint {
    pub wants_json: bool,
    pub report_out: Option<PathBuf>,
    pub quiet_json: bool,
}

impl MachineHint {
    pub fn should_emit(&self) -> bool {
        self.wants_json || self.report_out.is_some()
    }

    pub fn from_argv(argv: &[OsString]) -> Self {
        let mut out = Self::default();

        let mut it = argv.iter().skip(1).peekable();
        while let Some(arg) = it.next() {
            let s = arg.to_string_lossy();

            if s == "--json" || s == "--report-json" {
                out.wants_json = true;
                if let Some(next) = it.peek() {
                    if next.to_string_lossy() == "pretty" {
                        let _ = it.next();
                    }
                }
                continue;
            }

            if s.starts_with("--json=") || s.starts_with("--report-json=") {
                out.wants_json = true;
                continue;
            }

            if s == "--report-out" {
                if let Some(next) = it.next() {
                    out.report_out = Some(PathBuf::from(next.clone()));
                }
                continue;
            }
            if let Some(v) = s.strip_prefix("--report-out=") {
                out.report_out = Some(PathBuf::from(v.to_string()));
                continue;
            }

            if s == "--quiet-json" {
                out.quiet_json = true;
                continue;
            }
        }

        out
    }
}

pub fn emit_cli_parse_report(
    raw_argv: &[OsString],
    hint: &MachineHint,
    started: Instant,
    stage: &'static str,
    message: String,
    exit_code: u8,
) -> Result<u8> {
    if !hint.should_emit() {
        return Ok(exit_code);
    }

    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut d = Diagnostic::new(
        "X07WASM_CLI_ARGS_INVALID",
        Severity::Error,
        Stage::Parse,
        truncate_utf8(message, 16 * 1024),
    );
    d.data.insert("stage".to_string(), json!(stage));

    let report_doc = json!({
      "schema_version": "x07.wasm.cli.parse.report@0.1.0",
      "command": "x07-wasm.cli.parse",
      "ok": false,
      "exit_code": exit_code,
      "diagnostics": [d],
      "meta": meta,
      "result": {
        "stage": stage,
      },
    });

    let diags = store.validate(CLI_PARSE_REPORT_SCHEMA_ID, &report_doc)?;
    if !diags.is_empty() {
        anyhow::bail!(
            "internal error: report failed schema validation for {CLI_PARSE_REPORT_SCHEMA_ID:?}: {diags:?}"
        );
    }

    let bytes = report::canon::canonical_json_bytes(&report_doc)?;

    // For interactive Clap parse errors, default to not printing JSON to stdout unless the caller
    // explicitly opted into JSON mode.
    let quiet_json = if hint.wants_json {
        hint.quiet_json
    } else {
        true
    };
    report::schema::emit_bytes(&bytes, hint.report_out.as_deref(), quiet_json)?;

    Ok(exit_code)
}

fn truncate_utf8(mut s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s
}

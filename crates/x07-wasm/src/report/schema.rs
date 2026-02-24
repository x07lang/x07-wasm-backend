use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::{MachineArgs, Scope};
use crate::report::machine::{self, JsonMode};

const BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.build.report.schema.json");
const RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.run.report.schema.json");
const PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.profile.validate.report.schema.json");
const CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.cli.specrows.check.report.schema.json");
const DOCTOR_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.doctor.report.schema.json");

pub fn emit_schema_or_id(machine: &MachineArgs, scope: Scope) -> Result<u8> {
    let mode = machine::json_mode(machine).map_err(anyhow::Error::msg)?;
    if mode != JsonMode::Off {
        anyhow::bail!("--json cannot be combined with --json-schema/--json-schema-id");
    }

    if machine.json_schema {
        let bytes = schema_bytes_for_scope(scope)?;
        emit_bytes(bytes, machine.report_out.as_deref(), machine.quiet_json)?;
        return Ok(0);
    }

    if machine.json_schema_id {
        let id = schema_version_for_scope(scope);
        let mut bytes = id.as_bytes().to_vec();
        bytes.push(b'\n');
        emit_bytes(&bytes, machine.report_out.as_deref(), machine.quiet_json)?;
        return Ok(0);
    }

    Ok(0)
}

fn schema_bytes_for_scope(scope: Scope) -> Result<&'static [u8]> {
    match scope {
        Scope::Build => Ok(BUILD_REPORT_SCHEMA_BYTES),
        Scope::Run => Ok(RUN_REPORT_SCHEMA_BYTES),
        Scope::Doctor => Ok(DOCTOR_REPORT_SCHEMA_BYTES),
        Scope::ProfileValidate => Ok(PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::CliSpecrowsCheck => Ok(CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES),
    }
}

fn schema_version_for_scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Build => "x07.wasm.build.report@0.1.0",
        Scope::Run => "x07.wasm.run.report@0.1.0",
        Scope::Doctor => "x07.wasm.doctor.report@0.1.0",
        Scope::ProfileValidate => "x07.wasm.profile.validate.report@0.1.0",
        Scope::CliSpecrowsCheck => "x07.wasm.cli.specrows.check.report@0.1.0",
    }
}

pub fn emit_bytes(bytes: &[u8], report_out: Option<&Path>, quiet_json: bool) -> Result<()> {
    if let Some(path) = report_out {
        if path.to_string_lossy().trim() == "-" {
            anyhow::bail!("--report-out '-' is not supported (stdout is reserved for the report)");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir: {}", parent.display()))?;
        }
        std::fs::write(path, bytes).with_context(|| format!("write: {}", path.display()))?;
    }

    if !quiet_json {
        std::io::stdout().write_all(bytes).context("write stdout")?;
    }

    Ok(())
}

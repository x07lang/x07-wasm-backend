use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::{MachineArgs, Scope};
use crate::report::machine::{self, JsonMode};

const BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.build.report.schema.json");
const RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.run.report.schema.json");
const SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.serve.report.schema.json");
const PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.profile.validate.report.schema.json");
const WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.web_ui.contracts.validate.report.schema.json"
);
const WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.profile.validate.report.schema.json");
const WEB_UI_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.build.report.schema.json");
const WEB_UI_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.serve.report.schema.json");
const WEB_UI_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.test.report.schema.json");
const WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.web_ui.regress.from.incident.report.schema.json"
);
const APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.profile.validate.report.schema.json");
const APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.contracts.validate.report.schema.json");
const APP_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.build.report.schema.json");
const APP_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.serve.report.schema.json");
const APP_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.test.report.schema.json");
const APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.app.regress.from_incident.report.schema.json"
);
const CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.cli.specrows.check.report.schema.json");
const DOCTOR_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.doctor.report.schema.json");
const WIT_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.wit.validate.report.schema.json");
const COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.component.profile.validate.report.schema.json"
);
const COMPONENT_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.build.report.schema.json");
const COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.compose.report.schema.json");
const COMPONENT_TARGETS_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.targets.report.schema.json");
const COMPONENT_RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.run.report.schema.json");

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
        Scope::Serve => Ok(SERVE_REPORT_SCHEMA_BYTES),
        Scope::Doctor => Ok(DOCTOR_REPORT_SCHEMA_BYTES),
        Scope::WitValidate => Ok(WIT_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::ComponentProfileValidate => Ok(COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::ComponentBuild => Ok(COMPONENT_BUILD_REPORT_SCHEMA_BYTES),
        Scope::ComponentCompose => Ok(COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES),
        Scope::ComponentTargets => Ok(COMPONENT_TARGETS_REPORT_SCHEMA_BYTES),
        Scope::ComponentRun => Ok(COMPONENT_RUN_REPORT_SCHEMA_BYTES),
        Scope::ProfileValidate => Ok(PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::WebUiContractsValidate => Ok(WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::WebUiProfileValidate => Ok(WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::WebUiBuild => Ok(WEB_UI_BUILD_REPORT_SCHEMA_BYTES),
        Scope::WebUiServe => Ok(WEB_UI_SERVE_REPORT_SCHEMA_BYTES),
        Scope::WebUiTest => Ok(WEB_UI_TEST_REPORT_SCHEMA_BYTES),
        Scope::WebUiRegressFromIncident => Ok(WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::AppContractsValidate => Ok(APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::AppProfileValidate => Ok(APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::AppBuild => Ok(APP_BUILD_REPORT_SCHEMA_BYTES),
        Scope::AppServe => Ok(APP_SERVE_REPORT_SCHEMA_BYTES),
        Scope::AppTest => Ok(APP_TEST_REPORT_SCHEMA_BYTES),
        Scope::AppRegressFromIncident => Ok(APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::CliSpecrowsCheck => Ok(CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES),
    }
}

fn schema_version_for_scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Build => "x07.wasm.build.report@0.1.0",
        Scope::Run => "x07.wasm.run.report@0.1.0",
        Scope::Serve => "x07.wasm.serve.report@0.1.0",
        Scope::Doctor => "x07.wasm.doctor.report@0.1.0",
        Scope::WitValidate => "x07.wasm.wit.validate.report@0.1.0",
        Scope::ComponentProfileValidate => "x07.wasm.component.profile.validate.report@0.1.0",
        Scope::ComponentBuild => "x07.wasm.component.build.report@0.1.0",
        Scope::ComponentCompose => "x07.wasm.component.compose.report@0.1.0",
        Scope::ComponentTargets => "x07.wasm.component.targets.report@0.1.0",
        Scope::ComponentRun => "x07.wasm.component.run.report@0.1.0",
        Scope::ProfileValidate => "x07.wasm.profile.validate.report@0.1.0",
        Scope::WebUiContractsValidate => "x07.wasm.web_ui.contracts.validate.report@0.1.0",
        Scope::WebUiProfileValidate => "x07.wasm.web_ui.profile.validate.report@0.1.0",
        Scope::WebUiBuild => "x07.wasm.web_ui.build.report@0.1.0",
        Scope::WebUiServe => "x07.wasm.web_ui.serve.report@0.1.0",
        Scope::WebUiTest => "x07.wasm.web_ui.test.report@0.1.0",
        Scope::WebUiRegressFromIncident => "x07.wasm.web_ui.regress.from.incident.report@0.1.0",
        Scope::AppContractsValidate => "x07.wasm.app.contracts.validate.report@0.1.0",
        Scope::AppProfileValidate => "x07.wasm.app.profile.validate.report@0.1.0",
        Scope::AppBuild => "x07.wasm.app.build.report@0.1.0",
        Scope::AppServe => "x07.wasm.app.serve.report@0.1.0",
        Scope::AppTest => "x07.wasm.app.test.report@0.1.0",
        Scope::AppRegressFromIncident => "x07.wasm.app.regress.from_incident.report@0.1.0",
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

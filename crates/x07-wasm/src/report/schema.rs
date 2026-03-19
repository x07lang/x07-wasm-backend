use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::{MachineArgs, Scope};
use crate::report::machine::{self, JsonMode};

const BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.build.report.schema.json");
const RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.run.report.schema.json");
const SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.serve.report.schema.json");
const TOOLCHAIN_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.toolchain.validate.report.schema.json");
const OPS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.ops.validate.report.schema.json");
const CAPS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.caps.validate.report.schema.json");
const POLICY_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.policy.validate.report.schema.json");
const SLO_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.slo.validate.report.schema.json");
const SLO_EVAL_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.slo.eval.report.schema.json");
const DEPLOY_PLAN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.deploy.plan.report.schema.json");
const PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.provenance.attest.report.schema.json");
const PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.provenance.verify.report.schema.json");
const PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.profile.validate.report.schema.json");
const DEVICE_INDEX_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.index.validate.report.schema.json");
const DEVICE_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.profile.validate.report.schema.json");
const DEVICE_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.build.report.schema.json");
const DEVICE_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.verify.report.schema.json");
const DEVICE_PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.provenance.attest.report.schema.json");
const DEVICE_PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.provenance.verify.report.schema.json");
const DEVICE_RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.run.report.schema.json");
const DEVICE_PACKAGE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.package.report.schema.json");
const DEVICE_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.device.regress.from_incident.report.schema.json");
const WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.contracts.validate.report.schema.json");
const WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.profile.validate.report.schema.json");
const WEB_UI_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.build.report.schema.json");
const WEB_UI_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.serve.report.schema.json");
const WEB_UI_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.test.report.schema.json");
const WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.web_ui.regress.from.incident.report.schema.json");
const APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.profile.validate.report.schema.json");
const APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.contracts.validate.report.schema.json");
const APP_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.build.report.schema.json");
const APP_PACK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.pack.report.schema.json");
const APP_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.verify.report.schema.json");
const APP_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.serve.report.schema.json");
const APP_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.test.report.schema.json");
const APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.app.regress.from_incident.report.schema.json");
const SCAFFOLD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.scaffold.report.schema.json");
const HTTP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.http.contracts.validate.report.schema.json");
const HTTP_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.http.serve.report.schema.json");
const HTTP_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.http.test.report.schema.json");
const HTTP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.http.regress.from.incident.report.schema.json");
const CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.cli.specrows.check.report.schema.json");
const DOCTOR_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.doctor.report.schema.json");
const WIT_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.wit.validate.report.schema.json");
const COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.component.profile.validate.report.schema.json");
const COMPONENT_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.component.build.report.schema.json");
const COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.component.compose.report.schema.json");
const COMPONENT_TARGETS_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.component.targets.report.schema.json");
const COMPONENT_RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../spec/schemas/x07-wasm.component.run.report.schema.json");

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
        Scope::ToolchainValidate => Ok(TOOLCHAIN_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::OpsValidate => Ok(OPS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::CapsValidate => Ok(CAPS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::PolicyValidate => Ok(POLICY_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::SloValidate => Ok(SLO_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::SloEval => Ok(SLO_EVAL_REPORT_SCHEMA_BYTES),
        Scope::DeployPlan => Ok(DEPLOY_PLAN_REPORT_SCHEMA_BYTES),
        Scope::ProvenanceAttest => Ok(PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES),
        Scope::ProvenanceVerify => Ok(PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES),
        Scope::WitValidate => Ok(WIT_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::ComponentProfileValidate => Ok(COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::ComponentBuild => Ok(COMPONENT_BUILD_REPORT_SCHEMA_BYTES),
        Scope::ComponentCompose => Ok(COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES),
        Scope::ComponentTargets => Ok(COMPONENT_TARGETS_REPORT_SCHEMA_BYTES),
        Scope::ComponentRun => Ok(COMPONENT_RUN_REPORT_SCHEMA_BYTES),
        Scope::ProfileValidate => Ok(PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::DeviceIndexValidate => Ok(DEVICE_INDEX_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::DeviceProfileValidate => Ok(DEVICE_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::DeviceBuild => Ok(DEVICE_BUILD_REPORT_SCHEMA_BYTES),
        Scope::DeviceVerify => Ok(DEVICE_VERIFY_REPORT_SCHEMA_BYTES),
        Scope::DeviceProvenanceAttest => Ok(DEVICE_PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES),
        Scope::DeviceProvenanceVerify => Ok(DEVICE_PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES),
        Scope::DeviceRun => Ok(DEVICE_RUN_REPORT_SCHEMA_BYTES),
        Scope::DevicePackage => Ok(DEVICE_PACKAGE_REPORT_SCHEMA_BYTES),
        Scope::DeviceRegressFromIncident => Ok(DEVICE_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::WebUiContractsValidate => Ok(WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::WebUiProfileValidate => Ok(WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::WebUiBuild => Ok(WEB_UI_BUILD_REPORT_SCHEMA_BYTES),
        Scope::WebUiServe => Ok(WEB_UI_SERVE_REPORT_SCHEMA_BYTES),
        Scope::WebUiTest => Ok(WEB_UI_TEST_REPORT_SCHEMA_BYTES),
        Scope::WebUiRegressFromIncident => Ok(WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::AppContractsValidate => Ok(APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::AppProfileValidate => Ok(APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::AppBuild => Ok(APP_BUILD_REPORT_SCHEMA_BYTES),
        Scope::AppPack => Ok(APP_PACK_REPORT_SCHEMA_BYTES),
        Scope::AppVerify => Ok(APP_VERIFY_REPORT_SCHEMA_BYTES),
        Scope::AppServe => Ok(APP_SERVE_REPORT_SCHEMA_BYTES),
        Scope::AppTest => Ok(APP_TEST_REPORT_SCHEMA_BYTES),
        Scope::AppRegressFromIncident => Ok(APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::WorkloadBuild
        | Scope::WorkloadPack
        | Scope::WorkloadInspect
        | Scope::WorkloadContractsValidate
        | Scope::TopologyPreview
        | Scope::BindingResolve => Ok(SCAFFOLD_REPORT_SCHEMA_BYTES),
        Scope::HttpContractsValidate => Ok(HTTP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES),
        Scope::HttpServe => Ok(HTTP_SERVE_REPORT_SCHEMA_BYTES),
        Scope::HttpTest => Ok(HTTP_TEST_REPORT_SCHEMA_BYTES),
        Scope::HttpRegressFromIncident => Ok(HTTP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES),
        Scope::CliSpecrowsCheck => Ok(CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES),
    }
}

fn schema_version_for_scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Build => "x07.wasm.build.report@0.2.0",
        Scope::Run => "x07.wasm.run.report@0.1.0",
        Scope::Serve => "x07.wasm.serve.report@0.1.0",
        Scope::Doctor => "x07.wasm.doctor.report@0.1.0",
        Scope::ToolchainValidate => "x07.wasm.toolchain.validate.report@0.1.0",
        Scope::OpsValidate => "x07.wasm.ops.validate.report@0.1.0",
        Scope::CapsValidate => "x07.wasm.caps.validate.report@0.1.0",
        Scope::PolicyValidate => "x07.wasm.policy.validate.report@0.1.0",
        Scope::SloValidate => "x07.wasm.slo.validate.report@0.1.0",
        Scope::SloEval => "x07.wasm.slo.eval.report@0.1.0",
        Scope::DeployPlan => "x07.wasm.deploy.plan.report@0.1.0",
        Scope::ProvenanceAttest => "x07.wasm.provenance.attest.report@0.1.0",
        Scope::ProvenanceVerify => "x07.wasm.provenance.verify.report@0.1.0",
        Scope::WitValidate => "x07.wasm.wit.validate.report@0.1.0",
        Scope::ComponentProfileValidate => "x07.wasm.component.profile.validate.report@0.1.0",
        Scope::ComponentBuild => "x07.wasm.component.build.report@0.1.0",
        Scope::ComponentCompose => "x07.wasm.component.compose.report@0.1.0",
        Scope::ComponentTargets => "x07.wasm.component.targets.report@0.1.0",
        Scope::ComponentRun => "x07.wasm.component.run.report@0.1.0",
        Scope::ProfileValidate => "x07.wasm.profile.validate.report@0.1.0",
        Scope::DeviceIndexValidate => "x07.wasm.device.index.validate.report@0.1.0",
        Scope::DeviceProfileValidate => "x07.wasm.device.profile.validate.report@0.1.0",
        Scope::DeviceBuild => "x07.wasm.device.build.report@0.1.0",
        Scope::DeviceVerify => "x07.wasm.device.verify.report@0.2.0",
        Scope::DeviceProvenanceAttest => "x07.wasm.device.provenance.attest.report@0.1.0",
        Scope::DeviceProvenanceVerify => "x07.wasm.device.provenance.verify.report@0.1.0",
        Scope::DeviceRun => "x07.wasm.device.run.report@0.1.0",
        Scope::DevicePackage => "x07.wasm.device.package.report@0.2.0",
        Scope::DeviceRegressFromIncident => "x07.wasm.device.regress.from_incident.report@0.2.0",
        Scope::WebUiContractsValidate => "x07.wasm.web_ui.contracts.validate.report@0.1.0",
        Scope::WebUiProfileValidate => "x07.wasm.web_ui.profile.validate.report@0.1.0",
        Scope::WebUiBuild => "x07.wasm.web_ui.build.report@0.1.0",
        Scope::WebUiServe => "x07.wasm.web_ui.serve.report@0.1.0",
        Scope::WebUiTest => "x07.wasm.web_ui.test.report@0.1.0",
        Scope::WebUiRegressFromIncident => "x07.wasm.web_ui.regress.from.incident.report@0.1.0",
        Scope::AppContractsValidate => "x07.wasm.app.contracts.validate.report@0.1.0",
        Scope::AppProfileValidate => "x07.wasm.app.profile.validate.report@0.1.0",
        Scope::AppBuild => "x07.wasm.app.build.report@0.1.0",
        Scope::AppPack => "x07.wasm.app.pack.report@0.1.0",
        Scope::AppVerify => "x07.wasm.app.verify.report@0.1.0",
        Scope::AppServe => "x07.wasm.app.serve.report@0.1.0",
        Scope::AppTest => "x07.wasm.app.test.report@0.1.0",
        Scope::AppRegressFromIncident => "x07.wasm.app.regress.from_incident.report@0.1.0",
        Scope::WorkloadBuild
        | Scope::WorkloadPack
        | Scope::WorkloadInspect
        | Scope::WorkloadContractsValidate
        | Scope::TopologyPreview
        | Scope::BindingResolve => "x07.wasm.scaffold.report@0.1.0",
        Scope::HttpContractsValidate => "x07.wasm.http.contracts.validate.report@0.1.0",
        Scope::HttpServe => "x07.wasm.http.serve.report@0.1.0",
        Scope::HttpTest => "x07.wasm.http.test.report@0.1.0",
        Scope::HttpRegressFromIncident => "x07.wasm.http.regress.from.incident.report@0.1.0",
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

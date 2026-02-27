use std::collections::BTreeMap;

use anyhow::{Context, Result};
use jsonschema::{Draft, Resource};
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::machine::{self, JsonMode};

const X07DIAG_SCHEMA_BYTES: &[u8] = include_bytes!("../../../../spec/schemas/x07diag.schema.json");
const X07CLI_SPECROWS_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07cli.specrows.schema.json");

const X07_ARCH_WASM_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.wasm.index.schema.json");
const X07_ARCH_WIT_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.wit.index.schema.json");
const X07_ARCH_WASM_COMPONENT_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.wasm.component.index.schema.json");
const X07_ARCH_WEB_UI_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.web_ui.index.schema.json");
const X07_ARCH_APP_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.app.index.schema.json");
const X07_ARCH_APP_OPS_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.app.ops.index.schema.json");
const X07_APP_OPS_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.ops.profile.schema.json");
const X07_APP_CAPABILITIES_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.capabilities.schema.json");
const X07_POLICY_CARD_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-policy.card.schema.json");
const X07_SLO_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-slo.profile.schema.json");
const X07_WASM_CAPS_EVIDENCE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.caps.evidence.schema.json");
const X07_METRICS_SNAPSHOT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-metrics.snapshot.schema.json");
const X07_DEPLOY_PLAN_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-deploy.plan.schema.json");
const X07_PROVENANCE_SLSA_ATTESTATION_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-provenance.slsa.attestation.schema.json");
const X07_ARCH_WASM_TOOLCHAIN_INDEX_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-arch.wasm.toolchain.index.schema.json");
const X07_WASM_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.profile.schema.json");
const X07_WASM_RUNTIME_LIMITS_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.runtime.limits.schema.json");
const X07_WASM_COMPONENT_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.profile.schema.json");
const X07_WASM_COMPONENT_ARTIFACT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.artifact.schema.json");
const X07_WASM_ARTIFACT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.artifact.schema.json");

const X07_WASM_TOOLCHAIN_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.toolchain.profile.schema.json");

const X07_WEB_UI_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.profile.schema.json");
const X07_WEB_UI_DISPATCH_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.dispatch.schema.json");
const X07_WEB_UI_TREE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.tree.schema.json");
const X07_WEB_UI_PATCHSET_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.patchset.schema.json");
const X07_WEB_UI_FRAME_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.frame.schema.json");
const X07_WEB_UI_EFFECT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.effect.schema.json");
const X07_WEB_UI_TRACE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-web_ui.trace.schema.json");

const X07_APP_PROFILE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.profile.schema.json");
const X07_APP_BUNDLE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.bundle.schema.json");
const X07_APP_PACK_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.pack.schema.json");
const X07_HTTP_REQUEST_ENVELOPE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.request.envelope.schema.json");
const X07_HTTP_RESPONSE_ENVELOPE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.response.envelope.schema.json");
const X07_HTTP_EFFECT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.effect.schema.json");
const X07_HTTP_DISPATCH_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.dispatch.schema.json");
const X07_HTTP_FRAME_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.frame.schema.json");
const X07_HTTP_TRACE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-http.trace.schema.json");
const X07_APP_TRACE_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-app.trace.schema.json");

const X07_WASM_TOOLCHAIN_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.toolchain.validate.report.schema.json");
const X07_WASM_OPS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.ops.validate.report.schema.json");
const X07_WASM_CAPS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.caps.validate.report.schema.json");
const X07_WASM_POLICY_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.policy.validate.report.schema.json");
const X07_WASM_SLO_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.slo.validate.report.schema.json");
const X07_WASM_SLO_EVAL_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.slo.eval.report.schema.json");
const X07_WASM_DEPLOY_PLAN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.deploy.plan.report.schema.json");
const X07_WASM_PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.provenance.attest.report.schema.json");
const X07_WASM_PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.provenance.verify.report.schema.json");
const X07_WASM_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.build.report.schema.json");
const X07_WASM_RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.run.report.schema.json");
const X07_WASM_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.serve.report.schema.json");
const X07_WASM_COMPONENT_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.build.report.schema.json");
const X07_WASM_COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.compose.report.schema.json");
const X07_WASM_COMPONENT_TARGETS_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.targets.report.schema.json");
const X07_WASM_COMPONENT_RUN_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.component.run.report.schema.json");
const X07_WASM_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.profile.validate.report.schema.json");
const X07_WASM_WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.web_ui.contracts.validate.report.schema.json"
);
const X07_WASM_WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.profile.validate.report.schema.json");
const X07_WASM_WEB_UI_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.build.report.schema.json");
const X07_WASM_WEB_UI_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.serve.report.schema.json");
const X07_WASM_WEB_UI_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.web_ui.test.report.schema.json");
const X07_WASM_WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.web_ui.regress.from.incident.report.schema.json"
);
const X07_WASM_APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.profile.validate.report.schema.json");
const X07_WASM_APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.contracts.validate.report.schema.json");
const X07_WASM_APP_BUILD_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.build.report.schema.json");
const X07_WASM_APP_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.serve.report.schema.json");
const X07_WASM_APP_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.test.report.schema.json");
const X07_WASM_APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.app.regress.from_incident.report.schema.json"
);
const X07_WASM_APP_PACK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.pack.report.schema.json");
const X07_WASM_APP_VERIFY_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.app.verify.report.schema.json");

const X07_WASM_HTTP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.http.contracts.validate.report.schema.json");
const X07_WASM_HTTP_SERVE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.http.serve.report.schema.json");
const X07_WASM_HTTP_TEST_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.http.test.report.schema.json");
const X07_WASM_HTTP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.http.regress.from.incident.report.schema.json"
);
const X07_WASM_CLI_PARSE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.cli.parse.report.schema.json");
const X07_WASM_CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.cli.specrows.check.report.schema.json");
const X07_WASM_DOCTOR_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.doctor.report.schema.json");
const X07_WASM_WIT_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] =
    include_bytes!("../../../../spec/schemas/x07-wasm.wit.validate.report.schema.json");
const X07_WASM_COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES: &[u8] = include_bytes!(
    "../../../../spec/schemas/x07-wasm.component.profile.validate.report.schema.json"
);

#[derive(Debug, Clone)]
pub struct SchemaStore {
    by_id: BTreeMap<String, Value>,
}

impl SchemaStore {
    pub fn new() -> Result<Self> {
        let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
        for bytes in [
            X07DIAG_SCHEMA_BYTES,
            X07CLI_SPECROWS_SCHEMA_BYTES,
            X07_ARCH_WASM_INDEX_SCHEMA_BYTES,
            X07_ARCH_WIT_INDEX_SCHEMA_BYTES,
            X07_ARCH_WASM_COMPONENT_INDEX_SCHEMA_BYTES,
            X07_ARCH_WEB_UI_INDEX_SCHEMA_BYTES,
            X07_ARCH_APP_INDEX_SCHEMA_BYTES,
            X07_ARCH_APP_OPS_INDEX_SCHEMA_BYTES,
            X07_ARCH_WASM_TOOLCHAIN_INDEX_SCHEMA_BYTES,
            X07_WASM_PROFILE_SCHEMA_BYTES,
            X07_WASM_RUNTIME_LIMITS_SCHEMA_BYTES,
            X07_WASM_COMPONENT_PROFILE_SCHEMA_BYTES,
            X07_WASM_COMPONENT_ARTIFACT_SCHEMA_BYTES,
            X07_WASM_ARTIFACT_SCHEMA_BYTES,
            X07_WASM_TOOLCHAIN_PROFILE_SCHEMA_BYTES,
            X07_WEB_UI_PROFILE_SCHEMA_BYTES,
            X07_WEB_UI_DISPATCH_SCHEMA_BYTES,
            X07_WEB_UI_TREE_SCHEMA_BYTES,
            X07_WEB_UI_PATCHSET_SCHEMA_BYTES,
            X07_WEB_UI_FRAME_SCHEMA_BYTES,
            X07_WEB_UI_EFFECT_SCHEMA_BYTES,
            X07_WEB_UI_TRACE_SCHEMA_BYTES,
            X07_APP_PROFILE_SCHEMA_BYTES,
            X07_APP_BUNDLE_SCHEMA_BYTES,
            X07_APP_PACK_SCHEMA_BYTES,
            X07_HTTP_REQUEST_ENVELOPE_SCHEMA_BYTES,
            X07_HTTP_RESPONSE_ENVELOPE_SCHEMA_BYTES,
            X07_HTTP_EFFECT_SCHEMA_BYTES,
            X07_HTTP_DISPATCH_SCHEMA_BYTES,
            X07_HTTP_FRAME_SCHEMA_BYTES,
            X07_HTTP_TRACE_SCHEMA_BYTES,
            X07_APP_TRACE_SCHEMA_BYTES,
            X07_APP_OPS_PROFILE_SCHEMA_BYTES,
            X07_APP_CAPABILITIES_SCHEMA_BYTES,
            X07_POLICY_CARD_SCHEMA_BYTES,
            X07_SLO_PROFILE_SCHEMA_BYTES,
            X07_WASM_CAPS_EVIDENCE_SCHEMA_BYTES,
            X07_METRICS_SNAPSHOT_SCHEMA_BYTES,
            X07_DEPLOY_PLAN_SCHEMA_BYTES,
            X07_PROVENANCE_SLSA_ATTESTATION_SCHEMA_BYTES,
            X07_WASM_TOOLCHAIN_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_OPS_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_CAPS_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_POLICY_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_SLO_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_SLO_EVAL_REPORT_SCHEMA_BYTES,
            X07_WASM_DEPLOY_PLAN_REPORT_SCHEMA_BYTES,
            X07_WASM_PROVENANCE_ATTEST_REPORT_SCHEMA_BYTES,
            X07_WASM_PROVENANCE_VERIFY_REPORT_SCHEMA_BYTES,
            X07_WASM_BUILD_REPORT_SCHEMA_BYTES,
            X07_WASM_RUN_REPORT_SCHEMA_BYTES,
            X07_WASM_SERVE_REPORT_SCHEMA_BYTES,
            X07_WASM_COMPONENT_BUILD_REPORT_SCHEMA_BYTES,
            X07_WASM_COMPONENT_COMPOSE_REPORT_SCHEMA_BYTES,
            X07_WASM_COMPONENT_TARGETS_REPORT_SCHEMA_BYTES,
            X07_WASM_COMPONENT_RUN_REPORT_SCHEMA_BYTES,
            X07_WASM_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_BUILD_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_SERVE_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_TEST_REPORT_SCHEMA_BYTES,
            X07_WASM_WEB_UI_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_BUILD_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_SERVE_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_TEST_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_PACK_REPORT_SCHEMA_BYTES,
            X07_WASM_APP_VERIFY_REPORT_SCHEMA_BYTES,
            X07_WASM_HTTP_CONTRACTS_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_HTTP_SERVE_REPORT_SCHEMA_BYTES,
            X07_WASM_HTTP_TEST_REPORT_SCHEMA_BYTES,
            X07_WASM_HTTP_REGRESS_FROM_INCIDENT_REPORT_SCHEMA_BYTES,
            X07_WASM_CLI_PARSE_REPORT_SCHEMA_BYTES,
            X07_WASM_CLI_SPECROWS_CHECK_REPORT_SCHEMA_BYTES,
            X07_WASM_DOCTOR_REPORT_SCHEMA_BYTES,
            X07_WASM_WIT_VALIDATE_REPORT_SCHEMA_BYTES,
            X07_WASM_COMPONENT_PROFILE_VALIDATE_REPORT_SCHEMA_BYTES,
        ] {
            let doc: Value = serde_json::from_slice(bytes).context("parse embedded schema JSON")?;
            let id = doc
                .get("$id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("embedded schema missing $id"))?
                .to_string();
            by_id.insert(id, doc);
        }
        Ok(Self { by_id })
    }

    pub fn schema(&self, id: &str) -> Result<&Value> {
        self.by_id
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("missing embedded schema: {id:?}"))
    }

    pub fn validate(&self, schema_id: &str, instance: &Value) -> Result<Vec<Diagnostic>> {
        let schema = self.schema(schema_id)?.clone();
        let resources = self
            .by_id
            .iter()
            .map(|(id, v)| (id.clone(), Resource::from_contents(v.clone())));

        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .with_resources(resources)
            .build(&schema)
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        let mut diags = Vec::new();
        for err in validator.iter_errors(instance) {
            let mut d = Diagnostic::new(
                "X07WASM_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("{err}"),
            );
            d.data.insert(
                "instance_path".to_string(),
                json!(err.instance_path().to_string()),
            );
            d.data.insert(
                "schema_path".to_string(),
                json!(err.schema_path().to_string()),
            );
            diags.push(d);
        }
        Ok(diags)
    }

    pub fn validate_report_and_emit(
        &self,
        scope: Scope,
        machine: &MachineArgs,
        started: std::time::Instant,
        raw_argv: &[std::ffi::OsString],
        report_doc: Value,
    ) -> Result<()> {
        let schema_id = report_schema_id_for_scope(scope);
        let diags = self.validate(schema_id, &report_doc)?;
        if !diags.is_empty() {
            anyhow::bail!(
                "internal error: report failed schema validation for {schema_id:?}: {diags:?}"
            );
        }

        let mode = machine::json_mode(machine).map_err(anyhow::Error::msg)?;
        if mode == JsonMode::Off {
            return Ok(());
        }

        let bytes = match mode {
            JsonMode::Canon => report::canon::canonical_json_bytes(&report_doc)?,
            JsonMode::Pretty => report::canon::canonical_pretty_json_bytes(&report_doc)?,
            JsonMode::Off => unreachable!(),
        };

        report::schema::emit_bytes(&bytes, machine.report_out.as_deref(), machine.quiet_json)?;

        let _ = (started, raw_argv);
        Ok(())
    }
}

fn report_schema_id_for_scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Build => "https://x07.io/spec/x07-wasm.build.report.schema.json",
        Scope::Run => "https://x07.io/spec/x07-wasm.run.report.schema.json",
        Scope::Serve => "https://x07.io/spec/x07-wasm.serve.report.schema.json",
        Scope::Doctor => "https://x07.io/spec/x07-wasm.doctor.report.schema.json",
        Scope::ToolchainValidate => {
            "https://x07.io/spec/x07-wasm.toolchain.validate.report.schema.json"
        }
        Scope::OpsValidate => "https://x07.io/spec/x07-wasm.ops.validate.report.schema.json",
        Scope::CapsValidate => "https://x07.io/spec/x07-wasm.caps.validate.report.schema.json",
        Scope::PolicyValidate => "https://x07.io/spec/x07-wasm.policy.validate.report.schema.json",
        Scope::SloValidate => "https://x07.io/spec/x07-wasm.slo.validate.report.schema.json",
        Scope::SloEval => "https://x07.io/spec/x07-wasm.slo.eval.report.schema.json",
        Scope::DeployPlan => "https://x07.io/spec/x07-wasm.deploy.plan.report.schema.json",
        Scope::ProvenanceAttest => {
            "https://x07.io/spec/x07-wasm.provenance.attest.report.schema.json"
        }
        Scope::ProvenanceVerify => {
            "https://x07.io/spec/x07-wasm.provenance.verify.report.schema.json"
        }
        Scope::ProfileValidate => {
            "https://x07.io/spec/x07-wasm.profile.validate.report.schema.json"
        }
        Scope::WebUiContractsValidate => {
            "https://x07.io/spec/x07-wasm.web_ui.contracts.validate.report.schema.json"
        }
        Scope::WebUiProfileValidate => {
            "https://x07.io/spec/x07-wasm.web_ui.profile.validate.report.schema.json"
        }
        Scope::WebUiBuild => "https://x07.io/spec/x07-wasm.web_ui.build.report.schema.json",
        Scope::WebUiServe => "https://x07.io/spec/x07-wasm.web_ui.serve.report.schema.json",
        Scope::WebUiTest => "https://x07.io/spec/x07-wasm.web_ui.test.report.schema.json",
        Scope::WebUiRegressFromIncident => {
            "https://x07.io/spec/x07-wasm.web_ui.regress.from.incident.report.schema.json"
        }
        Scope::AppContractsValidate => {
            "https://x07.io/spec/x07-wasm.app.contracts.validate.report.schema.json"
        }
        Scope::AppProfileValidate => {
            "https://x07.io/spec/x07-wasm.app.profile.validate.report.schema.json"
        }
        Scope::AppBuild => "https://x07.io/spec/x07-wasm.app.build.report.schema.json",
        Scope::AppPack => "https://x07.io/spec/x07-wasm.app.pack.report.schema.json",
        Scope::AppVerify => "https://x07.io/spec/x07-wasm.app.verify.report.schema.json",
        Scope::AppServe => "https://x07.io/spec/x07-wasm.app.serve.report.schema.json",
        Scope::AppTest => "https://x07.io/spec/x07-wasm.app.test.report.schema.json",
        Scope::AppRegressFromIncident => {
            "https://x07.io/spec/x07-wasm.app.regress.from_incident.report.schema.json"
        }
        Scope::HttpContractsValidate => {
            "https://x07.io/spec/x07-wasm.http.contracts.validate.report.schema.json"
        }
        Scope::HttpServe => "https://x07.io/spec/x07-wasm.http.serve.report.schema.json",
        Scope::HttpTest => "https://x07.io/spec/x07-wasm.http.test.report.schema.json",
        Scope::HttpRegressFromIncident => {
            "https://x07.io/spec/x07-wasm.http.regress.from.incident.report.schema.json"
        }
        Scope::CliSpecrowsCheck => {
            "https://x07.io/spec/x07-wasm.cli.specrows.check.report.schema.json"
        }
        Scope::WitValidate => "https://x07.io/spec/x07-wasm.wit.validate.report.schema.json",
        Scope::ComponentProfileValidate => {
            "https://x07.io/spec/x07-wasm.component.profile.validate.report.schema.json"
        }
        Scope::ComponentBuild => "https://x07.io/spec/x07-wasm.component.build.report.schema.json",
        Scope::ComponentCompose => {
            "https://x07.io/spec/x07-wasm.component.compose.report.schema.json"
        }
        Scope::ComponentTargets => {
            "https://x07.io/spec/x07-wasm.component.targets.report.schema.json"
        }
        Scope::ComponentRun => "https://x07.io/spec/x07-wasm.component.run.report.schema.json",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_expected_schema_ids() {
        let store = SchemaStore::new().unwrap();
        for id in [
            "https://x07.io/spec/x07diag.schema.json",
            "https://x07.org/spec/x07cli.specrows.schema.json",
            "https://x07.io/spec/x07-arch.wasm.index.schema.json",
            "https://x07.io/spec/x07-arch.wit.index.schema.json",
            "https://x07.io/spec/x07-arch.wasm.component.index.schema.json",
            "https://x07.io/spec/x07-arch.web_ui.index.schema.json",
            "https://x07.io/spec/x07-arch.app.index.schema.json",
            "https://x07.io/spec/x07-arch.app.ops.index.schema.json",
            "https://x07.io/spec/x07-arch.wasm.toolchain.index.schema.json",
            "https://x07.io/spec/x07-wasm.profile.schema.json",
            "https://x07.io/spec/x07-wasm.runtime.limits.schema.json",
            "https://x07.io/spec/x07-wasm.component.profile.schema.json",
            "https://x07.io/spec/x07-wasm.component.artifact.schema.json",
            "https://x07.io/spec/x07-wasm.artifact.schema.json",
            "https://x07.io/spec/x07-wasm.toolchain.profile.schema.json",
            "https://x07.io/spec/x07-web_ui.profile.schema.json",
            "https://x07.io/spec/x07-web_ui.dispatch.schema.json",
            "https://x07.io/spec/x07-web_ui.tree.schema.json",
            "https://x07.io/spec/x07-web_ui.patchset.schema.json",
            "https://x07.io/spec/x07-web_ui.frame.schema.json",
            "https://x07.io/spec/x07-web_ui.effect.schema.json",
            "https://x07.io/spec/x07-web_ui.trace.schema.json",
            "https://x07.io/spec/x07-app.profile.schema.json",
            "https://x07.io/spec/x07-app.bundle.schema.json",
            "https://x07.io/spec/x07-app.pack.schema.json",
            "https://x07.io/spec/x07-app.ops.profile.schema.json",
            "https://x07.io/spec/x07-app.capabilities.schema.json",
            "https://x07.io/spec/x07-policy.card.schema.json",
            "https://x07.io/spec/x07-slo.profile.schema.json",
            "https://x07.io/spec/x07-wasm.caps.evidence.schema.json",
            "https://x07.io/spec/x07-metrics.snapshot.schema.json",
            "https://x07.io/spec/x07-deploy.plan.schema.json",
            "https://x07.io/spec/x07-provenance.slsa.attestation.schema.json",
            "https://x07.io/spec/x07-http.request.envelope.schema.json",
            "https://x07.io/spec/x07-http.response.envelope.schema.json",
            "https://x07.io/spec/x07-http.effect.schema.json",
            "https://x07.io/spec/x07-http.dispatch.schema.json",
            "https://x07.io/spec/x07-http.frame.schema.json",
            "https://x07.io/spec/x07-http.trace.schema.json",
            "https://x07.io/spec/x07-app.trace.schema.json",
            "https://x07.io/spec/x07-wasm.toolchain.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.ops.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.caps.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.policy.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.slo.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.slo.eval.report.schema.json",
            "https://x07.io/spec/x07-wasm.deploy.plan.report.schema.json",
            "https://x07.io/spec/x07-wasm.provenance.attest.report.schema.json",
            "https://x07.io/spec/x07-wasm.provenance.verify.report.schema.json",
            "https://x07.io/spec/x07-wasm.build.report.schema.json",
            "https://x07.io/spec/x07-wasm.run.report.schema.json",
            "https://x07.io/spec/x07-wasm.serve.report.schema.json",
            "https://x07.io/spec/x07-wasm.component.build.report.schema.json",
            "https://x07.io/spec/x07-wasm.component.compose.report.schema.json",
            "https://x07.io/spec/x07-wasm.component.targets.report.schema.json",
            "https://x07.io/spec/x07-wasm.component.run.report.schema.json",
            "https://x07.io/spec/x07-wasm.profile.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.contracts.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.profile.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.build.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.serve.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.test.report.schema.json",
            "https://x07.io/spec/x07-wasm.web_ui.regress.from.incident.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.profile.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.contracts.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.build.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.serve.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.test.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.regress.from_incident.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.pack.report.schema.json",
            "https://x07.io/spec/x07-wasm.app.verify.report.schema.json",
            "https://x07.io/spec/x07-wasm.http.contracts.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.http.serve.report.schema.json",
            "https://x07.io/spec/x07-wasm.http.test.report.schema.json",
            "https://x07.io/spec/x07-wasm.http.regress.from.incident.report.schema.json",
            "https://x07.io/spec/x07-wasm.cli.parse.report.schema.json",
            "https://x07.io/spec/x07-wasm.cli.specrows.check.report.schema.json",
            "https://x07.io/spec/x07-wasm.doctor.report.schema.json",
            "https://x07.io/spec/x07-wasm.wit.validate.report.schema.json",
            "https://x07.io/spec/x07-wasm.component.profile.validate.report.schema.json",
        ] {
            store.schema(id).unwrap();
        }
    }
}

use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{DeployPlanArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::ops::load_ops_profile_with_refs;
use crate::policy::engine::{apply_policy_cards, PolicyTarget};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_deploy_plan(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeployPlanArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let pack_bytes = match std::fs::read(&args.pack_manifest) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read pack manifest {}: {err}",
                    args.pack_manifest.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.out_dir,
                None,
                Vec::new(),
            );
        }
    };

    let pack_digest = report::meta::FileDigest {
        path: args.pack_manifest.display().to_string(),
        sha256: util::sha256_hex(&pack_bytes),
        bytes_len: pack_bytes.len() as u64,
    };
    meta.inputs.push(pack_digest.clone());

    let pack_doc: Value = match serde_json::from_slice(&pack_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("pack manifest JSON invalid: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.out_dir,
                None,
                Vec::new(),
            );
        }
    };

    let pack_schema_diags =
        store.validate("https://x07.io/spec/x07-app.pack.schema.json", &pack_doc)?;
    if !pack_schema_diags.is_empty() {
        for dd in pack_schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    let pack_profile_id = pack_doc
        .get("profile_id")
        .and_then(Value::as_str)
        .unwrap_or("app");

    let loaded_ops = load_ops_profile_with_refs(&store, &args.ops, &mut meta, &mut diagnostics)?;
    let Some(loaded_ops) = loaded_ops else {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    };

    let ops_digest = loaded_ops.ops.digest.clone();
    let policy_card_digests: Vec<report::meta::FileDigest> = loaded_ops
        .policy_cards
        .iter()
        .map(|c| c.digest.clone())
        .collect();
    let slo_digest = loaded_ops.slo_profile.as_ref().map(|s| s.digest.clone());

    // Enforce: analysis steps require SLO profile.
    let needs_slo = loaded_ops
        .ops
        .doc_json
        .get("deploy")
        .and_then(|d| d.get("canary"))
        .and_then(|c| c.get("steps"))
        .and_then(Value::as_array)
        .is_some_and(|steps| steps.iter().any(|s| s.get("analysis").is_some()));
    if needs_slo && slo_digest.is_none() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEPLOY_PLAN_SLO_PROFILE_REQUIRED",
            Severity::Error,
            Stage::Parse,
            "deploy plan requires ops.slo when deploy steps include analysis".to_string(),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    if let Err(err) = std::fs::create_dir_all(&args.out_dir) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEPLOY_PLAN_OUT_DIR_CREATE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to create out-dir {}: {err}", args.out_dir.display()),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    let rollout_path = args.out_dir.join("rollout.yaml");
    let analysis_path = args.out_dir.join("analysis-template.yaml");
    let service_path = args.out_dir.join("service.yaml");
    let ingress_path = args.out_dir.join("ingress.yaml");

    let k8s_name = k8s_name(pack_profile_id);
    let analysis_name = format!("{k8s_name}-analysis");

    let strategy = build_strategy(&loaded_ops.ops.doc_json);

    let rollout_yaml = rollout_yaml(&k8s_name, &analysis_name, &strategy);
    let analysis_yaml = analysis_template_yaml(&analysis_name);
    let service_yaml = service_yaml(&k8s_name);
    let ingress_yaml = ingress_yaml(&k8s_name);

    let mut outputs: Vec<report::meta::FileDigest> = Vec::new();
    for (path, content) in [
        (&rollout_path, rollout_yaml),
        (&analysis_path, analysis_yaml),
        (&service_path, service_yaml),
        (&ingress_path, ingress_yaml),
    ] {
        if let Err(err) = std::fs::write(path, content.as_bytes()) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_EMIT_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to write {}: {err}", path.display()),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                &args.out_dir,
                None,
                Vec::new(),
            );
        }
        let d = util::file_digest(path)?;
        meta.outputs.push(d.clone());
        outputs.push(d);
    }
    let outputs_report = outputs.clone();

    let mut plan_doc = json!({
      "schema_version": "x07.deploy.plan@0.1.0",
      "id": format!("{pack_profile_id}.{k8s_name}"),
      "v": 1,
      "pack_manifest": pack_digest,
      "ops_profile": ops_digest,
      "policy_cards": policy_card_digests,
      "slo_profile": slo_digest,
      "strategy": strategy,
      "outputs": outputs,
    });

    let policy_docs: Vec<Value> = loaded_ops
        .policy_cards
        .iter()
        .map(|c| c.doc_json.clone())
        .collect();
    let mut policy_diags: Vec<Diagnostic> = Vec::new();
    plan_doc = apply_policy_cards(
        plan_doc,
        &policy_docs,
        PolicyTarget::DeployPlan,
        &mut policy_diags,
    )?;
    diagnostics.extend(policy_diags);
    if diagnostics.iter().any(|d| {
        d.severity == Severity::Error
            && (d.code == "X07WASM_POLICY_DECISION_DENY"
                || d.code == "X07WASM_POLICY_OBLIGATION_UNSATISFIED")
    }) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEPLOY_PLAN_POLICY_DENIED",
            Severity::Error,
            Stage::Lint,
            "deploy plan denied by policy".to_string(),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    let plan_schema_diags =
        store.validate("https://x07.io/spec/x07-deploy.plan.schema.json", &plan_doc)?;
    if !plan_schema_diags.is_empty() {
        for dd in plan_schema_diags {
            let mut d = Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                dd.message,
            );
            d.data = dd.data;
            diagnostics.push(d);
        }
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    let plan_path = args.out_dir.join("deploy.plan.json");
    let plan_bytes = report::canon::canonical_pretty_json_bytes(&plan_doc)?;
    std::fs::write(&plan_path, &plan_bytes)
        .with_context(|| format!("write: {}", plan_path.display()))
        .map_err(|err| {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEPLOY_PLAN_EMIT_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            err
        })
        .ok();
    if diagnostics
        .iter()
        .any(|d| d.code == "X07WASM_DEPLOY_PLAN_EMIT_FAILED")
    {
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args.out_dir,
            None,
            Vec::new(),
        );
    }

    let plan_digest = util::file_digest(&plan_path)?;
    meta.outputs.push(plan_digest.clone());

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args.out_dir,
        Some(plan_digest),
        outputs_report,
    )
}

fn k8s_name(s: &str) -> String {
    let mut out = s.to_ascii_lowercase().replace('_', "-");
    out.retain(|c| c.is_ascii_alphanumeric() || c == '-');
    if out.is_empty() {
        out = "app".to_string();
    }
    out
}

fn build_strategy(ops_doc: &Value) -> Value {
    let deploy = ops_doc.get("deploy").cloned().unwrap_or(Value::Null);
    let strategy = deploy
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("canary");
    match strategy {
        "blue_green" => json!({
          "type": "blue_green",
          "canary": null,
          "blue_green": deploy.get("blue_green").cloned().unwrap_or(json!({"auto_promote": false})),
        }),
        _ => json!({
          "type": "canary",
          "canary": deploy.get("canary").cloned().unwrap_or(json!({"steps":[{"set_weight":100}]})),
          "blue_green": null,
        }),
    }
}

fn rollout_yaml(name: &str, analysis_name: &str, strategy: &Value) -> String {
    let steps_yaml = if strategy.get("type").and_then(Value::as_str) == Some("canary") {
        let steps = strategy
            .get("canary")
            .and_then(|c| c.get("steps"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = String::new();
        for s in steps {
            if let Some(w) = s.get("set_weight").and_then(Value::as_i64) {
                out.push_str(&format!("        - setWeight: {w}\n"));
                continue;
            }
            if let Some(p) = s.get("pause_s").and_then(Value::as_i64) {
                out.push_str(&format!("        - pause:\n            duration: {p}s\n"));
                continue;
            }
            if s.get("analysis").is_some() {
                out.push_str(&format!(
                    "        - analysis:\n            templates:\n              - templateName: {analysis_name}\n"
                ));
            }
        }
        if out.trim().is_empty() {
            "        - setWeight: 100\n".to_string()
        } else {
            out
        }
    } else {
        "        - setWeight: 100\n".to_string()
    };

    format!(
        "apiVersion: argoproj.io/v1alpha1\nkind: Rollout\nmetadata:\n  name: {name}\nspec:\n  replicas: 1\n  selector:\n    matchLabels:\n      app: {name}\n  template:\n    metadata:\n      labels:\n        app: {name}\n    spec:\n      containers:\n        - name: app\n          image: REPLACE_ME\n          ports:\n            - containerPort: 8080\n  strategy:\n    canary:\n      steps:\n{steps_yaml}"
    )
}

fn analysis_template_yaml(name: &str) -> String {
    format!(
        "apiVersion: argoproj.io/v1alpha1\nkind: AnalysisTemplate\nmetadata:\n  name: {name}\nspec:\n  metrics:\n    - name: slo-eval\n      interval: 30s\n      count: 1\n      successCondition: result == 'promote'\n      provider:\n        job:\n          spec:\n            template:\n              spec:\n                restartPolicy: Never\n                containers:\n                  - name: slo-eval\n                    image: REPLACE_ME\n                    command: ['sh','-c','echo promote']"
    )
}

fn service_yaml(name: &str) -> String {
    format!(
        "apiVersion: v1\nkind: Service\nmetadata:\n  name: {name}\nspec:\n  selector:\n    app: {name}\n  ports:\n    - name: http\n      port: 80\n      targetPort: 8080"
    )
}

fn ingress_yaml(name: &str) -> String {
    format!(
        "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: {name}\nspec:\n  rules:\n    - http:\n        paths:\n          - path: /\n            pathType: Prefix\n            backend:\n              service:\n                name: {name}\n                port:\n                  number: 80"
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    out_dir: &Path,
    plan_manifest: Option<report::meta::FileDigest>,
    outputs: Vec<report::meta::FileDigest>,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let plan_manifest = plan_manifest.unwrap_or(report::meta::FileDigest {
        path: out_dir.join("deploy.plan.json").display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.deploy.plan.report@0.1.0",
      "command": "x07-wasm.deploy.plan",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "out_dir": out_dir.display().to_string(),
        "plan_manifest": plan_manifest,
        "outputs": outputs,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

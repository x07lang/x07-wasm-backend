use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, SloEvalArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::json_doc::load_validated_json_doc;
use crate::report;
use crate::schema::SchemaStore;

#[derive(Debug, Clone, Deserialize)]
struct SloProfileDoc {
    service: String,
    indicators: Vec<SloIndicator>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SloIndicator {
    ErrorRate {
        id: String,
        metric: String,
        objective: ObjectiveErrorRate,
    },
    LatencyP95Ms {
        id: String,
        metric: String,
        objective: ObjectiveLatencyP95Ms,
    },
    Availability {
        id: String,
        metric: String,
        objective: ObjectiveAvailability,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct ObjectiveErrorRate {
    max: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ObjectiveLatencyP95Ms {
    max_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct ObjectiveAvailability {
    min: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct MetricsSnapshotDoc {
    service: String,
    metrics: Vec<MetricValue>,
}

#[derive(Debug, Clone, Deserialize)]
struct MetricValue {
    name: String,
    value: f64,
    unit: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SloEvalOutcome {
    pub decision: &'static str,
    pub violations: u64,
    pub indicators: Vec<Value>,
}

pub fn cmd_slo_eval(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: SloEvalArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let (slo_digest, slo_doc) = match load_validated_json_doc(
        &store,
        &args.profile,
        "https://x07.io/spec/x07-slo.profile.schema.json",
        "X07WASM_SLO_PROFILE_READ_FAILED",
        "X07WASM_SLO_SCHEMA_INVALID",
        &mut meta,
        &mut diagnostics,
    )? {
        Some(v) => v,
        None => {
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                args.profile.display().to_string(),
                args.metrics.display().to_string(),
                None,
                None,
                "inconclusive",
                0,
                Vec::new(),
                3,
            );
        }
    };

    let (metrics_digest, metrics_doc) = match load_validated_json_doc(
        &store,
        &args.metrics,
        "https://x07.io/spec/x07-metrics.snapshot.schema.json",
        "X07WASM_METRICS_SNAPSHOT_READ_FAILED",
        "X07WASM_METRICS_SNAPSHOT_SCHEMA_INVALID",
        &mut meta,
        &mut diagnostics,
    )? {
        Some(v) => v,
        None => {
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                meta,
                diagnostics,
                args.profile.display().to_string(),
                args.metrics.display().to_string(),
                Some(slo_digest),
                None,
                "inconclusive",
                0,
                Vec::new(),
                3,
            );
        }
    };

    let outcome = evaluate_slo_docs(&slo_doc, &metrics_doc, &mut diagnostics);
    let exit_code = match outcome.decision {
        "promote" => 0u8,
        "rollback" => 2u8,
        _ => 3u8,
    };

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        args.profile.display().to_string(),
        args.metrics.display().to_string(),
        Some(slo_digest),
        Some(metrics_digest),
        outcome.decision,
        outcome.violations,
        outcome.indicators,
        exit_code,
    )
}

pub(crate) fn evaluate_slo_docs(
    slo_doc: &Value,
    metrics_doc: &Value,
    diagnostics: &mut Vec<Diagnostic>,
) -> SloEvalOutcome {
    let slo_parsed: SloProfileDoc = match serde_json::from_value(slo_doc.clone()) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SLO_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse SLO profile: {err}"),
            ));
            return SloEvalOutcome {
                decision: "inconclusive",
                violations: 0,
                indicators: Vec::new(),
            };
        }
    };

    let metrics_parsed: MetricsSnapshotDoc = match serde_json::from_value(metrics_doc.clone()) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_METRICS_SNAPSHOT_SCHEMA_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse metrics snapshot: {err}"),
            ));
            return SloEvalOutcome {
                decision: "inconclusive",
                violations: 0,
                indicators: Vec::new(),
            };
        }
    };

    if metrics_parsed.service != slo_parsed.service {
        let mut d = Diagnostic::new(
            "X07WASM_SLO_EVAL_INCONCLUSIVE",
            Severity::Error,
            Stage::Run,
            format!(
                "metrics snapshot service does not match SLO profile service (metrics={:?}, slo={:?})",
                metrics_parsed.service, slo_parsed.service
            ),
        );
        d.data.insert(
            "metrics_service".to_string(),
            json!(metrics_parsed.service.clone()),
        );
        d.data
            .insert("slo_service".to_string(), json!(slo_parsed.service.clone()));
        diagnostics.push(d);
        return SloEvalOutcome {
            decision: "inconclusive",
            violations: 0,
            indicators: Vec::new(),
        };
    }

    let mut metrics_by_name: HashMap<String, f64> = HashMap::new();
    for m in metrics_parsed.metrics {
        let _ = m.unit;
        metrics_by_name.insert(m.name, m.value);
    }

    let mut indicators_eval: Vec<Value> = Vec::new();
    let mut violations = 0u64;
    let mut missing_metric = false;

    for ind in slo_parsed.indicators {
        match ind {
            SloIndicator::ErrorRate {
                id,
                metric,
                objective,
            } => {
                let Some(observed) = metrics_by_name.get(&metric).copied() else {
                    missing_metric = true;
                    diagnostics.push(missing_metric_diag(&metric, &id));
                    continue;
                };
                let ok = observed <= objective.max;
                if !ok {
                    violations += 1;
                }
                indicators_eval.push(json!({
                  "id": id,
                  "kind": "error_rate",
                  "metric": metric,
                  "observed": observed,
                  "ok": ok,
                  "objective": { "max": objective.max },
                }));
            }
            SloIndicator::LatencyP95Ms {
                id,
                metric,
                objective,
            } => {
                let Some(observed) = metrics_by_name.get(&metric).copied() else {
                    missing_metric = true;
                    diagnostics.push(missing_metric_diag(&metric, &id));
                    continue;
                };
                let ok = observed <= objective.max_ms as f64;
                if !ok {
                    violations += 1;
                }
                indicators_eval.push(json!({
                  "id": id,
                  "kind": "latency_p95_ms",
                  "metric": metric,
                  "observed": observed,
                  "ok": ok,
                  "objective": { "max_ms": objective.max_ms },
                }));
            }
            SloIndicator::Availability {
                id,
                metric,
                objective,
            } => {
                let Some(observed) = metrics_by_name.get(&metric).copied() else {
                    missing_metric = true;
                    diagnostics.push(missing_metric_diag(&metric, &id));
                    continue;
                };
                let ok = observed >= objective.min;
                if !ok {
                    violations += 1;
                }
                indicators_eval.push(json!({
                  "id": id,
                  "kind": "availability",
                  "metric": metric,
                  "observed": observed,
                  "ok": ok,
                  "objective": { "min": objective.min },
                }));
            }
        }
    }

    let decision = if missing_metric {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SLO_EVAL_INCONCLUSIVE",
            Severity::Error,
            Stage::Run,
            "missing required metric(s) for one or more indicators".to_string(),
        ));
        "inconclusive"
    } else if violations > 0 {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SLO_VIOLATION",
            Severity::Error,
            Stage::Run,
            format!("SLO violations: {violations}"),
        ));
        "rollback"
    } else {
        "promote"
    };

    SloEvalOutcome {
        decision,
        violations,
        indicators: indicators_eval,
    }
}

fn missing_metric_diag(metric: &str, indicator_id: &str) -> Diagnostic {
    let mut d = Diagnostic::new(
        "X07WASM_SLO_METRIC_MISSING",
        Severity::Error,
        Stage::Run,
        format!("missing metric for indicator {indicator_id:?}: {metric:?}"),
    );
    d.data
        .insert("metric".to_string(), json!(metric.to_string()));
    d.data
        .insert("indicator_id".to_string(), json!(indicator_id.to_string()));
    d
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
    profile_path: String,
    metrics_path: String,
    slo_digest: Option<report::meta::FileDigest>,
    metrics_digest: Option<report::meta::FileDigest>,
    decision: &str,
    violations: u64,
    indicators: Vec<Value>,
    exit_code: u8,
) -> Result<u8> {
    let slo_digest = slo_digest.unwrap_or(report::meta::FileDigest {
        path: profile_path,
        sha256: "0".repeat(64),
        bytes_len: 0,
    });
    let metrics_digest = metrics_digest.unwrap_or(report::meta::FileDigest {
        path: metrics_path,
        sha256: "0".repeat(64),
        bytes_len: 0,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.slo.eval.report@0.1.0",
      "command": "x07-wasm.slo.eval",
      "ok": decision == "promote",
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "slo_profile": slo_digest,
        "metrics_snapshot": metrics_digest,
        "decision": decision,
        "violations": violations,
        "indicators": indicators,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

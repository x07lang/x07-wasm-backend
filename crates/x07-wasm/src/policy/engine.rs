use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::web_ui::replay::apply_json_patch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyTarget {
    OpsProfile,
    Capabilities,
    SloProfile,
    DeployPlan,
    AppPack,
    ProvenanceAttestation,
}

impl PolicyTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyTarget::OpsProfile => "ops_profile",
            PolicyTarget::Capabilities => "capabilities",
            PolicyTarget::SloProfile => "slo_profile",
            PolicyTarget::DeployPlan => "deploy_plan",
            PolicyTarget::AppPack => "app_pack",
            PolicyTarget::ProvenanceAttestation => "provenance_attestation",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ops_profile" => Some(PolicyTarget::OpsProfile),
            "capabilities" => Some(PolicyTarget::Capabilities),
            "slo_profile" => Some(PolicyTarget::SloProfile),
            "deploy_plan" => Some(PolicyTarget::DeployPlan),
            "app_pack" => Some(PolicyTarget::AppPack),
            "provenance_attestation" => Some(PolicyTarget::ProvenanceAttestation),
            _ => None,
        }
    }
}

pub fn apply_policy_cards(
    mut doc: Value,
    cards: &[Value],
    target: PolicyTarget,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Value> {
    for card in cards {
        let card_id = card.get("id").and_then(Value::as_str).unwrap_or("unknown");
        let rules = card
            .get("rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for rule in rules {
            let Some(rule_target) = rule
                .get("target")
                .and_then(Value::as_str)
                .and_then(PolicyTarget::from_str)
            else {
                continue;
            };
            if rule_target != target {
                continue;
            }

            let rule_id = rule.get("id").and_then(Value::as_str).unwrap_or("unknown");
            let effect = rule.get("effect").and_then(Value::as_str).unwrap_or("deny");
            let assertions = rule
                .get("assertions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let patches = rule
                .get("patches")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            let mut satisfied =
                assertions_all_match(&doc, &assertions, diagnostics, card_id, rule_id)?;
            if !satisfied && !patches.is_empty() {
                let patchset = Value::Array(patches);
                match apply_json_patch(doc.clone(), &patchset) {
                    Ok(patched) => {
                        doc = patched;
                        satisfied =
                            assertions_all_match(&doc, &assertions, diagnostics, card_id, rule_id)?;
                    }
                    Err(err) => {
                        let mut d = Diagnostic::new(
                            "X07WASM_POLICY_PATCH_APPLY_FAILED",
                            Severity::Error,
                            Stage::Rewrite,
                            format!("failed to apply policy patch: {err:#}"),
                        );
                        d.data.insert("card_id".to_string(), json!(card_id));
                        d.data.insert("rule_id".to_string(), json!(rule_id));
                        diagnostics.push(d);
                        continue;
                    }
                }
            }

            if satisfied {
                continue;
            }

            let reason = rule
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("policy rule failed assertions");
            let (code, severity) = match effect {
                "warn" => ("X07WASM_POLICY_DECISION_WARN", Severity::Warning),
                "require" => ("X07WASM_POLICY_OBLIGATION_UNSATISFIED", Severity::Error),
                _ => ("X07WASM_POLICY_DECISION_DENY", Severity::Error),
            };
            let mut d = Diagnostic::new(code, severity, Stage::Lint, reason.to_string());
            d.data.insert("card_id".to_string(), json!(card_id));
            d.data.insert("rule_id".to_string(), json!(rule_id));
            d.data.insert("target".to_string(), json!(target.as_str()));
            diagnostics.push(d);
        }
    }

    Ok(doc)
}

fn assertions_all_match(
    doc: &Value,
    assertions: &[Value],
    diagnostics: &mut Vec<Diagnostic>,
    card_id: &str,
    rule_id: &str,
) -> Result<bool> {
    for a in assertions {
        if !assertion_matches(doc, a, diagnostics, card_id, rule_id)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn assertion_matches(
    doc: &Value,
    assertion: &Value,
    diagnostics: &mut Vec<Diagnostic>,
    card_id: &str,
    rule_id: &str,
) -> Result<bool> {
    let op = assertion.get("op").and_then(Value::as_str).unwrap_or("");
    let path = assertion.get("path").and_then(Value::as_str).unwrap_or("");
    let cur = doc.pointer(path);
    match op {
        "exists" => Ok(cur.is_some()),
        "not_exists" => Ok(cur.is_none()),
        "eq" => Ok(cur == assertion.get("value")),
        "neq" => Ok(cur != assertion.get("value")),
        "matches" => {
            let Some(v) = assertion.get("value").and_then(Value::as_str) else {
                return Ok(false);
            };
            let Ok(re) = Regex::new(v) else {
                let mut d = Diagnostic::new(
                    "X07WASM_POLICY_SCHEMA_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("invalid assertion regex: {v:?}"),
                );
                d.data.insert("card_id".to_string(), json!(card_id));
                d.data.insert("rule_id".to_string(), json!(rule_id));
                diagnostics.push(d);
                return Ok(false);
            };
            let Some(s) = cur.and_then(Value::as_str) else {
                return Ok(false);
            };
            Ok(re.is_match(s))
        }
        "in" | "nin" => {
            let Some(arr) = assertion.get("value").and_then(Value::as_array) else {
                return Ok(false);
            };
            let Some(cur) = cur else {
                return Ok(op == "nin");
            };
            let hit = arr.iter().any(|v| v == cur);
            Ok(if op == "in" { hit } else { !hit })
        }
        _ => Ok(false),
    }
}

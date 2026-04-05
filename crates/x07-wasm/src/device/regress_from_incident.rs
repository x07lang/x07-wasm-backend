use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::cli::{DeviceRegressFromIncidentArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_device_regress_from_incident(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceRegressFromIncidentArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut generated: Vec<report::meta::FileDigest> = Vec::new();
    let mut generated_trace_artifact_refs: Vec<report::meta::FileDigest> = Vec::new();
    let mut generated_report_artifact_refs: Vec<report::meta::FileDigest> = Vec::new();
    let mut replay_target_kind = Value::Null;
    let mut replay_mode = Value::Null;
    let mut replay_synthesis_status = Value::Null;
    let mut native_replay_hints = Value::Null;
    let mut invariant_evaluation = Value::Null;

    if !args.incident_dir.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_INCIDENT_DIR_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("incident dir not found: {}", args.incident_dir.display()),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            &args,
            generated,
            generated_trace_artifact_refs,
            generated_report_artifact_refs,
            replay_target_kind,
            replay_mode,
            replay_synthesis_status,
            native_replay_hints,
            invariant_evaluation,
        );
    }

    match load_platform_incident_input(&args.incident_dir, &mut meta, &mut diagnostics)? {
        Some(platform) => {
            replay_mode = json!("platform_native_v1");

            if diagnostics.iter().any(|d| d.severity == Severity::Error) {
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    &args,
                    generated,
                    generated_trace_artifact_refs,
                    generated_report_artifact_refs,
                    replay_target_kind,
                    replay_mode,
                    replay_synthesis_status,
                    native_replay_hints,
                    invariant_evaluation,
                );
            }

            let requested_invariants = requested_invariants(&platform.request_doc);
            let normalized_hints = synthesize_native_replay_hints(&platform);
            if normalized_hints.host_target.is_empty() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_REGRESS_HOST_TARGET_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    "platform incident bundle did not resolve a replay host_target".to_string(),
                ));
            }
            if normalized_hints.native_sequence.is_empty() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_REGRESS_NATIVE_SEQUENCE_EMPTY",
                    Severity::Error,
                    Stage::Parse,
                    "platform incident bundle did not resolve a native replay sequence".to_string(),
                ));
            }
            if diagnostics.iter().any(|d| d.severity == Severity::Error) {
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    &args,
                    generated,
                    generated_trace_artifact_refs,
                    generated_report_artifact_refs,
                    replay_target_kind,
                    replay_mode,
                    replay_synthesis_status,
                    native_replay_hints,
                    invariant_evaluation,
                );
            }

            replay_target_kind = json!(normalized_hints.host_target.clone());
            native_replay_hints = json!(normalized_hints);
            invariant_evaluation = json!({
                "requested": requested_invariants.clone(),
                "status": if requested_invariants.is_empty() { "not_requested" } else { "ok" },
            });
            replay_synthesis_status = json!(if args.dry_run {
                "validated"
            } else {
                "generated"
            });

            if !args.dry_run {
                std::fs::create_dir_all(&args.out_dir)
                    .with_context(|| format!("create dir: {}", args.out_dir.display()))?;

                let incident_out = args
                    .out_dir
                    .join(format!("{}.incident.bundle.json", args.name));
                let incident_digest = write_generated_json(
                    &incident_out,
                    &platform.bundle_doc,
                    &mut meta,
                    &mut generated,
                )?;
                generated_report_artifact_refs.push(incident_digest);

                if platform.meta_local_doc != Value::Null {
                    let meta_local_out = args
                        .out_dir
                        .join(format!("{}.incident.meta.local.json", args.name));
                    let digest = write_generated_json(
                        &meta_local_out,
                        &platform.meta_local_doc,
                        &mut meta,
                        &mut generated,
                    )?;
                    generated_report_artifact_refs.push(digest);
                }
                if platform.meta_remote_doc != Value::Null {
                    let meta_remote_out = args
                        .out_dir
                        .join(format!("{}.incident.meta.remote.json", args.name));
                    let digest = write_generated_json(
                        &meta_remote_out,
                        &platform.meta_remote_doc,
                        &mut meta,
                        &mut generated,
                    )?;
                    generated_report_artifact_refs.push(digest);
                }
                if platform.request_doc != Value::Null {
                    let request_out = args
                        .out_dir
                        .join(format!("{}.regression.request.json", args.name));
                    let digest = write_generated_json(
                        &request_out,
                        &platform.request_doc,
                        &mut meta,
                        &mut generated,
                    )?;
                    generated_report_artifact_refs.push(digest);
                }

                let replay_out = args
                    .out_dir
                    .join(format!("{}.native.replay.json", args.name));
                let replay_doc = json!({
                    "kind": "x07.device.native_replay_fixture",
                    "incident_id": platform.incident_id,
                    "regression_id": platform.regression_id,
                    "classification": platform.native_classification,
                    "host_target": normalized_hints.host_target,
                    "prelude": normalized_hints.prelude,
                    "native_sequence": normalized_hints.native_sequence,
                    "requested_invariants": requested_invariants,
                });
                let digest =
                    write_generated_json(&replay_out, &replay_doc, &mut meta, &mut generated)?;
                generated_trace_artifact_refs.push(digest);
            }
        }
        None => {
            let legacy =
                load_legacy_incident(&store, &args.incident_dir, &mut meta, &mut diagnostics)?;
            if diagnostics.iter().any(|d| d.severity == Severity::Error) {
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    meta,
                    diagnostics,
                    &args,
                    generated,
                    generated_trace_artifact_refs,
                    generated_report_artifact_refs,
                    replay_target_kind,
                    replay_mode,
                    replay_synthesis_status,
                    native_replay_hints,
                    invariant_evaluation,
                );
            }

            replay_mode = json!("legacy_web_ui_incident_v1");
            replay_synthesis_status = json!(if args.dry_run {
                "validated"
            } else {
                "generated"
            });
            invariant_evaluation = json!({
                "requested": ["device.trace.replay"],
                "status": "ok",
            });

            if !args.dry_run {
                std::fs::create_dir_all(&args.out_dir)
                    .with_context(|| format!("create dir: {}", args.out_dir.display()))?;

                let incident_out = args.out_dir.join(format!("{}.incident.json", args.name));
                let digest = write_generated_json(
                    &incident_out,
                    &legacy.incident_doc,
                    &mut meta,
                    &mut generated,
                )?;
                generated_report_artifact_refs.push(digest);

                let trace_out = args.out_dir.join(format!("{}.trace.json", args.name));
                let digest =
                    write_generated_json(&trace_out, &legacy.trace_doc, &mut meta, &mut generated)?;
                generated_trace_artifact_refs.push(digest);

                if legacy.app_trace_doc != Value::Null {
                    let app_trace_out = args.out_dir.join(format!("{}.app.trace.json", args.name));
                    let digest = write_generated_json(
                        &app_trace_out,
                        &legacy.app_trace_doc,
                        &mut meta,
                        &mut generated,
                    )?;
                    generated_trace_artifact_refs.push(digest);
                }
            }
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        meta,
        diagnostics,
        &args,
        generated,
        generated_trace_artifact_refs,
        generated_report_artifact_refs,
        replay_target_kind,
        replay_mode,
        replay_synthesis_status,
        native_replay_hints,
        invariant_evaluation,
    )
}

#[derive(Debug, Clone)]
struct PlatformIncidentInput {
    bundle_doc: Value,
    meta_local_doc: Value,
    meta_remote_doc: Value,
    request_doc: Value,
    incident_id: Value,
    regression_id: Value,
    native_classification: String,
}

#[derive(Debug, Clone)]
struct LegacyIncidentInput {
    incident_doc: Value,
    trace_doc: Value,
    app_trace_doc: Value,
}

#[derive(Debug, Clone, Serialize)]
struct NativeReplayHints {
    host_target: String,
    prelude: Vec<Value>,
    native_sequence: Vec<Value>,
}

fn load_platform_incident_input(
    incident_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<PlatformIncidentInput>> {
    let bundle_path = incident_dir.join("incident.bundle.json");
    let meta_local_path = incident_dir.join("incident.meta.local.json");
    let meta_remote_path = incident_dir.join("incident.meta.remote.json");
    let request_path = incident_dir.join("regression.request.json");

    let has_platform_inputs = bundle_path.is_file()
        || meta_local_path.is_file()
        || meta_remote_path.is_file()
        || request_path.is_file();
    if !has_platform_inputs {
        return Ok(None);
    }
    if !bundle_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_PLATFORM_BUNDLE_MISSING",
            Severity::Error,
            Stage::Parse,
            format!(
                "platform incident inputs are present but incident.bundle.json is missing: {}",
                bundle_path.display()
            ),
        ));
        return Ok(Some(PlatformIncidentInput {
            bundle_doc: Value::Null,
            meta_local_doc: Value::Null,
            meta_remote_doc: Value::Null,
            request_doc: Value::Null,
            incident_id: Value::Null,
            regression_id: Value::Null,
            native_classification: "native_runtime_error".to_string(),
        }));
    }

    let bundle_doc = read_input_json(
        &bundle_path,
        meta,
        diagnostics,
        "X07WASM_DEVICE_REGRESS_PLATFORM_BUNDLE_READ_FAILED",
        "X07WASM_DEVICE_REGRESS_PLATFORM_BUNDLE_JSON_INVALID",
    )
    .unwrap_or(Value::Null);
    let meta_local_doc = if meta_local_path.is_file() {
        read_input_json(
            &meta_local_path,
            meta,
            diagnostics,
            "X07WASM_DEVICE_REGRESS_PLATFORM_META_LOCAL_READ_FAILED",
            "X07WASM_DEVICE_REGRESS_PLATFORM_META_LOCAL_JSON_INVALID",
        )
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let meta_remote_doc = if meta_remote_path.is_file() {
        read_input_json(
            &meta_remote_path,
            meta,
            diagnostics,
            "X07WASM_DEVICE_REGRESS_PLATFORM_META_REMOTE_READ_FAILED",
            "X07WASM_DEVICE_REGRESS_PLATFORM_META_REMOTE_JSON_INVALID",
        )
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let request_doc = if request_path.is_file() {
        read_input_json(
            &request_path,
            meta,
            diagnostics,
            "X07WASM_DEVICE_REGRESS_PLATFORM_REQUEST_READ_FAILED",
            "X07WASM_DEVICE_REGRESS_PLATFORM_REQUEST_JSON_INVALID",
        )
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    let native_classification = resolve_native_classification(&bundle_doc, &meta_local_doc);
    let incident_id = bundle_doc
        .get("incident_id")
        .cloned()
        .unwrap_or(Value::Null);
    let regression_id = request_doc
        .get("regression_id")
        .cloned()
        .unwrap_or(Value::Null);

    Ok(Some(PlatformIncidentInput {
        bundle_doc,
        meta_local_doc,
        meta_remote_doc,
        request_doc,
        incident_id,
        regression_id,
        native_classification,
    }))
}

fn load_legacy_incident(
    store: &SchemaStore,
    incident_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<LegacyIncidentInput> {
    let incident_path = incident_dir.join("incident.json");
    let incident_doc = read_input_json(
        &incident_path,
        meta,
        diagnostics,
        "X07WASM_DEVICE_REGRESS_INCIDENT_READ_FAILED",
        "X07WASM_DEVICE_REGRESS_INCIDENT_JSON_INVALID",
    )
    .unwrap_or(Value::Null);

    let kind = incident_doc
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != "x07.web_ui.incident" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_INCIDENT_KIND_INVALID",
            Severity::Error,
            Stage::Parse,
            format!("unexpected incident.kind: {kind:?}"),
        ));
    }

    let trace_doc = incident_doc.get("trace").cloned().unwrap_or(Value::Null);
    if trace_doc == Value::Null {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_REGRESS_TRACE_MISSING",
            Severity::Error,
            Stage::Parse,
            "device incident is missing trace".to_string(),
        ));
    } else {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-web_ui.trace.schema.json",
            &trace_doc,
        )?);
    }

    let app_trace_doc = incident_doc.get("appTrace").cloned().unwrap_or(Value::Null);
    if app_trace_doc != Value::Null {
        diagnostics.extend(store.validate(
            "https://x07.io/spec/x07-app.trace.schema.json",
            &app_trace_doc,
        )?);
    }

    Ok(LegacyIncidentInput {
        incident_doc,
        trace_doc,
        app_trace_doc,
    })
}

fn read_input_json(
    path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    read_failed_code: &str,
    json_invalid_code: &str,
) -> Option<Value> {
    if let Ok(d) = util::file_digest(path) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                read_failed_code,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {}: {err}", path.display()),
            ));
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                json_invalid_code,
                Severity::Error,
                Stage::Parse,
                format!("{} is not JSON: {err}", path.display()),
            ));
            None
        }
    }
}

fn synthesize_native_replay_hints(input: &PlatformIncidentInput) -> NativeReplayHints {
    let request_hints = input
        .request_doc
        .get("native_replay_hints")
        .cloned()
        .unwrap_or(Value::Null);
    let native_context = native_context_doc(input);
    let host_target = json_str(&request_hints, "/host_target")
        .or_else(|| json_str(&input.meta_local_doc, "/target_kind"))
        .or_else(|| json_str(&input.meta_local_doc, "/device_release/target"))
        .or_else(|| json_str(&input.bundle_doc, "/meta/target_kind"))
        .or_else(|| json_str(&native_context, "/platform"))
        .unwrap_or_default();

    let prelude = if let Some(items) = request_hints.get("prelude").and_then(Value::as_array) {
        items.iter().map(normalize_prelude_step).collect::<Vec<_>>()
    } else {
        synthesize_prelude(&native_context)
    };

    let native_sequence = if let Some(items) = request_hints
        .get("native_sequence")
        .and_then(Value::as_array)
    {
        items
            .iter()
            .map(normalize_native_sequence_step)
            .collect::<Vec<_>>()
    } else {
        synthesize_native_sequence(&native_context, &input.native_classification)
    };

    NativeReplayHints {
        host_target,
        prelude,
        native_sequence,
    }
}

fn requested_invariants(request_doc: &Value) -> Vec<String> {
    request_doc
        .get("invariants")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn native_context_doc(input: &PlatformIncidentInput) -> Value {
    input
        .bundle_doc
        .pointer("/meta/native_context")
        .cloned()
        .or_else(|| input.meta_local_doc.get("native_context").cloned())
        .or_else(|| input.meta_remote_doc.get("native_context").cloned())
        .unwrap_or(Value::Null)
}

fn synthesize_prelude(native_context: &Value) -> Vec<Value> {
    let mut prelude = Vec::new();

    if let Some(state) = json_str(native_context, "/lifecycle_state") {
        prelude.push(json!({ "kind": format!("lifecycle.{state}") }));
    }
    if let Some(state) = json_str(native_context, "/connectivity_state") {
        prelude.push(json!({ "kind": format!("connectivity.{state}") }));
    }

    prelude
}

fn synthesize_native_sequence(native_context: &Value, classification: &str) -> Vec<Value> {
    let mut sequence = native_context
        .get("breadcrumbs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(ord, item)| {
                    let request_id = item
                        .get("request_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("step_{ord}"));
                    let op = item
                        .get("op")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| fallback_op_for_classification(classification));
                    let mut result = json!({
                        "status": item
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_else(|| fallback_status_for_classification(classification)),
                    });
                    if let Some(obj) = result.as_object_mut() {
                        copy_string_field(item, obj, "permission");
                        copy_u64_field(item, obj, "duration_ms");
                        copy_u64_field(item, obj, "timeout_ms");
                        copy_string_field(item, obj, "error_code");
                        copy_string_field(item, obj, "reason");
                    }
                    json!({
                        "request_id": request_id,
                        "op": op,
                        "event_class": item.get("event_class").cloned().unwrap_or(Value::Null),
                        "result": result,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if sequence.is_empty() {
        sequence.extend(fallback_sequence_from_context(
            native_context,
            classification,
        ));
    }

    sequence
}

fn fallback_sequence_from_context(native_context: &Value, classification: &str) -> Vec<Value> {
    let denied_permission = native_context
        .get("permission_state_snapshot")
        .and_then(Value::as_object)
        .and_then(|items| {
            items.iter().find_map(|(key, value)| {
                if value.as_str() == Some("denied") {
                    Some(key.to_string())
                } else {
                    None
                }
            })
        });

    match classification {
        "native_permission_blocked" => {
            let permission = denied_permission.unwrap_or_else(|| "unknown".to_string());
            vec![
                json!({
                    "request_id": "perm_0",
                    "op": "permissions.request",
                    "event_class": "policy.violation",
                    "result": {
                        "status": "denied",
                        "permission": permission,
                    }
                }),
                json!({
                    "request_id": "op_0",
                    "op": "location.get_current",
                    "event_class": "policy.violation",
                    "result": {
                        "status": "denied",
                    }
                }),
            ]
        }
        "native_bridge_timeout" => vec![json!({
            "request_id": "bridge_0",
            "op": "bridge.call",
            "event_class": "bridge.timing",
            "result": {
                "status": "timeout",
                "timeout_ms": 5000,
            }
        })],
        "native_policy_violation" => vec![json!({
            "request_id": "policy_0",
            "op": "policy.enforce",
            "event_class": "policy.violation",
            "result": {
                "status": "denied",
            }
        })],
        "native_host_crash" => vec![json!({
            "request_id": "host_0",
            "op": "webview.load",
            "event_class": "host.webview_crash",
            "result": {
                "status": "crashed",
            }
        })],
        _ => vec![json!({
            "request_id": "runtime_0",
            "op": "runtime.dispatch",
            "event_class": "runtime.error",
            "result": {
                "status": "error",
            }
        })],
    }
}

fn fallback_op_for_classification(classification: &str) -> String {
    match classification {
        "native_permission_blocked" => "permissions.request".to_string(),
        "native_bridge_timeout" => "bridge.call".to_string(),
        "native_policy_violation" => "policy.enforce".to_string(),
        "native_host_crash" => "webview.load".to_string(),
        _ => "runtime.dispatch".to_string(),
    }
}

fn fallback_status_for_classification(classification: &str) -> &'static str {
    match classification {
        "native_permission_blocked" | "native_policy_violation" => "denied",
        "native_bridge_timeout" => "timeout",
        "native_host_crash" => "crashed",
        _ => "error",
    }
}

fn resolve_native_classification(bundle_doc: &Value, meta_local_doc: &Value) -> String {
    let raw = json_str(bundle_doc, "/meta/native_classification")
        .or_else(|| json_str(meta_local_doc, "/native_classification"))
        .or_else(|| json_str(meta_local_doc, "/classification"))
        .unwrap_or_else(|| "native_runtime_error".to_string());

    match raw.as_str() {
        "native_runtime_error"
        | "device_js_unhandled"
        | "device_bridge_parse"
        | "device_release_gate_failed"
        | "device_release_provider_failed" => "native_runtime_error".to_string(),
        "native_policy_violation" | "device_policy_violation" => {
            "native_policy_violation".to_string()
        }
        "native_bridge_timeout" => "native_bridge_timeout".to_string(),
        "native_host_crash" | "device_webview_crash" | "device_crash_spike" => {
            "native_host_crash".to_string()
        }
        "native_permission_blocked" => "native_permission_blocked".to_string(),
        _ => "native_runtime_error".to_string(),
    }
}

fn json_str(doc: &Value, pointer: &str) -> Option<String> {
    doc.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn normalize_prelude_step(item: &Value) -> Value {
    let mut doc = json!({
        "kind": item
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
    });
    if let Some(obj) = doc.as_object_mut() {
        copy_string_field(item, obj, "state");
        copy_string_field(item, obj, "permission");
        copy_string_field(item, obj, "status");
        copy_string_field(item, obj, "request_id");
        copy_u64_field(item, obj, "unix_ms");
        copy_u64_field(item, obj, "duration_ms");
        copy_string_field(item, obj, "detail");
    }
    doc
}

fn normalize_native_sequence_step(item: &Value) -> Value {
    let mut result = item.get("result").cloned().unwrap_or_else(|| {
        json!({
            "status": item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
        })
    });
    util::canon_value_jcs(&mut result);
    let mut doc = json!({
        "request_id": item
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("step_0"),
        "op": item
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        "result": result,
    });
    if let Some(obj) = doc.as_object_mut() {
        if let Some(event_class) = item.get("event_class") {
            obj.insert("event_class".to_string(), event_class.clone());
        }
    }
    doc
}

fn copy_string_field(from: &Value, to: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(value) = from.get(key).and_then(Value::as_str) {
        to.insert(key.to_string(), json!(value));
    }
}

fn copy_u64_field(from: &Value, to: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(value) = from.get(key).and_then(Value::as_u64) {
        to.insert(key.to_string(), json!(value));
    }
}

fn write_generated_json(
    out_path: &Path,
    doc: &Value,
    meta: &mut report::meta::ReportMeta,
    generated: &mut Vec<report::meta::FileDigest>,
) -> Result<report::meta::FileDigest> {
    std::fs::write(out_path, report::canon::canonical_pretty_json_bytes(doc)?)
        .with_context(|| format!("write: {}", out_path.display()))?;
    let digest = util::file_digest(out_path)?;
    meta.outputs.push(digest.clone());
    generated.push(digest.clone());
    Ok(digest)
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
    args: &DeviceRegressFromIncidentArgs,
    generated: Vec<report::meta::FileDigest>,
    generated_trace_artifact_refs: Vec<report::meta::FileDigest>,
    generated_report_artifact_refs: Vec<report::meta::FileDigest>,
    replay_target_kind: Value,
    replay_mode: Value,
    replay_synthesis_status: Value,
    native_replay_hints: Value,
    invariant_evaluation: Value,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.device.regress.from_incident.report@0.2.0",
      "command": "x07-wasm.device.regress.from-incident",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "incident_dir": args.incident_dir.display().to_string(),
        "out_dir": args.out_dir.display().to_string(),
        "name": args.name,
        "dry_run": args.dry_run,
        "generated": generated,
        "generated_trace_artifact_refs": generated_trace_artifact_refs,
        "generated_report_artifact_refs": generated_report_artifact_refs,
        "replay_target_kind": replay_target_kind,
        "replay_mode": replay_mode,
        "replay_synthesis_status": replay_synthesis_status,
        "native_replay_hints": native_replay_hints,
        "invariant_evaluation": invariant_evaluation
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "x07-wasm-device-regress-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn device_regress_accepts_platform_incident_artifacts() {
        let tmp = tmp_dir("platform");
        let incident_dir = tmp.join("incident");
        let out_dir = tmp.join("out");
        std::fs::create_dir_all(&incident_dir).expect("create incident dir");
        let bundle_doc = json!({
            "schema_version": "lp.incident.bundle@0.2.0",
            "incident_id": "lpinc_native_1",
            "meta": {
                "native_classification": "native_permission_blocked",
                "native_context": {
                    "platform": "ios",
                    "permission_state_snapshot": {
                        "location_foreground": "denied"
                    },
                    "lifecycle_state": "foreground",
                    "connectivity_state": "offline"
                }
            }
        });
        let request_doc = json!({
            "schema_version": "lp.regression.request@0.2.0",
            "regression_id": "lprgr_native_1",
            "invariants": ["device.trace.replay"],
            "native_replay_hints": {
                "host_target": "ios",
                "prelude": [
                    { "kind": "lifecycle.foreground" },
                    { "kind": "connectivity.offline" }
                ],
                "native_sequence": [
                    {
                        "request_id": "req_1",
                        "op": "permissions.request",
                        "event_class": "policy.violation",
                        "result": {
                            "status": "denied",
                            "permission": "location_foreground"
                        }
                    }
                ]
            }
        });
        std::fs::write(
            incident_dir.join("incident.bundle.json"),
            report::canon::canonical_pretty_json_bytes(&bundle_doc).expect("bundle bytes"),
        )
        .expect("write bundle");
        std::fs::write(
            incident_dir.join("regression.request.json"),
            report::canon::canonical_pretty_json_bytes(&request_doc).expect("request bytes"),
        )
        .expect("write request");

        let report_out = tmp.join("report.json");
        let machine = MachineArgs {
            json: Some(String::new()),
            report_json: None,
            report_out: Some(report_out.clone()),
            quiet_json: true,
            json_schema: false,
            json_schema_id: false,
        };

        let exit_code = cmd_device_regress_from_incident(
            &[
                OsString::from("x07-wasm"),
                OsString::from("device"),
                OsString::from("regress"),
                OsString::from("from-incident"),
            ],
            Scope::DeviceRegressFromIncident,
            &machine,
            DeviceRegressFromIncidentArgs {
                incident_dir: incident_dir.clone(),
                out_dir: out_dir.clone(),
                name: "native_case".to_string(),
                dry_run: false,
                strict: false,
            },
        )
        .expect("device regress");
        assert_eq!(exit_code, 0);

        let report_doc: Value =
            serde_json::from_slice(&std::fs::read(&report_out).expect("read report"))
                .expect("parse report");
        assert_eq!(
            report_doc
                .pointer("/result/replay_target_kind")
                .and_then(Value::as_str),
            Some("ios")
        );
        assert_eq!(
            report_doc
                .pointer("/result/replay_mode")
                .and_then(Value::as_str),
            Some("platform_native_v1")
        );
        assert_eq!(
            report_doc
                .pointer("/result/native_replay_hints/host_target")
                .and_then(Value::as_str),
            Some("ios")
        );
        assert!(out_dir.join("native_case.native.replay.json").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn device_regress_keeps_legacy_incident_support() {
        let tmp = tmp_dir("legacy");
        let incident_dir = tmp.join("incident");
        let out_dir = tmp.join("out");
        std::fs::create_dir_all(&incident_dir).expect("create incident dir");
        let trace_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/x07_capture_min/tests/web_ui/success.trace.json");
        let trace_doc: Value =
            serde_json::from_slice(&std::fs::read(&trace_path).expect("read trace fixture"))
                .expect("parse trace fixture");
        let incident_doc = json!({
            "v": 1,
            "kind": "x07.web_ui.incident",
            "trace": trace_doc
        });
        std::fs::write(
            incident_dir.join("incident.json"),
            report::canon::canonical_pretty_json_bytes(&incident_doc).expect("incident bytes"),
        )
        .expect("write incident");

        let report_out = tmp.join("report.json");
        let machine = MachineArgs {
            json: Some(String::new()),
            report_json: None,
            report_out: Some(report_out.clone()),
            quiet_json: true,
            json_schema: false,
            json_schema_id: false,
        };

        let exit_code = cmd_device_regress_from_incident(
            &[
                OsString::from("x07-wasm"),
                OsString::from("device"),
                OsString::from("regress"),
                OsString::from("from-incident"),
            ],
            Scope::DeviceRegressFromIncident,
            &machine,
            DeviceRegressFromIncidentArgs {
                incident_dir: incident_dir.clone(),
                out_dir: out_dir.clone(),
                name: "legacy_case".to_string(),
                dry_run: false,
                strict: false,
            },
        )
        .expect("device regress");
        assert_eq!(exit_code, 0);

        let report_doc: Value =
            serde_json::from_slice(&std::fs::read(&report_out).expect("read report"))
                .expect("parse report");
        assert_eq!(
            report_doc
                .pointer("/result/replay_mode")
                .and_then(Value::as_str),
            Some("legacy_web_ui_incident_v1")
        );
        assert!(out_dir.join("legacy_case.trace.json").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

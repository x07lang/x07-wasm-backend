use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder, Val};

use crate::cli::{MachineArgs, RunArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::machine::{self, JsonMode};
use crate::schema::SchemaStore;
use crate::util;
use crate::wasm::{abi_solve_v2, incident, inspect, memory_plan};

#[derive(Debug)]
struct HostState {
    limits: StoreLimits,
}

pub fn cmd_run(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: RunArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let json_mode = machine::json_mode(machine).map_err(anyhow::Error::msg)?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let loaded_profile = match crate::arch::load_profile(
        &store,
        &args.index,
        args.profile.as_deref(),
        args.profile_file.as_ref(),
    ) {
        Ok(p) => p,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            let report_doc = run_report_doc(meta, diagnostics, run_result_placeholder());
            return emit_run_report(&store, scope, machine, json_mode, report_doc, None);
        }
    };

    meta.inputs.push(loaded_profile.digest.clone());
    if let Some(d) = loaded_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let profile = loaded_profile.doc;
    let profile_ref = json!({ "id": profile.id, "v": profile.v });

    let input = match load_input_bytes(&args, &mut meta, &mut diagnostics) {
        Ok(v) => v,
        Err(_) => {
            let report_doc = run_report_doc(meta, diagnostics, run_result_placeholder());
            return emit_run_report(&store, scope, machine, json_mode, report_doc, None);
        }
    };

    let arena_cap_bytes = args
        .arena_cap_bytes
        .unwrap_or(profile.defaults.arena_cap_bytes);
    let max_output_bytes = args
        .max_output_bytes
        .unwrap_or(profile.defaults.max_output_bytes);

    let wasm_bytes = match std::fs::read(&args.wasm) {
        Ok(b) => b,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read wasm {}: {err:#}", args.wasm.display()),
            ));
            let report_doc = run_report_doc(meta, diagnostics, run_result_placeholder());
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };
    let wasm_digest = report::meta::FileDigest {
        path: args.wasm.display().to_string(),
        sha256: util::sha256_hex(&wasm_bytes),
        bytes_len: wasm_bytes.len() as u64,
    };
    meta.inputs.push(wasm_digest.clone());

    let wasm_info = match inspect::inspect(&wasm_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_INSPECT_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to inspect wasm: {err:#}"),
            ));
            let report_doc = run_report_doc(meta, diagnostics, run_result_placeholder());
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    let mem = match wasm_info.memory.clone() {
        Some(m) => m,
        None => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_NO_MEMORY",
                Severity::Error,
                Stage::Run,
                "wasm module has no memory section".to_string(),
            ));
            inspect::WasmMemoryPlan {
                initial_pages: 0,
                max_pages: Some(0),
            }
        }
    };
    let initial_bytes = mem.initial_bytes();
    let max_bytes = mem.max_bytes().unwrap_or(initial_bytes);
    let growable = mem.growable();

    let mem_plan = memory_plan::memory_plan_from_ldflags_and_wasm(
        &profile.wasm_ld.ldflags,
        initial_bytes,
        max_bytes,
        growable,
    );
    let expected = memory_plan::memory_expectations_from_ldflags(&profile.wasm_ld.ldflags);
    if let Some(exp) = expected.initial_memory_bytes {
        if exp != mem_plan.initial_memory_bytes {
            diagnostics.push(Diagnostic::new(
                "X07WASM_MEMORY_INITIAL_MISMATCH",
                Severity::Error,
                Stage::Run,
                format!(
                    "wasm initial memory mismatch: expected={} actual={}",
                    exp, mem_plan.initial_memory_bytes
                ),
            ));
        }
    }
    if let Some(exp) = expected.max_memory_bytes {
        if exp != mem_plan.max_memory_bytes {
            diagnostics.push(Diagnostic::new(
                "X07WASM_MEMORY_MAX_MISMATCH",
                Severity::Error,
                Stage::Run,
                format!(
                    "wasm max memory mismatch: expected={} actual={}",
                    exp, mem_plan.max_memory_bytes
                ),
            ));
        }
    }
    if expected.no_growable_memory && mem_plan.growable_memory {
        diagnostics.push(Diagnostic::new(
            "X07WASM_MEMORY_GROWABLE",
            Severity::Error,
            Stage::Run,
            "growable memory is not allowed for Phase 0 deterministic runs".to_string(),
        ));
    }

    // Preflight: required exports.
    for name in ["x07_solve_v2", "memory", "__heap_base", "__data_end"] {
        if !wasm_info.exports.iter().any(|x| x == name) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_EXPORTS_MISSING",
                Severity::Error,
                Stage::Run,
                format!("missing required export: {name}"),
            ));
        }
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        let result = run_result_doc(
            &profile_ref,
            &wasm_digest,
            &input,
            arena_cap_bytes,
            None,
            None,
            None,
            None,
            Some(mem_plan.clone()),
        );
        let report_doc = run_report_doc(meta, diagnostics, result);
        return emit_run_report(
            &store,
            scope,
            machine,
            json_mode,
            report_doc,
            Some(input.bytes),
        );
    }

    let stack_limit = if mem_plan.stack_size_bytes != 0 {
        mem_plan.stack_size_bytes
    } else {
        1024 * 1024
    };

    let mut config = Config::new();
    config.max_wasm_stack(stack_limit as usize);
    let engine = match Engine::new(&config) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASMTIME_ENGINE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    if Module::validate(&engine, &wasm_bytes).is_err() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WASM_INVALID",
            Severity::Error,
            Stage::Run,
            "wasm module failed validation".to_string(),
        ));
        let result = run_result_doc(
            &profile_ref,
            &wasm_digest,
            &input,
            arena_cap_bytes,
            None,
            None,
            None,
            None,
            Some(mem_plan.clone()),
        );
        let report_doc = run_report_doc(meta, diagnostics, result);
        return emit_run_report(
            &store,
            scope,
            machine,
            json_mode,
            report_doc,
            Some(input.bytes),
        );
    }

    let module = match Module::new(&engine, &wasm_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_COMPILE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };
    if module.imports().next().is_some() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WASM_IMPORTS_UNSUPPORTED",
            Severity::Error,
            Stage::Run,
            "wasm module has imports (Phase 0 runner expects no imports)".to_string(),
        ));
        let result = run_result_doc(
            &profile_ref,
            &wasm_digest,
            &input,
            arena_cap_bytes,
            None,
            None,
            None,
            None,
            Some(mem_plan.clone()),
        );
        let report_doc = run_report_doc(meta, diagnostics, result);
        return emit_run_report(
            &store,
            scope,
            machine,
            json_mode,
            report_doc,
            Some(input.bytes),
        );
    }

    let max_store_memory = max_bytes.min(u64::from(u32::MAX));
    let limits = StoreLimitsBuilder::new()
        .memory_size(max_store_memory as usize)
        .build();
    let mut store_rt = Store::new(&engine, HostState { limits });
    store_rt.limiter(|s| &mut s.limits);

    let instance = match Instance::new(&mut store_rt, &module, &[]) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_INSTANTIATE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    let Some(memory) = instance.get_memory(&mut store_rt, "memory") else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_EXPORT_MISSING_MEMORY",
            Severity::Error,
            Stage::Run,
            "missing export: memory".to_string(),
        ));
        let result = run_result_doc(
            &profile_ref,
            &wasm_digest,
            &input,
            arena_cap_bytes,
            None,
            None,
            None,
            None,
            Some(mem_plan.clone()),
        );
        let report_doc = run_report_doc(meta, diagnostics, result);
        return emit_run_report(
            &store,
            scope,
            machine,
            json_mode,
            report_doc,
            Some(input.bytes),
        );
    };
    let func = match instance
        .get_typed_func::<(i32, i32, i32, i32, i32), ()>(&mut store_rt, "x07_solve_v2")
    {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_EXPORT_INVALID_X07_SOLVE_V2",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    let heap_base = match read_global_u32(&mut store_rt, &instance, "__heap_base") {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_EXPORT_MISSING_HEAP_BASE",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };
    let data_end = match read_global_u32(&mut store_rt, &instance, "__data_end") {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_EXPORT_MISSING_DATA_END",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    let arena_cap_u32 = match u32::try_from(arena_cap_bytes) {
        Ok(v) => v,
        Err(_) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_ARENA_CAP_TOO_LARGE",
                Severity::Error,
                Stage::Parse,
                format!("arena_cap_bytes too large for wasm32: {arena_cap_bytes}"),
            ));
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                None,
                None,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };
    let max_output_u32 = match u32::try_from(max_output_bytes) {
        Ok(v) => v,
        Err(_) => u32::MAX,
    };

    let call_res = match abi_solve_v2::call_solve_v2(
        &mut store_rt,
        &memory,
        &func,
        heap_base,
        data_end,
        &input.bytes,
        arena_cap_u32,
        max_output_u32,
    ) {
        Ok(v) => v,
        Err(err) => {
            let mut trap: Option<String> = None;

            if let Some(limit) = err.downcast_ref::<abi_solve_v2::OutputLimitExceeded>() {
                let mut d = Diagnostic::new(
                    "X07WASM_BUDGET_EXCEEDED_OUTPUT",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "output exceeded cap: out_len={} max_output_bytes={}",
                        limit.out_len, limit.max_output_bytes
                    ),
                );
                d.data.insert("out_len".to_string(), json!(limit.out_len));
                d.data.insert(
                    "max_output_bytes".to_string(),
                    json!(limit.max_output_bytes),
                );
                diagnostics.push(d);
            } else if let Some(mem) = err.downcast_ref::<abi_solve_v2::LinearMemoryTooSmall>() {
                let mut d = Diagnostic::new(
                    "X07WASM_BUDGET_EXCEEDED_MEMORY",
                    Severity::Error,
                    Stage::Run,
                    mem.to_string(),
                );
                d.data
                    .insert("need_bytes".to_string(), json!(mem.need_bytes));
                d.data
                    .insert("have_bytes".to_string(), json!(mem.have_bytes));
                diagnostics.push(d);
            } else {
                let msg = format!("{err:#}");
                diagnostics.push(Diagnostic::new(
                    "X07WASM_RUN_FAILED",
                    Severity::Error,
                    Stage::Run,
                    msg.clone(),
                ));
                if msg.to_ascii_lowercase().contains("wasm trap") {
                    trap = Some(msg);
                }
            }
            let result = run_result_doc(
                &profile_ref,
                &wasm_digest,
                &input,
                arena_cap_bytes,
                Some(call_memory_doc(
                    Some(mem_plan.clone()),
                    heap_base,
                    data_end,
                    None,
                )),
                trap,
                None,
                None,
                Some(mem_plan.clone()),
            );
            let report_doc = run_report_doc(meta, diagnostics, result);
            return emit_run_report(
                &store,
                scope,
                machine,
                json_mode,
                report_doc,
                Some(input.bytes),
            );
        }
    };

    let output_len = call_res.output.len();
    let output_sha = util::sha256_hex(&call_res.output);

    let mut output_blob_obj = serde_json::Map::new();
    output_blob_obj.insert("bytes_len".to_string(), json!(output_len as u64));
    output_blob_obj.insert("sha256".to_string(), json!(output_sha.clone()));

    let mut output_written = false;
    if let Some(path) = args.output_out.as_ref() {
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir: {}", parent.display()))
            {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_OUTPUT_IO_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to create output dir {}: {err:#}", parent.display()),
                ));
            }
        }

        if diagnostics.iter().all(|d| d.severity != Severity::Error) {
            match std::fs::write(path, &call_res.output)
                .with_context(|| format!("write: {}", path.display()))
            {
                Ok(()) => {
                    output_written = true;
                    output_blob_obj.insert("path".to_string(), json!(path.display().to_string()));
                    meta.outputs.push(report::meta::FileDigest {
                        path: path.display().to_string(),
                        sha256: output_sha.clone(),
                        bytes_len: output_len as u64,
                    });
                }
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_OUTPUT_WRITE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("failed to write output {}: {err:#}", path.display()),
                    ));
                }
            }
        }
    } else if json_mode == JsonMode::Off {
        std::io::Write::write_all(&mut std::io::stdout(), &call_res.output)
            .context("write stdout")?;
    } else {
        const INLINE_OUTPUT_MAX_BYTES: usize = 4096;
        if output_len <= INLINE_OUTPUT_MAX_BYTES {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&call_res.output);
            output_blob_obj.insert("base64".to_string(), json!(b64));
        }
    }

    if args.output_out.is_some() && !output_written && json_mode != JsonMode::Off {
        const INLINE_OUTPUT_MAX_BYTES: usize = 4096;
        if output_len <= INLINE_OUTPUT_MAX_BYTES {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&call_res.output);
            output_blob_obj.insert("base64".to_string(), json!(b64));
        }
    }

    let output_blob = Value::Object(output_blob_obj);

    let memory_doc = call_memory_doc(
        Some(mem_plan.clone()),
        call_res.layout.heap_base,
        call_res.layout.data_end,
        Some(&call_res.layout),
    );

    let result = run_result_doc(
        &profile_ref,
        &wasm_digest,
        &input,
        arena_cap_bytes,
        Some(memory_doc),
        None,
        Some(output_blob),
        None,
        Some(mem_plan),
    );
    let report_doc = run_report_doc(meta, diagnostics, result);
    emit_run_report(
        &store,
        scope,
        machine,
        json_mode,
        report_doc,
        Some(input.bytes),
    )
}

struct InputBytes {
    bytes: Vec<u8>,
    blob_ref: Value,
}

fn load_input_bytes(
    args: &RunArgs,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<InputBytes> {
    if let Some(path) = args.input.as_ref() {
        let bytes = match std::fs::read(path).with_context(|| format!("read: {}", path.display())) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_INPUT_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to read input {}: {err:#}", path.display()),
                ));
                anyhow::bail!("failed to read input")
            }
        };
        let len = bytes.len();
        let sha = util::sha256_hex(&bytes);
        meta.inputs.push(report::meta::FileDigest {
            path: path.display().to_string(),
            sha256: sha.clone(),
            bytes_len: len as u64,
        });
        return Ok(InputBytes {
            bytes,
            blob_ref: json!({ "bytes_len": len as u64, "sha256": sha, "path": path.display().to_string() }),
        });
    }

    if let Some(hex_s) = args.input_hex.as_deref() {
        let bytes = match hex::decode(hex_s.trim()) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_INPUT_HEX_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to decode --input-hex: {err}"),
                ));
                anyhow::bail!("invalid hex input")
            }
        };
        let len = bytes.len();
        let sha = util::sha256_hex(&bytes);
        return Ok(InputBytes {
            bytes,
            blob_ref: json!({ "bytes_len": len as u64, "sha256": sha }),
        });
    }

    if let Some(b64) = args.input_base64.as_deref() {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_INPUT_BASE64_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to decode --input-base64: {err}"),
                ));
                anyhow::bail!("invalid base64 input")
            }
        };
        let len = bytes.len();
        let sha = util::sha256_hex(&bytes);
        let canon_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(InputBytes {
            bytes,
            blob_ref: json!({ "bytes_len": len as u64, "sha256": sha, "base64": canon_b64 }),
        });
    }

    diagnostics.push(Diagnostic::new(
        "X07WASM_INPUT_MISSING",
        Severity::Error,
        Stage::Parse,
        "expected exactly one of: --input, --input-hex, --input-base64".to_string(),
    ));
    anyhow::bail!("missing input")
}

fn read_global_u32<T>(store: &mut Store<T>, instance: &Instance, name: &str) -> Result<u32> {
    let g = instance
        .get_global(&mut *store, name)
        .ok_or_else(|| anyhow::anyhow!("missing export: {name}"))?;
    let v = g.get(&mut *store);
    match v {
        Val::I32(x) => Ok(x as u32),
        _ => anyhow::bail!("global {name} is not i32"),
    }
}

fn call_memory_doc(
    plan: Option<memory_plan::MemoryPlan>,
    heap_base: u32,
    data_end: u32,
    layout: Option<&abi_solve_v2::SolveV2MemoryLayout>,
) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(p) = plan {
        obj.insert("plan".to_string(), serde_json::to_value(p).unwrap());
    }
    obj.insert("heap_base".to_string(), json!(heap_base));
    obj.insert("data_end".to_string(), json!(data_end));
    if let Some(l) = layout {
        obj.insert("retptr".to_string(), json!(l.retptr));
        obj.insert("input_ptr".to_string(), json!(l.input_ptr));
        obj.insert("arena_ptr".to_string(), json!(l.arena_ptr));
    }
    Value::Object(obj)
}

fn run_result_placeholder() -> Value {
    json!({
      "profile": { "id": "wasm_release", "v": 1 },
      "wasm": { "path": "dist/unknown.wasm", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "bytes_len": 0 },
      "input": { "bytes_len": 0 },
      "arena_cap_bytes": 0,
      "output": { "bytes_len": 0 },
      "trap": null,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_result_doc(
    profile_ref: &Value,
    wasm: &report::meta::FileDigest,
    input: &InputBytes,
    arena_cap_bytes: u64,
    memory: Option<Value>,
    trap: Option<String>,
    output: Option<Value>,
    engine_version: Option<String>,
    plan: Option<memory_plan::MemoryPlan>,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("profile".to_string(), profile_ref.clone());
    obj.insert("wasm".to_string(), serde_json::to_value(wasm).unwrap());
    obj.insert("input".to_string(), input.blob_ref.clone());
    obj.insert("arena_cap_bytes".to_string(), json!(arena_cap_bytes));

    let output_doc = output.unwrap_or_else(|| json!({ "bytes_len": 0, "sha256": "0000000000000000000000000000000000000000000000000000000000000000" }));
    obj.insert("output".to_string(), output_doc);

    obj.insert(
        "trap".to_string(),
        match trap {
            Some(t) => json!(t),
            None => Value::Null,
        },
    );

    if let Some(v) = engine_version {
        obj.insert(
            "engine".to_string(),
            json!({ "name": "wasmtime", "version": v }),
        );
    }
    if let Some(m) = memory {
        obj.insert("memory".to_string(), m);
    } else if let Some(p) = plan {
        obj.insert("memory".to_string(), json!({ "plan": p }));
    }

    Value::Object(obj)
}

fn run_report_doc(
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    result: Value,
) -> Value {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    json!({
      "schema_version": "x07.wasm.run.report@0.1.0",
      "command": "x07-wasm.run",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": result,
    })
}

fn emit_run_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    json_mode: JsonMode,
    mut report_doc: Value,
    maybe_input_bytes: Option<Vec<u8>>,
) -> Result<u8> {
    let exit_code = report_doc
        .get("exit_code")
        .and_then(Value::as_u64)
        .unwrap_or(2) as u8;

    let ok = report_doc
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ok {
        if let Some(input_bytes) = maybe_input_bytes.as_ref() {
            if let Err(err) = maybe_write_incident_bundle(&report_doc, input_bytes) {
                if let Some(arr) = report_doc
                    .get_mut("diagnostics")
                    .and_then(Value::as_array_mut)
                {
                    arr.push(serde_json::to_value(Diagnostic::new(
                        "X07WASM_INCIDENT_BUNDLE_WRITE_FAILED",
                        Severity::Warning,
                        Stage::Run,
                        format!("{err:#}"),
                    ))?);
                }
            }
        }
    }

    store.validate_report_and_emit(scope, machine, std::time::Instant::now(), &[], report_doc)?;

    let _ = json_mode;
    Ok(exit_code)
}

fn maybe_write_incident_bundle(report_doc: &Value, input_bytes: &[u8]) -> Result<()> {
    let Some(result) = report_doc.get("result").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(wasm_sha) = result
        .get("wasm")
        .and_then(|v| v.get("sha256"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };

    let input_sha = util::sha256_hex(input_bytes);
    let run_id = incident::incident_run_id(wasm_sha, &input_sha);
    let date = incident::utc_date_yyyy_mm_dd();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let dir = incident::incident_dir(&cwd, &date, &run_id);

    let report_bytes = report::canon::canonical_json_bytes(report_doc)?;

    let manifest_bytes = guess_manifest_bytes(result.get("wasm"))?;
    let stderr = trap_and_diagnostics_text(report_doc);

    incident::write_incident_bundle(
        &dir,
        input_bytes,
        &report_bytes,
        manifest_bytes.as_deref(),
        stderr.as_deref(),
    )?;
    Ok(())
}

fn guess_manifest_bytes(wasm_doc: Option<&Value>) -> Result<Option<Vec<u8>>> {
    let Some(path) = wasm_doc.and_then(|v| v.get("path")).and_then(Value::as_str) else {
        return Ok(None);
    };
    // Prefer "<wasm>.manifest.json" (x07-wasm build default).
    let manifest_path = format!("{path}.manifest.json");
    let manifest = Path::new(&manifest_path);
    if !manifest.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(manifest).with_context(|| format!("read: {}", manifest.display()))?;
    Ok(Some(bytes))
}

fn trap_and_diagnostics_text(report_doc: &Value) -> Option<String> {
    let mut out = String::new();
    if let Some(trap) = report_doc
        .get("result")
        .and_then(|r| r.get("trap"))
        .and_then(Value::as_str)
    {
        out.push_str(trap);
        out.push('\n');
    }
    if let Some(diags) = report_doc.get("diagnostics").and_then(Value::as_array) {
        for d in diags {
            if let Some(msg) = d.get("message").and_then(Value::as_str) {
                out.push_str(msg);
                out.push('\n');
            }
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use wasmtime::{Config, Engine, Instance, Module, Store, Val};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::wasm::abi_solve_v2;
use crate::wasmtime_limits::{self, WasmResourceLimiter};

#[derive(Debug, Clone, Deserialize)]
struct WebUiProfileDoc {
    defaults: WebUiProfileDefaults,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiProfileDefaults {
    arena_cap_bytes: u64,
    max_output_bytes: u64,
}

pub fn load_web_ui_budgets(
    store: &SchemaStore,
    dist_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> (u32, u32) {
    let profile_path = dist_dir.join("web-ui.profile.json");
    if !profile_path.is_file() {
        // Fallback defaults that match the Phase 2 arch profiles.
        return (32 * 1024 * 1024, 2 * 1024 * 1024);
    }

    if let Ok(d) = util::file_digest(&profile_path) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(&profile_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_PROFILE_READ_FAILED",
                Severity::Warning,
                Stage::Parse,
                format!("failed to read web-ui.profile.json: {err}"),
            ));
            return (32 * 1024 * 1024, 2 * 1024 * 1024);
        }
    };
    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_PROFILE_JSON_INVALID",
                Severity::Warning,
                Stage::Parse,
                format!("web-ui.profile.json is not JSON: {err}"),
            ));
            return (32 * 1024 * 1024, 2 * 1024 * 1024);
        }
    };
    let _ = store
        .validate(
            "https://x07.io/spec/x07-web_ui.profile.schema.json",
            &doc_json,
        )
        .map(|diags| diagnostics.extend(diags));
    let parsed: WebUiProfileDoc = match serde_json::from_value(doc_json) {
        Ok(v) => v,
        Err(_) => {
            return (32 * 1024 * 1024, 2 * 1024 * 1024);
        }
    };

    let arena = u32::try_from(parsed.defaults.arena_cap_bytes).unwrap_or(32 * 1024 * 1024);
    let max_out = u32::try_from(parsed.defaults.max_output_bytes).unwrap_or(2 * 1024 * 1024);
    (arena, max_out)
}

pub fn load_wasm_runtime_limits(
    store: &SchemaStore,
    dist_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<crate::arch::WasmRuntimeLimits> {
    let profile_path = dist_dir.join("wasm.profile.json");
    if !profile_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("missing wasm profile: {}", profile_path.display()),
        ));
        return None;
    }

    if let Ok(d) = util::file_digest(&profile_path) {
        meta.inputs.push(d);
    }
    let bytes = match std::fs::read(&profile_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read wasm profile {}: {err}",
                    profile_path.display()
                ),
            ));
            return None;
        }
    };

    let doc_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
                Severity::Error,
                Stage::Parse,
                format!("wasm profile is not JSON: {err}"),
            ));
            return None;
        }
    };

    let diags = match store.validate(
        "https://x07.io/spec/x07-wasm.profile.schema.json",
        &doc_json,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return None;
        }
    };
    if !diags.is_empty() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
            Severity::Error,
            Stage::Parse,
            "wasm profile schema invalid".to_string(),
        ));
        diagnostics.extend(diags);
        return None;
    }

    let parsed: crate::arch::WasmProfileDoc = match serde_json::from_value(doc_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse wasm profile: {err}"),
            ));
            return None;
        }
    };

    Some(parsed.runtime)
}

#[derive(Debug)]
struct CoreHostState {
    limiter: WasmResourceLimiter,
}

pub struct CoreWasmRunner {
    pub wasm: report::meta::FileDigest,
    store: Store<CoreHostState>,
    memory: wasmtime::Memory,
    func: wasmtime::TypedFunc<(i32, i32, i32, i32, i32), ()>,
    heap_base: u32,
    data_end: u32,
    arena_cap_bytes: u32,
    max_output_bytes: u32,
}

impl CoreWasmRunner {
    pub fn new(
        wasm_path: &Path,
        runtime_limits: &crate::arch::WasmRuntimeLimits,
        arena_cap_bytes: u32,
        max_output_bytes: u32,
        meta: &mut report::meta::ReportMeta,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<Self> {
        let wasm_bytes =
            std::fs::read(wasm_path).with_context(|| format!("read: {}", wasm_path.display()))?;
        let wasm_digest = report::meta::FileDigest {
            path: wasm_path.display().to_string(),
            sha256: util::sha256_hex(&wasm_bytes),
            bytes_len: wasm_bytes.len() as u64,
        };
        meta.inputs.push(wasm_digest.clone());

        let mut runtime_limits = runtime_limits.clone();
        if runtime_limits.max_wasm_stack_bytes.is_none() {
            runtime_limits.max_wasm_stack_bytes = Some(2 * 1024 * 1024);
        }

        let mut config = Config::new();
        wasmtime_limits::apply_config(&mut config, &runtime_limits);
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, &wasm_bytes)?;

        let mut store = Store::new(
            &engine,
            CoreHostState {
                limiter: WasmResourceLimiter::new(
                    runtime_limits.max_memory_bytes,
                    runtime_limits.max_table_elements,
                ),
            },
        );
        store.limiter(|s| &mut s.limiter);
        wasmtime_limits::store_add_fuel(&mut store, &runtime_limits)?;
        let instance = Instance::new(&mut store, &module, &[])?;

        let Some(memory) = instance.get_memory(&mut store, "memory") else {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_TEST_EXPORT_MISSING_MEMORY",
                Severity::Error,
                Stage::Run,
                "missing export: memory".to_string(),
            ));
            anyhow::bail!("missing wasm export memory");
        };

        let func = instance
            .get_typed_func::<(i32, i32, i32, i32, i32), ()>(&mut store, "x07_solve_v2")
            .context("get export x07_solve_v2")?;
        let heap_base = read_global_u32(&mut store, &instance, "__heap_base")?;
        let data_end = read_global_u32(&mut store, &instance, "__data_end")?;

        Ok(Self {
            wasm: wasm_digest,
            store,
            memory,
            func,
            heap_base,
            data_end,
            arena_cap_bytes,
            max_output_bytes,
        })
    }

    pub fn call(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        let out = abi_solve_v2::call_solve_v2(
            &mut self.store,
            &self.memory,
            &self.func,
            self.heap_base,
            self.data_end,
            input,
            self.arena_cap_bytes,
            self.max_output_bytes,
        )?;
        Ok(out.output)
    }
}

fn read_global_u32<T>(store: &mut Store<T>, instance: &Instance, name: &str) -> Result<u32> {
    let Some(export) = instance.get_export(&mut *store, name) else {
        anyhow::bail!("missing global export {name:?}");
    };
    match export.into_global() {
        Some(g) => match g.get(&mut *store) {
            Val::I32(x) => Ok(x as u32),
            _ => anyhow::bail!("global {name:?} is not i32"),
        },
        None => anyhow::bail!("export {name:?} is not a global"),
    }
}

pub fn case_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "case".to_string())
}

pub fn canonical_json_bytes_no_newline(v: &Value) -> Result<Vec<u8>> {
    let mut vv = v.clone();
    util::canon_value_jcs(&mut vv);
    Ok(serde_json::to_vec(&vv)?)
}

pub fn apply_json_patch(mut doc: Value, patchset: &Value) -> Result<Value> {
    let ops = patchset.as_array().cloned().unwrap_or_default();
    for op in ops {
        let kind = op.get("op").and_then(Value::as_str).unwrap_or("");
        let path = op.get("path").and_then(Value::as_str).unwrap_or("");
        let tokens = parse_json_pointer(path)?;
        match kind {
            "add" | "replace" => {
                let value = op.get("value").cloned().unwrap_or(Value::Null);
                apply_set(&mut doc, &tokens, value, kind == "add")?;
            }
            "remove" => apply_remove(&mut doc, &tokens)?,
            "copy" => {
                let from = op.get("from").and_then(Value::as_str).unwrap_or("");
                let from_tokens = parse_json_pointer(from)?;
                let value = navigate(&doc, &from_tokens)?.clone();
                apply_set(&mut doc, &tokens, value, true)?;
            }
            "move" => {
                let from = op.get("from").and_then(Value::as_str).unwrap_or("");
                let from_tokens = parse_json_pointer(from)?;
                let value = navigate(&doc, &from_tokens)?.clone();
                apply_remove(&mut doc, &from_tokens)?;
                apply_set(&mut doc, &tokens, value, true)?;
            }
            "test" => {
                let want = op.get("value").cloned().unwrap_or(Value::Null);
                let got = navigate(&doc, &tokens)?;
                if got != &want {
                    anyhow::bail!("test failed at {path:?}");
                }
            }
            _ => anyhow::bail!("unsupported op: {kind:?}"),
        }
    }
    Ok(doc)
}

fn parse_json_pointer(path: &str) -> Result<Vec<String>> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        anyhow::bail!("invalid JSON Pointer: {path:?}");
    }
    Ok(path[1..]
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect())
}

fn apply_set(doc: &mut Value, tokens: &[String], value: Value, is_add: bool) -> Result<()> {
    if tokens.is_empty() {
        *doc = value;
        return Ok(());
    }
    let (parent_tokens, last) = tokens.split_at(tokens.len() - 1);
    let parent = navigate_mut(doc, parent_tokens)?;
    let last = &last[0];
    if let Some(obj) = parent.as_object_mut() {
        if !is_add && !obj.contains_key(last) {
            anyhow::bail!("replace key missing: {last:?}");
        }
        obj.insert(last.clone(), value);
        return Ok(());
    }
    if let Some(arr) = parent.as_array_mut() {
        if last == "-" {
            arr.push(value);
            return Ok(());
        }
        let idx: usize = last.parse().context("array index")?;
        if is_add {
            if idx > arr.len() {
                anyhow::bail!("add index out of bounds: {idx}");
            }
            arr.insert(idx, value);
        } else if idx >= arr.len() {
            anyhow::bail!("replace index out of bounds: {idx}");
        } else {
            arr[idx] = value;
        }
        return Ok(());
    }
    anyhow::bail!("invalid parent for set")
}

fn apply_remove(doc: &mut Value, tokens: &[String]) -> Result<()> {
    if tokens.is_empty() {
        anyhow::bail!("remove root is not supported");
    }
    let (parent_tokens, last) = tokens.split_at(tokens.len() - 1);
    let parent = navigate_mut(doc, parent_tokens)?;
    let last = &last[0];
    if let Some(obj) = parent.as_object_mut() {
        if obj.remove(last).is_none() {
            anyhow::bail!("remove key missing: {last:?}");
        }
        return Ok(());
    }
    if let Some(arr) = parent.as_array_mut() {
        let idx: usize = last.parse().context("array index")?;
        if idx >= arr.len() {
            anyhow::bail!("remove index out of bounds: {idx}");
        }
        arr.remove(idx);
        return Ok(());
    }
    anyhow::bail!("invalid parent for remove")
}

fn navigate_mut<'a>(doc: &'a mut Value, tokens: &[String]) -> Result<&'a mut Value> {
    if tokens.is_empty() {
        return Ok(doc);
    }
    let t = &tokens[0];
    match doc {
        Value::Object(map) => {
            let child = map
                .get_mut(t)
                .ok_or_else(|| anyhow::anyhow!("missing key in path: {t:?}"))?;
            navigate_mut(child, &tokens[1..])
        }
        Value::Array(arr) => {
            let idx: usize = t.parse().context("array index")?;
            let child = arr
                .get_mut(idx)
                .ok_or_else(|| anyhow::anyhow!("index out of bounds: {idx}"))?;
            navigate_mut(child, &tokens[1..])
        }
        _ => anyhow::bail!("invalid container in path at token {t:?}"),
    }
}

fn navigate<'a>(doc: &'a Value, tokens: &[String]) -> Result<&'a Value> {
    if tokens.is_empty() {
        return Ok(doc);
    }
    let t = &tokens[0];
    match doc {
        Value::Object(map) => {
            let child = map
                .get(t)
                .ok_or_else(|| anyhow::anyhow!("missing key in path: {t:?}"))?;
            navigate(child, &tokens[1..])
        }
        Value::Array(arr) => {
            let idx: usize = t.parse().context("array index")?;
            let child = arr
                .get(idx)
                .ok_or_else(|| anyhow::anyhow!("index out of bounds: {idx}"))?;
            navigate(child, &tokens[1..])
        }
        _ => anyhow::bail!("invalid container in path at token {t:?}"),
    }
}

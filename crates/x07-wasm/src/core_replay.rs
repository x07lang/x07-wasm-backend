use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use wasmtime::{Config, Engine, Instance, Module, Store, Val};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::util;
use crate::wasm::abi_solve_v2;
use crate::wasmtime_limits::{self, WasmResourceLimiter};

#[derive(Debug)]
struct CoreHostState {
    limiter: WasmResourceLimiter,
}

pub struct CoreWasmRunner {
    store: Store<CoreHostState>,
    memory: wasmtime::Memory,
    func: wasmtime::TypedFunc<(i32, i32, i32, i32, i32), ()>,
    heap_base: u32,
    data_end: u32,
    arena_cap_bytes: u32,
    max_output_bytes: u32,
    max_fuel: Option<u64>,
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
        meta.inputs.push(report::meta::FileDigest {
            path: wasm_path.display().to_string(),
            sha256: util::sha256_hex(&wasm_bytes),
            bytes_len: wasm_bytes.len() as u64,
        });

        let mut runtime_limits = runtime_limits.clone();
        if runtime_limits.max_wasm_stack_bytes.is_none() {
            runtime_limits.max_wasm_stack_bytes = Some(2 * 1024 * 1024);
        }

        let mut config = Config::new();
        wasmtime_limits::apply_config(&mut config, &runtime_limits)?;
        wasmtime_limits::apply_instance_allocator_config(&mut config, &runtime_limits, 1)?;
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
                "X07WASM_CORE_EXPORT_MISSING_MEMORY",
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
            store,
            memory,
            func,
            heap_base,
            data_end,
            arena_cap_bytes,
            max_output_bytes,
            max_fuel: runtime_limits.max_fuel,
        })
    }

    pub fn call(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        if let Some(max_fuel) = self.max_fuel {
            self.store.set_fuel(max_fuel)?;
        }
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

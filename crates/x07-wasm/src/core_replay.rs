use std::path::Path;

use anyhow::{Context, Result};
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

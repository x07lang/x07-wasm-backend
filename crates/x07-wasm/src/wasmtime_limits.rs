use anyhow::{Context, Result};
use std::path::Path;
use wasmtime::{Config, PoolingAllocationConfig};

use crate::arch::{WasmInstanceAllocator, WasmRuntimeLimits};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExceededKind {
    CpuFuel,
    WasmStack,
    Memory,
    Table,
}

#[derive(Debug)]
pub struct MemoryLimitExceeded {
    pub desired_bytes: usize,
    pub max_bytes: usize,
}

impl std::fmt::Display for MemoryLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "wasm memory budget exceeded: desired_bytes={} max_bytes={}",
            self.desired_bytes, self.max_bytes
        )
    }
}

impl std::error::Error for MemoryLimitExceeded {}

#[derive(Debug)]
pub struct TableLimitExceeded {
    pub desired_elements: usize,
    pub max_elements: usize,
}

impl std::fmt::Display for TableLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "wasm table budget exceeded: desired_elements={} max_elements={}",
            self.desired_elements, self.max_elements
        )
    }
}

impl std::error::Error for TableLimitExceeded {}

#[derive(Debug, Clone)]
pub struct WasmResourceLimiter {
    pub max_memory_bytes: Option<usize>,
    pub max_table_elements: Option<usize>,
}

impl WasmResourceLimiter {
    pub fn new(max_memory_bytes: Option<u64>, max_table_elements: Option<u32>) -> Self {
        let max_memory_bytes = max_memory_bytes
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0);
        let max_table_elements = max_table_elements
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0);
        Self {
            max_memory_bytes,
            max_table_elements,
        }
    }
}

impl wasmtime::ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool> {
        if let Some(max) = self.max_memory_bytes {
            if desired > max {
                return Err(anyhow::Error::new(MemoryLimitExceeded {
                    desired_bytes: desired,
                    max_bytes: max,
                }));
            }
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool> {
        if let Some(max) = self.max_table_elements {
            if desired > max {
                return Err(anyhow::Error::new(TableLimitExceeded {
                    desired_elements: desired,
                    max_elements: max,
                }));
            }
        }
        Ok(true)
    }
}

pub fn apply_config(config: &mut Config, limits: &WasmRuntimeLimits) -> Result<()> {
    if limits.max_fuel.is_some() {
        config.consume_fuel(true);
    }
    if let Some(stack) = limits.max_wasm_stack_bytes {
        if let Ok(stack_usize) = usize::try_from(stack) {
            config.max_wasm_stack(stack_usize);
        }
    }

    if let Some(path) = limits.cache_config.as_deref() {
        // Cache config does not affect deterministic semantics; it's a host performance knob.
        let cache = wasmtime::Cache::from_file(Some(Path::new(path)))
            .with_context(|| format!("load Wasmtime cache config: {path}"))?;
        config.cache(Some(cache));
    }

    Ok(())
}

pub fn store_add_fuel<T>(store: &mut wasmtime::Store<T>, limits: &WasmRuntimeLimits) -> Result<()> {
    if let Some(fuel) = limits.max_fuel {
        store.set_fuel(fuel)?;
    }
    Ok(())
}

pub fn classify_budget_exceeded(err: &anyhow::Error) -> Option<BudgetExceededKind> {
    for cause in err.chain() {
        if let Some(trap) = cause.downcast_ref::<wasmtime::Trap>() {
            match trap {
                wasmtime::Trap::OutOfFuel => return Some(BudgetExceededKind::CpuFuel),
                wasmtime::Trap::StackOverflow => return Some(BudgetExceededKind::WasmStack),
                _ => {}
            }
        }
        if cause.downcast_ref::<MemoryLimitExceeded>().is_some() {
            return Some(BudgetExceededKind::Memory);
        }
        if cause.downcast_ref::<TableLimitExceeded>().is_some() {
            return Some(BudgetExceededKind::Table);
        }
    }
    None
}

pub fn apply_instance_allocator_config(
    config: &mut Config,
    limits: &WasmRuntimeLimits,
    max_concurrency: usize,
) -> Result<bool> {
    if limits.instance_allocator != WasmInstanceAllocator::Pooling {
        return Ok(false);
    }

    let Ok(total_component_instances) = u32::try_from(max_concurrency.max(1)) else {
        anyhow::bail!("pooling allocator unsupported: max_concurrency too large");
    };
    let Some(max_memory_bytes) = limits.max_memory_bytes else {
        anyhow::bail!("pooling allocator requires runtime.max_memory_bytes");
    };
    let Some(max_table_elements) = limits.max_table_elements else {
        anyhow::bail!("pooling allocator requires runtime.max_table_elements");
    };
    let Ok(max_memory_size) = usize::try_from(max_memory_bytes) else {
        anyhow::bail!("pooling allocator unsupported: runtime.max_memory_bytes too large");
    };
    let Ok(table_elements) = usize::try_from(max_table_elements) else {
        anyhow::bail!("pooling allocator unsupported: runtime.max_table_elements too large");
    };
    if max_memory_size == 0 || table_elements == 0 {
        anyhow::bail!("pooling allocator requires non-zero memory/table limits");
    }

    let total_core_instances = total_component_instances.saturating_mul(16).max(1);

    let mut pooling = PoolingAllocationConfig::new();
    pooling
        .total_component_instances(total_component_instances)
        .total_core_instances(total_core_instances)
        .total_memories(total_core_instances)
        .total_tables(total_core_instances)
        .max_memories_per_module(1)
        .max_tables_per_module(1)
        .max_memory_size(max_memory_size)
        .table_elements(table_elements);
    config.allocation_strategy(pooling);
    Ok(true)
}

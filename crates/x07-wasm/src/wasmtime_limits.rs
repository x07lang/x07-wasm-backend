use anyhow::Result;
use wasmtime::Config;

use crate::arch::WasmRuntimeLimits;

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

pub fn apply_config(config: &mut Config, limits: &WasmRuntimeLimits) {
    if limits.max_fuel.is_some() {
        config.consume_fuel(true);
    }
    if let Some(stack) = limits.max_wasm_stack_bytes {
        if let Ok(stack_usize) = usize::try_from(stack) {
            config.max_wasm_stack(stack_usize);
        }
    }
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

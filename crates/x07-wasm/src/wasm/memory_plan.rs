use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MemoryPlan {
    pub stack_first: bool,
    pub stack_size_bytes: u64,
    pub initial_memory_bytes: u64,
    pub max_memory_bytes: u64,
    pub growable_memory: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryPlanExpectations {
    pub initial_memory_bytes: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub no_growable_memory: bool,
}

pub fn memory_plan_from_ldflags_and_wasm(
    ldflags: &[String],
    wasm_initial_bytes: u64,
    wasm_max_bytes: u64,
    wasm_growable: bool,
) -> MemoryPlan {
    let stack_first = ldflags.iter().any(|w| w == "--stack-first");

    let mut stack_size_bytes: u64 = 0;
    let mut i = 0usize;
    while i < ldflags.len() {
        let w = &ldflags[i];
        if w == "-z" {
            if let Some(next) = ldflags.get(i + 1) {
                if let Some(v) = next.strip_prefix("stack-size=") {
                    stack_size_bytes = v.parse::<u64>().unwrap_or(0);
                }
            }
        } else if let Some(v) = w.strip_prefix("stack-size=") {
            stack_size_bytes = v.parse::<u64>().unwrap_or(0);
        }
        i += 1;
    }

    MemoryPlan {
        stack_first,
        stack_size_bytes,
        initial_memory_bytes: wasm_initial_bytes,
        max_memory_bytes: wasm_max_bytes,
        growable_memory: wasm_growable,
    }
}

pub fn memory_expectations_from_ldflags(ldflags: &[String]) -> MemoryPlanExpectations {
    let mut exp = MemoryPlanExpectations::default();
    for w in ldflags {
        if let Some(v) = w.strip_prefix("--initial-memory=") {
            exp.initial_memory_bytes = v.parse::<u64>().ok();
        } else if let Some(v) = w.strip_prefix("--max-memory=") {
            exp.max_memory_bytes = v.parse::<u64>().ok();
        } else if w == "--no-growable-memory" {
            exp.no_growable_memory = true;
        }
    }
    exp
}

use anyhow::{Context, Result};
use wasmparser::{ExternalKind, Parser, Payload};

#[derive(Debug, Clone)]
pub struct WasmMemoryPlan {
    pub initial_pages: u64,
    pub max_pages: Option<u64>,
}

impl WasmMemoryPlan {
    pub fn initial_bytes(&self) -> u64 {
        self.initial_pages.saturating_mul(65536)
    }

    pub fn max_bytes(&self) -> Option<u64> {
        self.max_pages.map(|p| p.saturating_mul(65536))
    }

    pub fn growable(&self) -> bool {
        match self.max_pages {
            Some(max) => max > self.initial_pages,
            None => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WasmModuleInfo {
    pub exports: Vec<String>,
    pub memory: Option<WasmMemoryPlan>,
}

pub fn inspect(bytes: &[u8]) -> Result<WasmModuleInfo> {
    let mut exports: Vec<String> = Vec::new();
    let mut memory: Option<WasmMemoryPlan> = None;

    for payload in Parser::new(0).parse_all(bytes) {
        match payload.context("parse wasm")? {
            Payload::ExportSection(s) => {
                for export in s {
                    let export = export.context("parse export")?;
                    let name = export.name.to_string();
                    match export.kind {
                        ExternalKind::Func
                        | ExternalKind::Memory
                        | ExternalKind::Global
                        | ExternalKind::Table
                        | ExternalKind::Tag => {
                            exports.push(name);
                        }
                    }
                }
            }
            Payload::MemorySection(s) => {
                for (idx, mem) in s.into_iter().enumerate() {
                    let mem = mem.context("parse memory")?;
                    if idx == 0 {
                        memory = Some(WasmMemoryPlan {
                            initial_pages: mem.initial,
                            max_pages: mem.maximum,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    exports.sort();
    Ok(WasmModuleInfo { exports, memory })
}

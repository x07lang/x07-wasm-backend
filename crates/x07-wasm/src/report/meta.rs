use std::ffi::OsString;
use std::time::Instant;

use serde::Serialize;

use crate::util;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Nondeterminism {
    pub uses_os_time: bool,
    pub uses_network: bool,
    pub uses_process: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDigest {
    pub path: String,
    pub sha256: String,
    pub bytes_len: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolMeta {
    pub name: &'static str,
    pub version: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rustc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_ld: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasmtime: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportMeta {
    pub tool: ToolMeta,
    pub elapsed_ms: u64,
    pub cwd: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<FileDigest>,
    #[serde(default)]
    pub outputs: Vec<FileDigest>,
    pub nondeterminism: Nondeterminism,
}

pub fn tool_meta(raw_argv: &[OsString], started: Instant) -> ReportMeta {
    ReportMeta {
        tool: ToolMeta {
            name: "x07-wasm",
            version: env!("CARGO_PKG_VERSION"),
            git_sha: None,
            rustc: None,
            clang: None,
            wasm_ld: None,
            wasmtime: util::wasmtime_version(),
        },
        elapsed_ms: started.elapsed().as_millis() as u64,
        cwd: std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .display()
            .to_string(),
        argv: raw_argv
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        nondeterminism: Nondeterminism::default(),
    }
}

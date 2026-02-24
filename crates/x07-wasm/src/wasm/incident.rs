use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn utc_date_yyyy_mm_dd() -> String {
    time::OffsetDateTime::now_utc().date().to_string()
}

pub fn incident_run_id(wasm_sha256: &str, input_sha256: &str) -> String {
    let s = format!("{wasm_sha256}:{input_sha256}");
    let hex = crate::util::sha256_hex(s.as_bytes());
    hex[..32].to_string()
}

pub fn incident_dir(root: &Path, utc_date: &str, run_id: &str) -> PathBuf {
    root.join(".x07-wasm")
        .join("incidents")
        .join(utc_date)
        .join(run_id)
}

pub fn write_incident_bundle(
    dir: &Path,
    input_bytes: &[u8],
    run_report_bytes: &[u8],
    manifest_bytes: &[u8],
    stderr_text: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create dir: {}", dir.display()))?;

    std::fs::write(dir.join("input.bin"), input_bytes)
        .with_context(|| format!("write: {}", dir.join("input.bin").display()))?;
    std::fs::write(dir.join("run.report.json"), run_report_bytes)
        .with_context(|| format!("write: {}", dir.join("run.report.json").display()))?;

    std::fs::write(dir.join("wasm.manifest.json"), manifest_bytes)
        .with_context(|| format!("write: {}", dir.join("wasm.manifest.json").display()))?;

    if let Some(text) = stderr_text {
        if !text.trim().is_empty() {
            let mut bytes = text.as_bytes().to_vec();
            if bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            std::fs::write(dir.join("stderr.txt"), bytes)
                .with_context(|| format!("write: {}", dir.join("stderr.txt").display()))?;
        }
    }

    Ok(())
}

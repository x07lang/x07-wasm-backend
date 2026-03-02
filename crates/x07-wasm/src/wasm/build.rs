use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::arch::CodegenBackend;
use crate::cli::{BuildArgs, MachineArgs, Scope};
use crate::cmdutil;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::report::machine::{self, JsonMode};
use crate::schema::SchemaStore;
use crate::toolchain;
use crate::util;
use crate::wasm::inspect;
use crate::wasm::memory_plan;

const PHASE0_SHIM_C: &str = include_str!("phase0_shim.c");

pub fn cmd_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: BuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let json_mode = machine::json_mode(machine).map_err(anyhow::Error::msg)?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;

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
            let result = build_result_placeholder();
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    };

    meta.inputs.push(loaded_profile.digest.clone());
    if let Some(d) = loaded_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let profile = loaded_profile.doc;
    let profile_ref = json!({ "id": profile.id, "v": profile.v });
    let codegen_backend: CodegenBackend = match args.codegen_backend.as_deref() {
        Some(raw) => match raw.parse::<CodegenBackend>() {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_CLI_ARGS_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    err,
                ));
                let result = build_result_placeholder();
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        },
        None => profile.codegen_backend,
    };

    let project_path = &args.project;

    let project_dir = project_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let project_name = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app");

    let paths = build_paths(&args, &project_dir, project_name, profile_ref.get("id"));

    // Step B: x07 -> freestanding C
    let mut x07_build_args = vec![
        "build".to_string(),
        "--project".to_string(),
        project_path.display().to_string(),
        "--out".to_string(),
        paths.c_path.display().to_string(),
        "--emit-c-header".to_string(),
        paths.h_path.display().to_string(),
        "--freestanding".to_string(),
    ];
    if codegen_backend == CodegenBackend::NativeX07WasmV1 {
        let exp = memory_plan::memory_expectations_from_ldflags(&profile.wasm_ld.ldflags);
        x07_build_args.extend([
            "--emit-wasm".to_string(),
            paths.out_wasm.display().to_string(),
        ]);
        if let Some(v) = exp.initial_memory_bytes {
            x07_build_args.extend(["--wasm-initial-memory-bytes".to_string(), v.to_string()]);
        }
        if let Some(v) = exp.max_memory_bytes {
            x07_build_args.extend(["--wasm-max-memory-bytes".to_string(), v.to_string()]);
        }
        if exp.no_growable_memory {
            x07_build_args.push("--wasm-no-growable-memory".to_string());
        }
    }

    match util::file_digest(project_path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROJECT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read project manifest {}: {err:#}",
                    project_path.display()
                ),
            ));
            meta.inputs.push(report::meta::FileDigest {
                path: project_path.display().to_string(),
                sha256: zero_sha256(),
                bytes_len: 0,
            });
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    }

    match project_entry_path(project_path) {
        Ok(Some(entry_path)) => {
            if !entry_path.exists() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_PROJECT_ENTRY_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("missing entry module: {}", entry_path.display()),
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
            match util::file_digest(&entry_path) {
                Ok(d) => meta.inputs.push(d),
                Err(err) => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_PROJECT_ENTRY_READ_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!(
                            "failed to read entry module {}: {err:#}",
                            entry_path.display()
                        ),
                    ));
                    meta.inputs.push(report::meta::FileDigest {
                        path: entry_path.display().to_string(),
                        sha256: zero_sha256(),
                        bytes_len: 0,
                    });
                    let result = build_result_from_state(
                        &profile_ref,
                        &paths,
                        &profile,
                        &x07_build_args,
                        &profile.clang.cflags,
                        &profile.wasm_ld.ldflags,
                        codegen_backend,
                        None,
                        None,
                    )?;
                    let report_doc = build_report_doc(meta, diagnostics, result);
                    return emit_build_report(&store, scope, machine, json_mode, report_doc);
                }
            }
        }
        Ok(None) => {}
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_PROJECT_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse project manifest: {err:#}"),
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    }

    if let Err(err) = std::fs::create_dir_all(&paths.emit_dir)
        .with_context(|| format!("create dir: {}", paths.emit_dir.display()))
    {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_BUILD_IO_FAILED",
            Stage::Run,
            format!("failed to create emit dir: {}", paths.emit_dir.display()),
            &err,
        ));
        let result = build_result_from_state(
            &profile_ref,
            &paths,
            &profile,
            &x07_build_args,
            &profile.clang.cflags,
            &profile.wasm_ld.ldflags,
            codegen_backend,
            None,
            None,
        )?;
        let report_doc = build_report_doc(meta, diagnostics, result);
        return emit_build_report(&store, scope, machine, json_mode, report_doc);
    }
    if let Some(parent) = paths.out_wasm.parent() {
        if let Err(err) = std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))
        {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_BUILD_IO_FAILED",
                Stage::Run,
                format!("failed to create out dir: {}", parent.display()),
                &err,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    }
    if let Some(parent) = paths.artifact_out.parent() {
        if let Err(err) = std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))
        {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_BUILD_IO_FAILED",
                Stage::Run,
                format!("failed to create artifact dir: {}", parent.display()),
                &err,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    }

    let x07_out = match cmdutil::run_cmd_capture("x07", &x07_build_args) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_X07_BUILD_SPAWN_FAILED",
                Stage::Codegen,
                "x07 build",
                &err,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &[],
                &[],
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    };
    if !x07_out.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_X07_BUILD_FAILED",
            Stage::Codegen,
            "x07 build",
            x07_out.code,
            &x07_out.stderr,
        ));
        let result = build_result_from_state(
            &profile_ref,
            &paths,
            &profile,
            &x07_build_args,
            &[],
            &[],
            codegen_backend,
            None,
            None,
        )?;
        let report_doc = build_report_doc(meta, diagnostics, result);
        return emit_build_report(&store, scope, machine, json_mode, report_doc);
    }

    let c_digest = match util::file_digest(&paths.c_path) {
        Ok(d) => d,
        Err(err) => {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_BUILD_OUTPUT_IO_FAILED",
                Stage::Run,
                format!("failed to digest output: {}", paths.c_path.display()),
                &err,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    };
    meta.outputs.push(c_digest);

    let h_digest = match util::file_digest(&paths.h_path) {
        Ok(d) => d,
        Err(err) => {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_BUILD_OUTPUT_IO_FAILED",
                Stage::Run,
                format!("failed to digest output: {}", paths.h_path.display()),
                &err,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    };
    meta.outputs.push(h_digest);

    let cc = profile
        .clang
        .cc
        .clone()
        .unwrap_or_else(|| "clang".to_string());
    let linker = profile
        .wasm_ld
        .linker
        .clone()
        .unwrap_or_else(|| "wasm-ld".to_string());

    let wasm_bytes = if codegen_backend == CodegenBackend::CToolchainV1 {
        // Step C0: write Phase 0 shims (no WASI imports)
        let shim_c_path = paths.emit_dir.join("phase0_shim.c");
        let shim_o_path = paths.emit_dir.join("phase0_shim.o");
        match std::fs::write(&shim_c_path, PHASE0_SHIM_C)
            .with_context(|| format!("write: {}", shim_c_path.display()))
        {
            Ok(()) => match util::file_digest(&shim_c_path) {
                Ok(d) => meta.outputs.push(d),
                Err(err) => diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_SHIM_DIGEST_IO_FAILED",
                    Stage::Run,
                    format!("failed to digest output: {}", shim_c_path.display()),
                    &err,
                )),
            },
            Err(err) => {
                diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_SHIM_WRITE_FAILED",
                    Stage::Run,
                    format!("failed to write shims: {}", shim_c_path.display()),
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        }

        // Step C: clang compile
        let mut clang_full_args = profile.clang.cflags.clone();
        if !clang_full_args.iter().any(|f| f.starts_with("--target=")) {
            clang_full_args.insert(0, format!("--target={}", profile.target.triple));
        }
        clang_full_args.extend([
            "-c".to_string(),
            paths.c_path.display().to_string(),
            "-o".to_string(),
            paths.o_path.display().to_string(),
        ]);
        let clang_out = match cmdutil::run_cmd_capture(&cc, &clang_full_args) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                    "X07WASM_CLANG_SPAWN_FAILED",
                    Stage::Codegen,
                    "clang",
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        };
        if !clang_out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_CLANG_FAILED",
                Stage::Codegen,
                "clang",
                clang_out.code,
                &clang_out.stderr,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }

        let mut clang_shim_args = profile.clang.cflags.clone();
        if !clang_shim_args.iter().any(|f| f.starts_with("--target=")) {
            clang_shim_args.insert(0, format!("--target={}", profile.target.triple));
        }
        clang_shim_args.extend([
            "-c".to_string(),
            shim_c_path.display().to_string(),
            "-o".to_string(),
            shim_o_path.display().to_string(),
        ]);
        let clang_shim_out = match cmdutil::run_cmd_capture(&cc, &clang_shim_args) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                    "X07WASM_CLANG_SHIM_SPAWN_FAILED",
                    Stage::Codegen,
                    "clang",
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        };
        if !clang_shim_out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_CLANG_SHIM_FAILED",
                Stage::Codegen,
                "clang",
                clang_shim_out.code,
                &clang_shim_out.stderr,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }

        // Step D: wasm-ld link
        let mut wasm_ld_full_args = profile.wasm_ld.ldflags.clone();
        wasm_ld_full_args.push(paths.o_path.display().to_string());
        wasm_ld_full_args.push(shim_o_path.display().to_string());
        wasm_ld_full_args.extend(["-o".to_string(), paths.out_wasm.display().to_string()]);
        let ld_out = match cmdutil::run_cmd_capture(&linker, &wasm_ld_full_args) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                    "X07WASM_WASM_LD_SPAWN_FAILED",
                    Stage::Link,
                    "wasm-ld",
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        };
        if !ld_out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_WASM_LD_FAILED",
                Stage::Link,
                "wasm-ld",
                ld_out.code,
                &ld_out.stderr,
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }

        match std::fs::read(&paths.out_wasm)
            .with_context(|| format!("read: {}", paths.out_wasm.display()))
        {
            Ok(b) => b,
            Err(err) => {
                diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_BUILD_OUTPUT_IO_FAILED",
                    Stage::Link,
                    format!("failed to read output: {}", paths.out_wasm.display()),
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &profile.clang.cflags,
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        }
    } else {
        if !paths.out_wasm.is_file() {
            diagnostics.push(Diagnostic::new(
                "X07WASM_NATIVE_BACKEND_WASM_MISSING",
                Severity::Error,
                Stage::Codegen,
                format!(
                    "native backend did not produce wasm output: {}",
                    paths.out_wasm.display()
                ),
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &[],
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }

        match std::fs::read(&paths.out_wasm)
            .with_context(|| format!("read: {}", paths.out_wasm.display()))
        {
            Ok(b) => b,
            Err(err) => {
                diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_BUILD_OUTPUT_IO_FAILED",
                    Stage::Codegen,
                    format!("failed to read output: {}", paths.out_wasm.display()),
                    &err,
                ));
                let result = build_result_from_state(
                    &profile_ref,
                    &paths,
                    &profile,
                    &x07_build_args,
                    &[],
                    &profile.wasm_ld.ldflags,
                    codegen_backend,
                    None,
                    None,
                )?;
                let report_doc = build_report_doc(meta, diagnostics, result);
                return emit_build_report(&store, scope, machine, json_mode, report_doc);
            }
        }
    };

    let wasm_digest = report::meta::FileDigest {
        path: paths.out_wasm.display().to_string(),
        sha256: util::sha256_hex(&wasm_bytes),
        bytes_len: wasm_bytes.len() as u64,
    };
    meta.outputs.push(wasm_digest.clone());

    // Step E: inspect wasm and enforce invariants
    let info = match inspect::inspect(&wasm_bytes) {
        Ok(v) => v,
        Err(err) => {
            let (code, stage) = if codegen_backend == CodegenBackend::NativeX07WasmV1 {
                ("X07WASM_NATIVE_BACKEND_WASM_INVALID", Stage::Codegen)
            } else {
                ("X07WASM_WASM_INSPECT_FAILED", Stage::Link)
            };
            diagnostics.push(Diagnostic::new(
                code,
                Severity::Error,
                stage,
                format!("failed to inspect wasm: {err:#}"),
            ));
            let result = build_result_from_state(
                &profile_ref,
                &paths,
                &profile,
                &x07_build_args,
                &profile.clang.cflags,
                &profile.wasm_ld.ldflags,
                codegen_backend,
                None,
                None,
            )?;
            let report_doc = build_report_doc(meta, diagnostics, result);
            return emit_build_report(&store, scope, machine, json_mode, report_doc);
        }
    };

    if args.check_exports {
        let required_exports = required_exports();
        let missing_exports: Vec<String> = required_exports
            .iter()
            .filter(|name| !info.exports.contains(*name))
            .cloned()
            .collect();
        if !missing_exports.is_empty() {
            diagnostics.push(Diagnostic::new(
                "X07WASM_EXPORTS_MISSING",
                Severity::Error,
                Stage::Link,
                format!("missing required exports: {:?}", missing_exports),
            ));
        }
    }

    let mem = match info.memory.clone() {
        Some(m) => m,
        None => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_NO_MEMORY",
                Severity::Error,
                Stage::Link,
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
                Stage::Link,
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
                Stage::Link,
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
            Stage::Link,
            "growable memory is not allowed for Phase 0 deterministic runs".to_string(),
        ));
    }
    let memory_plan: Value = serde_json::to_value(&mem_plan).context("encode memory plan")?;

    // Step F: build artifact manifest
    let x07_semver = toolchain::x07_semver().unwrap_or_else(|_| "0.0.0".to_string());
    let (clang_ver, wasm_ld_ver) = if codegen_backend == CodegenBackend::CToolchainV1 {
        let clang_ver = toolchain::tool_first_line(&cc, &["--version"]).ok();
        let wasm_ld_ver = toolchain::tool_first_line(&linker, &["--version"]).ok();

        meta.tool.clang = clang_ver.clone();
        meta.tool.wasm_ld = wasm_ld_ver.clone();
        (clang_ver, wasm_ld_ver)
    } else {
        (None, None)
    };

    let artifact_doc = build_artifact_doc(
        project_name,
        &profile_ref,
        &wasm_digest,
        &memory_plan,
        codegen_backend,
        &x07_semver,
        env!("CARGO_PKG_VERSION"),
        clang_ver.as_deref(),
        wasm_ld_ver.as_deref(),
        &x07_build_args,
        if codegen_backend == CodegenBackend::CToolchainV1 {
            &profile.clang.cflags
        } else {
            &[]
        },
        &profile.wasm_ld.ldflags,
    );

    ensure_schema_ok(
        &store,
        "https://x07.io/spec/x07-wasm.artifact.schema.json",
        &artifact_doc,
    )
    .context("validate artifact manifest")?;

    if !args.no_manifest {
        let bytes = report::canon::canonical_json_bytes(&artifact_doc)?;
        match std::fs::write(&paths.artifact_out, &bytes)
            .with_context(|| format!("write: {}", paths.artifact_out.display()))
        {
            Ok(()) => match util::file_digest(&paths.artifact_out) {
                Ok(d) => meta.outputs.push(d),
                Err(err) => diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_MANIFEST_DIGEST_IO_FAILED",
                    Stage::Run,
                    format!("failed to digest output: {}", paths.artifact_out.display()),
                    &err,
                )),
            },
            Err(err) => diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_MANIFEST_WRITE_FAILED",
                Stage::Run,
                format!("failed to write manifest: {}", paths.artifact_out.display()),
                &err,
            )),
        }
    }

    let result = build_result_from_state(
        &profile_ref,
        &paths,
        &profile,
        &x07_build_args,
        if codegen_backend == CodegenBackend::CToolchainV1 {
            &profile.clang.cflags
        } else {
            &[]
        },
        &profile.wasm_ld.ldflags,
        codegen_backend,
        Some(info.exports),
        Some(memory_plan),
    )?;

    // Replace placeholder artifact with real artifact.
    let mut result_obj = result
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("internal error: build result must be object"))?;
    result_obj.insert("artifact".to_string(), artifact_doc);
    let report_doc = build_report_doc(meta, diagnostics, Value::Object(result_obj));

    emit_build_report(&store, scope, machine, json_mode, report_doc)
}

#[derive(Clone)]
struct BuildPaths {
    emit_dir: PathBuf,
    c_path: PathBuf,
    h_path: PathBuf,
    o_path: PathBuf,
    out_wasm: PathBuf,
    artifact_out: PathBuf,
}

fn build_paths(
    args: &BuildArgs,
    project_dir: &Path,
    project_name: &str,
    profile_id: Option<&Value>,
) -> BuildPaths {
    let profile_id = profile_id.and_then(Value::as_str).unwrap_or("wasm_release");
    let emit_dir = args
        .emit_dir
        .clone()
        .unwrap_or_else(|| project_dir.join("build/wasm").join(profile_id));
    let out_wasm = args.out.clone().unwrap_or_else(|| {
        project_dir
            .join("dist")
            .join(format!("{project_name}.wasm"))
    });
    let artifact_out = args
        .artifact_out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.manifest.json", out_wasm.to_string_lossy())));
    BuildPaths {
        c_path: emit_dir.join("program.c"),
        h_path: emit_dir.join("x07.h"),
        o_path: emit_dir.join("program.o"),
        emit_dir,
        out_wasm,
        artifact_out,
    }
}

fn project_entry_path(project_path: &Path) -> Result<Option<PathBuf>> {
    let bytes =
        std::fs::read(project_path).with_context(|| format!("read: {}", project_path.display()))?;
    let doc: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", project_path.display()))?;
    let Some(entry) = doc.get("entry").and_then(Value::as_str) else {
        return Ok(None);
    };
    let dir = project_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(Some(dir.join(entry)))
}

fn required_exports() -> Vec<String> {
    vec![
        "x07_solve_v2".to_string(),
        "memory".to_string(),
        "__heap_base".to_string(),
        "__data_end".to_string(),
    ]
}

fn zero_sha256() -> String {
    "0".repeat(64)
}

fn file_digest_or_zero(path: &Path) -> report::meta::FileDigest {
    match util::file_digest(path) {
        Ok(d) => d,
        Err(_) => report::meta::FileDigest {
            path: path.display().to_string(),
            sha256: zero_sha256(),
            bytes_len: 0,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn build_artifact_doc(
    project_name: &str,
    profile_ref: &Value,
    wasm: &report::meta::FileDigest,
    memory_plan: &Value,
    codegen_backend: CodegenBackend,
    x07_semver: &str,
    x07_wasm_semver: &str,
    clang_version: Option<&str>,
    wasm_ld_version: Option<&str>,
    x07_build_args: &[String],
    clang_args: &[String],
    wasm_ld_args: &[String],
) -> Value {
    let artifact_id = format!("{project_name}-{}", &wasm.sha256[..16]);

    let mut toolchain_obj = serde_json::Map::new();
    toolchain_obj.insert("x07".to_string(), json!(x07_semver));
    toolchain_obj.insert("x07_wasm".to_string(), json!(x07_wasm_semver));
    if let Some(v) = clang_version {
        toolchain_obj.insert("clang".to_string(), json!(v));
    }
    if let Some(v) = wasm_ld_version {
        toolchain_obj.insert("wasm_ld".to_string(), json!(v));
    }
    if let Some(v) = crate::util::wasmtime_version() {
        toolchain_obj.insert("wasmtime".to_string(), json!(v));
    }

    let mut repro_obj = serde_json::Map::new();
    repro_obj.insert("x07_build_args".to_string(), json!(x07_build_args));
    repro_obj.insert("clang_args".to_string(), json!(clang_args));
    repro_obj.insert("wasm_ld_args".to_string(), json!(wasm_ld_args));

    json!({
      "schema_version": "x07.wasm.artifact@0.2.0",
      "artifact_id": artifact_id,
      "codegen_backend": codegen_backend,
      "profile": profile_ref,
      "wasm": wasm,
      "abi": {
        "kind": "x07_solve_v2",
        "c_signature": "bytes_t x07_solve_v2(uint8_t* arena_mem, uint32_t arena_cap, const uint8_t* input_ptr, uint32_t input_len)",
        "wasm_c_abi": { "kind": "basic_c_abi", "v": 1 },
        "calling_convention": {
          "kind": "sret_v1",
          "retptr_param_index": 0,
          "return_struct": {
            "kind": "bytes_t",
            "size_bytes": 8,
            "align_bytes": 4,
            "fields": [
              { "name": "ptr", "type": "i32", "offset_bytes": 0 },
              { "name": "len", "type": "i32", "offset_bytes": 4 }
            ]
          }
        },
        "params": [
          { "index": 0, "name": "retptr", "type": "i32", "role": "sret_ptr" },
          { "index": 1, "name": "arena_mem", "type": "i32", "role": "arena_ptr" },
          { "index": 2, "name": "arena_cap", "type": "i32", "role": "arena_cap" },
          { "index": 3, "name": "input_ptr", "type": "i32", "role": "input_ptr" },
          { "index": 4, "name": "input_len", "type": "i32", "role": "input_len" }
        ]
      },
      "exports": {
        "solve": "x07_solve_v2",
        "memory": "memory",
        "heap_base": "__heap_base",
        "data_end": "__data_end",
        "optional": {}
      },
      "memory": memory_plan,
      "toolchain": toolchain_obj,
      "repro": repro_obj
    })
}

fn build_result_placeholder() -> Value {
    let wasm = report::meta::FileDigest {
        path: "dist/unknown.wasm".to_string(),
        sha256: zero_sha256(),
        bytes_len: 0,
    };
    json!({
      "profile": { "id": "wasm_release", "v": 1 },
      "emit": { "emit_dir": "", "c_path": "", "h_path": null },
      "wasm": wasm,
      "artifact": build_artifact_doc(
          "unknown",
          &json!({"id":"wasm_release","v":1}),
          &wasm,
          &json!({"stack_first": false, "stack_size_bytes": 0, "initial_memory_bytes": 0, "max_memory_bytes": 0, "growable_memory": false}),
          CodegenBackend::NativeX07WasmV1,
          "0.0.0",
          env!("CARGO_PKG_VERSION"),
          None,
          None,
          &[],
          &[],
          &[],
      ),
      "exports": { "required": required_exports(), "found": [], "missing": required_exports() },
      "memory": { "stack_first": false, "stack_size_bytes": 0, "initial_memory_bytes": 0, "max_memory_bytes": 0, "growable_memory": false },
      "flags": { "codegen_backend": CodegenBackend::NativeX07WasmV1, "clang": [], "wasm_ld": [] }
    })
}

#[allow(clippy::too_many_arguments)]
fn build_result_from_state(
    profile_ref: &Value,
    paths: &BuildPaths,
    _profile: &crate::arch::WasmProfileDoc,
    x07_build_args: &[String],
    clang_args: &[String],
    wasm_ld_args: &[String],
    codegen_backend: CodegenBackend,
    exports_found: Option<Vec<String>>,
    memory_plan: Option<Value>,
) -> Result<Value> {
    let wasm = file_digest_or_zero(&paths.out_wasm);
    let found = exports_found.unwrap_or_default();
    let required = required_exports();
    let missing: Vec<String> = required
        .iter()
        .filter(|name| !found.contains(*name))
        .cloned()
        .collect();

    let memory = memory_plan.unwrap_or_else(|| {
        json!({
          "stack_first": false,
          "stack_size_bytes": 0,
          "initial_memory_bytes": 0,
          "max_memory_bytes": 0,
          "growable_memory": false
        })
    });

    let artifact_placeholder = build_artifact_doc(
        "unknown",
        profile_ref,
        &wasm,
        &memory,
        codegen_backend,
        &toolchain::x07_semver().unwrap_or_else(|_| "0.0.0".to_string()),
        env!("CARGO_PKG_VERSION"),
        None,
        None,
        x07_build_args,
        clang_args,
        wasm_ld_args,
    );

    Ok(json!({
      "profile": profile_ref,
      "emit": {
        "emit_dir": paths.emit_dir.display().to_string(),
        "c_path": paths.c_path.display().to_string(),
        "h_path": paths.h_path.display().to_string(),
      },
      "wasm": wasm,
      "artifact": artifact_placeholder,
      "exports": { "required": required, "found": found, "missing": missing },
      "memory": memory,
      "flags": { "codegen_backend": codegen_backend, "clang": clang_args, "wasm_ld": wasm_ld_args }
    }))
}

fn ensure_schema_ok(store: &SchemaStore, schema_id: &str, doc: &Value) -> Result<()> {
    let diags = store.validate(schema_id, doc)?;
    if diags.is_empty() {
        return Ok(());
    }
    anyhow::bail!("schema validation failed for {schema_id:?}: {diags:?}");
}

fn build_report_doc(
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    result: Value,
) -> Value {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    json!({
      "schema_version": "x07.wasm.build.report@0.2.0",
      "command": "x07-wasm.build",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": result,
    })
}

fn emit_build_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    json_mode: JsonMode,
    report_doc: Value,
) -> Result<u8> {
    ensure_schema_ok(
        store,
        "https://x07.io/spec/x07-wasm.build.report.schema.json",
        &report_doc,
    )?;

    let exit_code = report_doc
        .get("exit_code")
        .and_then(Value::as_u64)
        .unwrap_or(2) as u8;
    if json_mode != JsonMode::Off {
        store.validate_report_and_emit(
            scope,
            machine,
            std::time::Instant::now(),
            &[],
            report_doc,
        )?;
    } else if exit_code == 0 {
        eprintln!("ok");
    }
    Ok(exit_code)
}

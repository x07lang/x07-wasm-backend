use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::arch;
use crate::cli::{ComponentBuildArgs, ComponentBuildEmit, MachineArgs, Scope};
use crate::cmdutil;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::toolchain;
use crate::util;

const SOLVE_BINDGEN_C_TEMPLATE: &str =
    include_str!("../support/component/solve_bindgen_c_template.c");
const PHASE0_SHIM_C: &str = include_str!("../wasm/phase0_shim.c");

const DEFAULT_COMPONENT_PROFILE_ID: &str = "component_release";

const HTTP_ADAPTER_MANIFEST: &str = "guest/http-adapter/Cargo.toml";
const HTTP_ADAPTER_LOCK: &str = "guest/http-adapter/Cargo.lock";
const HTTP_ADAPTER_COMPONENT_WASM: &str =
    "guest/http-adapter/target/wasm32-wasip2/release/x07_wasm_http_adapter.wasm";

const CLI_ADAPTER_MANIFEST: &str = "guest/cli-adapter/Cargo.toml";
const CLI_ADAPTER_LOCK: &str = "guest/cli-adapter/Cargo.lock";
const CLI_ADAPTER_COMPONENT_WASM: &str =
    "guest/cli-adapter/target/wasm32-wasip2/release/x07_wasm_cli_adapter.wasm";

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexDoc {
    profiles: Vec<ComponentIndexProfileRef>,
    #[serde(default)]
    defaults: Option<ComponentIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexDefaults {
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentIndexProfileRef {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCmd {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentProfileToolchain {
    wit_bindgen: ToolCmd,
    wasm_tools: ToolCmd,
    wac: ToolCmd,
    wasmtime: ToolCmd,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentizeCfg {
    mode: String,
    solve_package: String,
    solve_world: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposeCfg {
    mode: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TargetsCfg {
    http_package: String,
    http_world: String,
    cli_package: String,
    cli_world: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentProfileCfg {
    wit_index_path: String,
    toolchain: ComponentProfileToolchain,
    componentize: ComponentizeCfg,
    compose: ComposeCfg,
    targets: TargetsCfg,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentProfileDoc {
    id: String,
    v: u64,
    cfg: ComponentProfileCfg,
}

#[derive(Debug, Clone)]
struct LoadedComponentProfile {
    digest: report::meta::FileDigest,
    doc: ComponentProfileDoc,
    index_digest: Option<report::meta::FileDigest>,
}

pub fn cmd_component_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: ComponentBuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let project_digest = match util::file_digest(&args.project) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_BUILD_PROJECT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read project {}: {err:#}", args.project.display()),
            ));
            report::meta::FileDigest {
                path: args.project.display().to_string(),
                sha256: "0".repeat(64),
                bytes_len: 0,
            }
        }
    };

    let loaded_component_profile = match load_component_profile(
        &store,
        &args.index,
        args.profile.as_deref(),
        args.profile_file.as_ref(),
    ) {
        Ok(p) => p,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            let report_doc = component_build_report_doc(
                &project_digest,
                None,
                None,
                &args.out_dir,
                args.emit,
                None,
                Vec::new(),
                meta,
                diagnostics,
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };

    meta.inputs.push(loaded_component_profile.digest.clone());
    if let Some(d) = loaded_component_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let component_profile_ref = json!({
      "id": loaded_component_profile.doc.id.clone(),
      "v": loaded_component_profile.doc.v,
    });

    let loaded_wasm_profile = match arch::load_profile(
        &store,
        &args.wasm_index,
        args.wasm_profile.as_deref(),
        args.wasm_profile_file.as_ref(),
    ) {
        Ok(p) => p,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            let report_doc = component_build_report_doc(
                &project_digest,
                Some(&component_profile_ref),
                None,
                &args.out_dir,
                args.emit,
                None,
                Vec::new(),
                meta,
                diagnostics,
            );
            let exit_code = report_doc
                .get("exit_code")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u8;
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };

    meta.inputs.push(loaded_wasm_profile.digest.clone());
    if let Some(d) = loaded_wasm_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let wasm_profile_ref = json!({
      "id": loaded_wasm_profile.doc.id.clone(),
      "v": loaded_wasm_profile.doc.v,
    });

    if args.clean {
        if let Err(err) = std::fs::remove_dir_all(&args.out_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                diagnostics.push(cmdutil::diag_io_failed(
                    "X07WASM_COMPONENT_BUILD_CLEAN_FAILED",
                    Stage::Run,
                    format!("failed to clean out dir: {}", args.out_dir.display()),
                    &anyhow::Error::new(err),
                ));
            }
        }
    }

    if let Err(err) = std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create dir: {}", args.out_dir.display()))
    {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_COMPONENT_BUILD_OUTDIR_CREATE_FAILED",
            Stage::Run,
            format!("failed to create out dir: {}", args.out_dir.display()),
            &err,
        ));
    }

    let mut solve_core_wasm_digest: Option<report::meta::FileDigest> = None;
    let mut artifacts: Vec<Value> = Vec::new();

    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        match args.emit {
            ComponentBuildEmit::Solve => {
                let built = match build_solve_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        SolveBuildOutput {
                            solve_core_wasm: None,
                            solve_artifact: None,
                        }
                    }
                };
                solve_core_wasm_digest = built.solve_core_wasm;
                if let Some(a) = built.solve_artifact {
                    artifacts.push(a);
                }
            }
            ComponentBuildEmit::HttpAdapter => {
                let built = match build_http_adapter_component(
                    &store,
                    &loaded_component_profile.doc,
                    &args.out_dir,
                    &component_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        None
                    }
                };
                if let Some(a) = built {
                    artifacts.push(a);
                }
            }
            ComponentBuildEmit::CliAdapter => {
                let built = match build_cli_adapter_component(
                    &store,
                    &loaded_component_profile.doc,
                    &args.out_dir,
                    &component_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        None
                    }
                };
                if let Some(a) = built {
                    artifacts.push(a);
                }
            }
            ComponentBuildEmit::All => {
                let built = match build_solve_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        SolveBuildOutput {
                            solve_core_wasm: None,
                            solve_artifact: None,
                        }
                    }
                };
                solve_core_wasm_digest = built.solve_core_wasm;
                if let Some(a) = built.solve_artifact {
                    artifacts.push(a);
                }

                let built_http = match build_http_adapter_component(
                    &store,
                    &loaded_component_profile.doc,
                    &args.out_dir,
                    &component_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        None
                    }
                };
                if let Some(a) = built_http {
                    artifacts.push(a);
                }
                let built_cli = match build_cli_adapter_component(
                    &store,
                    &loaded_component_profile.doc,
                    &args.out_dir,
                    &component_profile_ref,
                    &mut meta,
                    &mut diagnostics,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                        None
                    }
                };
                if let Some(a) = built_cli {
                    artifacts.push(a);
                }
            }
        }
    }

    let report_doc = component_build_report_doc(
        &project_digest,
        Some(&component_profile_ref),
        Some(&wasm_profile_ref),
        &args.out_dir,
        args.emit,
        solve_core_wasm_digest.as_ref(),
        artifacts,
        meta,
        diagnostics,
    );

    let exit_code = report_doc
        .get("exit_code")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u8;
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

struct SolveBuildOutput {
    solve_core_wasm: Option<report::meta::FileDigest>,
    solve_artifact: Option<Value>,
}

#[allow(clippy::too_many_arguments)]
fn build_solve_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    project_path: &Path,
    out_dir: &Path,
    component_profile_ref: &Value,
    wasm_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<SolveBuildOutput> {
    let cfg = &component_profile.cfg;

    if cfg.componentize.mode.trim() != "wasm-tools-component-new_v1" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_COMPONENTIZE_MODE_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!("unsupported componentize mode: {:?}", cfg.componentize.mode),
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }

    let program_c = out_dir.join("program.c");
    let x07_h = out_dir.join("x07.h");
    let shim_c = out_dir.join("phase0_shim.c");
    let shim_o = out_dir.join("phase0_shim.o");

    let wit_out_dir = out_dir.join("wit-bindgen");
    let wit_solve_c = wit_out_dir.join("solve.c");
    let wit_solve_o = out_dir.join("solve_bindgen.o");
    let wit_component_type_o = wit_out_dir.join("solve_component_type.o");
    let solve_glue_c = out_dir.join("solve_glue.c");
    let solve_glue_o = out_dir.join("solve_glue.o");

    let solve_core_wasm = out_dir.join("solve.core.wasm");
    let solve_component_wasm = out_dir.join("solve.component.wasm");
    let solve_component_manifest = out_dir.join("solve.component.wasm.manifest.json");

    if let Err(err) = std::fs::create_dir_all(&wit_out_dir)
        .with_context(|| format!("create dir: {}", wit_out_dir.display()))
    {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_COMPONENT_BUILD_IO_FAILED",
            Stage::Run,
            format!(
                "failed to create wit-bindgen dir: {}",
                wit_out_dir.display()
            ),
            &err,
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }

    // Step A: x07 -> freestanding C
    let x07_build_args = vec![
        "build".to_string(),
        "--project".to_string(),
        project_path.display().to_string(),
        "--out".to_string(),
        program_c.display().to_string(),
        "--emit-c-header".to_string(),
        x07_h.display().to_string(),
        "--freestanding".to_string(),
    ];
    let x07_out = match cmdutil::run_cmd_capture("x07", &x07_build_args) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_X07_BUILD_SPAWN_FAILED",
                Stage::Codegen,
                "x07 build",
                &err,
            ));
            return Ok(SolveBuildOutput {
                solve_core_wasm: None,
                solve_artifact: None,
            });
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
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }
    meta.outputs.push(util::file_digest(&program_c)?);
    meta.outputs.push(util::file_digest(&x07_h)?);

    // Step B: wit-bindgen c for x07:solve
    let solve_wit_dir = resolve_wit_package_dir(
        store,
        Path::new(&cfg.wit_index_path),
        &cfg.componentize.solve_package,
        meta,
        diagnostics,
    )?;
    let mut wit_bindgen_args = cfg.toolchain.wit_bindgen.args.clone();
    wit_bindgen_args.extend([
        "c".to_string(),
        solve_wit_dir.display().to_string(),
        "--world".to_string(),
        cfg.componentize.solve_world.clone(),
        "--out-dir".to_string(),
        wit_out_dir.display().to_string(),
    ]);
    let wit_out = match run_tool_cmd_capture(
        &cfg.toolchain.wit_bindgen.cmd,
        &wit_bindgen_args,
        &cfg.toolchain.wit_bindgen.env,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WIT_BINDGEN_SPAWN_FAILED",
                Stage::Codegen,
                "wit-bindgen c",
                &err,
            ));
            return Ok(SolveBuildOutput {
                solve_core_wasm: None,
                solve_artifact: None,
            });
        }
    };
    if !wit_out.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_WIT_BINDGEN_FAILED",
            Stage::Codegen,
            "wit-bindgen c",
            wit_out.code,
            &wit_out.stderr,
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }

    meta.outputs.push(util::file_digest(&wit_solve_c)?);
    meta.outputs
        .push(util::file_digest(&wit_out_dir.join("solve.h"))?);
    meta.outputs.push(util::file_digest(&wit_component_type_o)?);

    // Step C: write shims + glue
    std::fs::write(&shim_c, PHASE0_SHIM_C)
        .with_context(|| format!("write: {}", shim_c.display()))?;
    meta.outputs.push(util::file_digest(&shim_c)?);

    let glue = format!(
        "#define X07_SOLVE_ARENA_CAP_BYTES ({arena}u)\n#define X07_SOLVE_MAX_OUTPUT_BYTES ({max_out}u)\n{template}",
        arena = wasm_profile.defaults.arena_cap_bytes,
        max_out = wasm_profile.defaults.max_output_bytes,
        template = SOLVE_BINDGEN_C_TEMPLATE
    );
    std::fs::write(&solve_glue_c, glue)
        .with_context(|| format!("write: {}", solve_glue_c.display()))?;
    meta.outputs.push(util::file_digest(&solve_glue_c)?);

    // Step D: clang compile
    let cc = wasm_profile
        .clang
        .cc
        .clone()
        .unwrap_or_else(|| "clang".to_string());

    let mut cflags = wasm_profile.clang.cflags.clone();
    if !cflags.iter().any(|f| f.starts_with("--target=")) {
        cflags.insert(0, format!("--target={}", wasm_profile.target.triple));
    }

    let program_o = out_dir.join("program.o");
    compile_one_c(&cc, &cflags, &program_c, &program_o, &[], diagnostics)?;
    compile_one_c(&cc, &cflags, &shim_c, &shim_o, &[], diagnostics)?;
    compile_one_c(
        &cc,
        &cflags,
        &wit_solve_c,
        &wit_solve_o,
        &[format!("-I{}", wit_out_dir.display())],
        diagnostics,
    )?;
    compile_one_c(
        &cc,
        &cflags,
        &solve_glue_c,
        &solve_glue_o,
        &[
            format!("-I{}", wit_out_dir.display()),
            format!("-I{}", out_dir.display()),
        ],
        diagnostics,
    )?;

    // Step E: wasm-ld link (core module)
    let linker = wasm_profile
        .wasm_ld
        .linker
        .clone()
        .unwrap_or_else(|| "wasm-ld".to_string());
    let mut ldflags = wasm_profile.wasm_ld.ldflags.clone();
    ldflags.push(program_o.display().to_string());
    ldflags.push(shim_o.display().to_string());
    ldflags.push(wit_solve_o.display().to_string());
    ldflags.push(solve_glue_o.display().to_string());
    ldflags.push(wit_component_type_o.display().to_string());
    ldflags.extend(["-o".to_string(), solve_core_wasm.display().to_string()]);
    let ld_out = match cmdutil::run_cmd_capture(&linker, &ldflags) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WASM_LD_SPAWN_FAILED",
                Stage::Link,
                "wasm-ld",
                &err,
            ));
            return Ok(SolveBuildOutput {
                solve_core_wasm: None,
                solve_artifact: None,
            });
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
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }

    let solve_core_digest = util::file_digest(&solve_core_wasm)?;
    meta.outputs.push(solve_core_digest.clone());

    // Step F: wasm-tools component new (component)
    let mut wasm_tools_args = cfg.toolchain.wasm_tools.args.clone();
    wasm_tools_args.extend([
        "component".to_string(),
        "new".to_string(),
        solve_core_wasm.display().to_string(),
        "-o".to_string(),
        solve_component_wasm.display().to_string(),
    ]);
    let comp_out = match run_tool_cmd_capture(
        &cfg.toolchain.wasm_tools.cmd,
        &wasm_tools_args,
        &cfg.toolchain.wasm_tools.env,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WASM_TOOLS_SPAWN_FAILED",
                Stage::Link,
                "wasm-tools component new",
                &err,
            ));
            return Ok(SolveBuildOutput {
                solve_core_wasm: Some(solve_core_digest),
                solve_artifact: None,
            });
        }
    };
    if !comp_out.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_WASM_TOOLS_FAILED",
            Stage::Link,
            "wasm-tools component new",
            comp_out.code,
            &comp_out.stderr,
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: Some(solve_core_digest),
            solve_artifact: None,
        });
    }

    let solve_component_digest = util::file_digest(&solve_component_wasm)?;
    meta.outputs.push(solve_component_digest.clone());

    // Step G: write component artifact manifest
    let x07_semver = toolchain::x07_semver().unwrap_or_else(|_| "0.0.0".to_string());
    let clang_ver = toolchain::tool_first_line(&cc, &["--version"]).ok();
    let wasm_ld_ver = toolchain::tool_first_line(&linker, &["--version"]).ok();
    let wit_bindgen_ver =
        toolchain::tool_first_line(&cfg.toolchain.wit_bindgen.cmd, &["--version"]).ok();
    let wasm_tools_ver =
        toolchain::tool_first_line(&cfg.toolchain.wasm_tools.cmd, &["--version"]).ok();

    meta.tool.clang = clang_ver.clone();
    meta.tool.wasm_ld = wasm_ld_ver.clone();

    let mut toolchain_obj = serde_json::Map::new();
    toolchain_obj.insert("x07_wasm".to_string(), json!(env!("CARGO_PKG_VERSION")));
    toolchain_obj.insert("x07".to_string(), json!(x07_semver));
    if let Some(v) = clang_ver.clone() {
        toolchain_obj.insert("clang".to_string(), json!(v));
    }
    if let Some(v) = wasm_ld_ver.clone() {
        toolchain_obj.insert("wasm_ld".to_string(), json!(v));
    }
    if let Some(v) = wit_bindgen_ver.clone() {
        toolchain_obj.insert("wit_bindgen".to_string(), json!(v));
    }
    if let Some(v) = wasm_tools_ver.clone() {
        toolchain_obj.insert("wasm_tools".to_string(), json!(v));
    }

    let artifact_doc = json!({
      "schema_version": "x07.wasm.component.artifact@0.1.0",
      "artifact_id": format!("solve-{}", &solve_component_digest.sha256[..16]),
      "kind": "solve",
      "component": solve_component_digest,
      "wit": { "package": cfg.componentize.solve_package.clone(), "world": cfg.componentize.solve_world.clone() },
      "profiles": {
        "component": component_profile_ref,
        "wasm": wasm_profile_ref,
      },
      "toolchain": Value::Object(toolchain_obj)
    });

    let artifact_diags = store.validate(
        "https://x07.io/spec/x07-wasm.component.artifact.schema.json",
        &artifact_doc,
    )?;
    if artifact_diags.iter().any(|d| d.severity == Severity::Error) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_COMPONENT_ARTIFACT_SCHEMA_INVALID",
            Severity::Error,
            Stage::Run,
            format!(
                "internal error: component artifact failed schema validation: {artifact_diags:?}"
            ),
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: Some(solve_core_digest),
            solve_artifact: None,
        });
    }

    let bytes = report::canon::canonical_json_bytes(&artifact_doc)?;
    std::fs::write(&solve_component_manifest, &bytes)
        .with_context(|| format!("write: {}", solve_component_manifest.display()))?;
    meta.outputs
        .push(util::file_digest(&solve_component_manifest)?);

    Ok(SolveBuildOutput {
        solve_core_wasm: Some(solve_core_digest),
        solve_artifact: Some(artifact_doc),
    })
}

fn build_http_adapter_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    out_dir: &Path,
    component_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Value>> {
    build_adapter_component(
        store,
        component_profile,
        out_dir,
        component_profile_ref,
        "http-adapter",
        HTTP_ADAPTER_MANIFEST,
        HTTP_ADAPTER_LOCK,
        HTTP_ADAPTER_COMPONENT_WASM,
        "x07:http-adapter@0.1.0",
        "proxy-with-solve",
        meta,
        diagnostics,
    )
}

fn build_cli_adapter_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    out_dir: &Path,
    component_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Value>> {
    build_adapter_component(
        store,
        component_profile,
        out_dir,
        component_profile_ref,
        "cli-adapter",
        CLI_ADAPTER_MANIFEST,
        CLI_ADAPTER_LOCK,
        CLI_ADAPTER_COMPONENT_WASM,
        "x07:cli-adapter@0.1.0",
        "command-with-solve",
        meta,
        diagnostics,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_adapter_component(
    store: &SchemaStore,
    _component_profile: &ComponentProfileDoc,
    out_dir: &Path,
    component_profile_ref: &Value,
    kind: &str,
    cargo_manifest: &str,
    cargo_lock: &str,
    built_component_wasm: &str,
    wit_package: &str,
    wit_world: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Value>> {
    let manifest_path = Path::new(cargo_manifest);
    if let Ok(d) = util::file_digest(manifest_path) {
        meta.inputs.push(d);
    }
    let lock_path = Path::new(cargo_lock);
    if lock_path.is_file() {
        if let Ok(d) = util::file_digest(lock_path) {
            meta.inputs.push(d);
        }
    }

    let cargo_args = vec![
        "build".to_string(),
        "--release".to_string(),
        "--locked".to_string(),
        "--target".to_string(),
        "wasm32-wasip2".to_string(),
        "--manifest-path".to_string(),
        manifest_path.display().to_string(),
    ];
    let cargo_out = match cmdutil::run_cmd_capture("cargo", &cargo_args) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_CARGO_BUILD_SPAWN_FAILED",
                Stage::Run,
                "cargo build (adapter)",
                &err,
            ));
            None
        }
    };
    if cargo_out.as_ref().is_some_and(|o| !o.status.success()) {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_CARGO_BUILD_FAILED",
            Stage::Run,
            "cargo build (adapter)",
            cargo_out.as_ref().unwrap().code,
            &cargo_out.as_ref().unwrap().stderr,
        ));
        return Ok(None);
    }
    if cargo_out.is_none() {
        return Ok(None);
    }

    let built_path = Path::new(built_component_wasm);
    if !built_path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_ADAPTER_BUILD_OUTPUT_MISSING",
            Severity::Error,
            Stage::Run,
            format!("adapter build output missing: {}", built_path.display()),
        ));
        return Ok(None);
    }

    let out_component = out_dir.join(format!("{kind}.component.wasm"));
    if let Err(err) = std::fs::copy(built_path, &out_component) {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_ADAPTER_COMPONENT_COPY_FAILED",
            Stage::Run,
            format!(
                "failed to copy adapter component {} -> {}",
                built_path.display(),
                out_component.display()
            ),
            &anyhow::Error::new(err),
        ));
        return Ok(None);
    }

    let out_digest = match util::file_digest(&out_component) {
        Ok(d) => d,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_ADAPTER_COMPONENT_DIGEST_FAILED",
                Severity::Error,
                Stage::Run,
                format!(
                    "failed to digest adapter component {}: {err:#}",
                    out_component.display()
                ),
            ));
            return Ok(None);
        }
    };
    meta.outputs.push(out_digest.clone());

    let artifact_manifest = out_dir.join(format!("{kind}.component.wasm.manifest.json"));

    let cargo_ver = toolchain::tool_first_line("cargo", &["--version"]).ok();
    let rustc_ver = toolchain::tool_first_line("rustc", &["--version"]).ok();

    let mut toolchain_obj = serde_json::Map::new();
    toolchain_obj.insert("x07_wasm".to_string(), json!(env!("CARGO_PKG_VERSION")));
    if let Some(v) = cargo_ver {
        toolchain_obj.insert("cargo".to_string(), json!(v));
    }
    if let Some(v) = rustc_ver {
        toolchain_obj.insert("rustc".to_string(), json!(v));
    }

    let artifact_doc = json!({
      "schema_version": "x07.wasm.component.artifact@0.1.0",
      "artifact_id": format!("{kind}-{}", &out_digest.sha256[..16]),
      "kind": kind,
      "component": out_digest,
      "wit": { "package": wit_package, "world": wit_world },
      "profiles": {
        "component": component_profile_ref
      },
      "toolchain": Value::Object(toolchain_obj)
    });

    let diags = match store.validate(
        "https://x07.io/spec/x07-wasm.component.artifact.schema.json",
        &artifact_doc,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SCHEMA_VALIDATE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return Ok(None);
        }
    };
    if diags.iter().any(|d| d.severity == Severity::Error) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_COMPONENT_ARTIFACT_SCHEMA_INVALID",
            Severity::Error,
            Stage::Run,
            format!("internal error: component artifact failed schema validation: {diags:?}"),
        ));
        return Ok(None);
    }

    let bytes = match report::canon::canonical_json_bytes(&artifact_doc) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_CANONICAL_JSON_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return Ok(None);
        }
    };
    if let Err(err) = std::fs::write(&artifact_manifest, &bytes) {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_COMPONENT_ARTIFACT_WRITE_FAILED",
            Stage::Run,
            format!("failed to write artifact: {}", artifact_manifest.display()),
            &anyhow::Error::new(err),
        ));
        return Ok(None);
    }
    match util::file_digest(&artifact_manifest) {
        Ok(d) => meta.outputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_COMPONENT_ARTIFACT_DIGEST_FAILED",
                Severity::Error,
                Stage::Run,
                format!(
                    "failed to digest component artifact {}: {err:#}",
                    artifact_manifest.display()
                ),
            ));
            return Ok(None);
        }
    }

    Ok(Some(artifact_doc))
}

fn compile_one_c(
    cc: &str,
    base_cflags: &[String],
    src: &Path,
    out: &Path,
    extra_args: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let mut args: Vec<String> = base_cflags.to_vec();
    args.extend(extra_args.iter().cloned());
    args.extend([
        "-c".to_string(),
        src.display().to_string(),
        "-o".to_string(),
        out.display().to_string(),
    ]);
    let outp = match cmdutil::run_cmd_capture(cc, &args) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_CLANG_SPAWN_FAILED",
                Stage::Codegen,
                "clang",
                &err,
            ));
            return Ok(());
        }
    };
    if !outp.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_CLANG_FAILED",
            Stage::Codegen,
            "clang",
            outp.code,
            &outp.stderr,
        ));
    }
    Ok(())
}

fn resolve_wit_package_dir(
    store: &SchemaStore,
    wit_index_path: &Path,
    pkg_id: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<PathBuf> {
    let digest = util::file_digest(wit_index_path)?;
    meta.inputs.push(digest.clone());

    let bytes = std::fs::read(wit_index_path)
        .with_context(|| format!("read: {}", wit_index_path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", wit_index_path.display()))?;
    diagnostics.extend(store.validate(
        "https://x07.io/spec/x07-arch.wit.index.schema.json",
        &doc_json,
    )?);

    #[derive(Deserialize)]
    struct WitIndexDoc {
        packages: Vec<WitIndexPackageRef>,
    }
    #[derive(Deserialize)]
    struct WitIndexPackageRef {
        id: String,
        path: String,
    }
    let doc: WitIndexDoc = serde_json::from_value(doc_json).context("parse arch/wit index")?;
    let entry = doc
        .packages
        .iter()
        .find(|p| p.id == pkg_id)
        .ok_or_else(|| anyhow::anyhow!("wit package not found in index: {pkg_id:?}"))?;
    Ok(PathBuf::from(&entry.path))
}

fn run_tool_cmd_capture(
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<cmdutil::CmdCapture> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().with_context(|| format!("run: {program}"))?;
    let code = out.status.code().unwrap_or(1);
    Ok(cmdutil::CmdCapture {
        status: out.status,
        code,
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

fn load_component_profile(
    store: &SchemaStore,
    index_path: &Path,
    profile_id: Option<&str>,
    profile_file: Option<&PathBuf>,
) -> Result<LoadedComponentProfile> {
    if let Some(path) = profile_file {
        return load_component_profile_file(store, path, None);
    }

    let index_digest = util::file_digest(index_path)?;
    let index_bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let index_doc: Value = serde_json::from_slice(&index_bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-arch.wasm.component.index.schema.json",
        &index_doc,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid component index: {}", index_digest.path);
    }

    let idx: ComponentIndexDoc = serde_json::from_value(index_doc)
        .with_context(|| format!("parse component index: {}", index_path.display()))?;
    let default_id = idx
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
        .unwrap_or_else(|| DEFAULT_COMPONENT_PROFILE_ID.to_string());
    let wanted = profile_id.unwrap_or(&default_id);
    let entry = idx
        .profiles
        .iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| anyhow::anyhow!("profile id not found in component index: {wanted:?}"))?;
    let profile_path = PathBuf::from(&entry.path);
    load_component_profile_file(store, &profile_path, Some(index_digest))
}

fn load_component_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_digest: Option<report::meta::FileDigest>,
) -> Result<LoadedComponentProfile> {
    let digest = util::file_digest(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-wasm.component.profile.schema.json",
        &doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid component profile: {}", digest.path);
    }
    let doc: ComponentProfileDoc = serde_json::from_value(doc_json)
        .with_context(|| format!("parse component profile doc: {}", path.display()))?;

    if doc.cfg.compose.mode.trim() != "wac-plug_v1" {
        anyhow::bail!("unsupported compose mode: {:?}", doc.cfg.compose.mode);
    }
    if doc.cfg.toolchain.wac.cmd.trim().is_empty() {
        anyhow::bail!("toolchain.wac.cmd must be non-empty");
    }
    if doc.cfg.toolchain.wasmtime.cmd.trim().is_empty() {
        anyhow::bail!("toolchain.wasmtime.cmd must be non-empty");
    }
    if doc.cfg.targets.http_world.trim().is_empty() || doc.cfg.targets.cli_world.trim().is_empty() {
        anyhow::bail!("invalid targets config");
    }
    if doc.cfg.targets.http_package.trim().is_empty()
        || doc.cfg.targets.cli_package.trim().is_empty()
    {
        anyhow::bail!("invalid targets config");
    }

    Ok(LoadedComponentProfile {
        digest,
        doc,
        index_digest,
    })
}

#[allow(clippy::too_many_arguments)]
fn component_build_report_doc(
    project_digest: &report::meta::FileDigest,
    component_profile_ref: Option<&Value>,
    wasm_profile_ref: Option<&Value>,
    out_dir: &Path,
    emit: ComponentBuildEmit,
    solve_core_wasm: Option<&report::meta::FileDigest>,
    artifacts: Vec<Value>,
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
) -> Value {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    json!({
      "schema_version": "x07.wasm.component.build.report@0.1.0",
      "command": "x07-wasm.component.build",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "emit": match emit {
          ComponentBuildEmit::Solve => "solve",
          ComponentBuildEmit::HttpAdapter => "http-adapter",
          ComponentBuildEmit::CliAdapter => "cli-adapter",
          ComponentBuildEmit::All => "all",
        },
        "project": project_digest,
        "out_dir": out_dir.display().to_string(),
        "component_profile": component_profile_ref.cloned().unwrap_or_else(|| json!({"id":"unknown","v":0})),
        "wasm_profile": wasm_profile_ref.cloned().unwrap_or_else(|| json!({"id":"unknown","v":0})),
        "solve_core_wasm": match solve_core_wasm {
          Some(d) => json!(d),
          None => Value::Null,
        },
        "artifacts": artifacts,
      }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_schema_accepts_placeholder() {
        let store = SchemaStore::new().unwrap();
        let raw_argv: Vec<OsString> = Vec::new();
        let meta = report::meta::tool_meta(&raw_argv, std::time::Instant::now());
        let doc = json!({
          "schema_version": "x07.wasm.component.build.report@0.1.0",
          "command": "x07-wasm.component.build",
          "ok": false,
          "exit_code": 1,
          "diagnostics": [],
          "meta": meta,
          "result": {
            "emit": "all",
            "project": { "path": "x07.json", "sha256": "0".repeat(64), "bytes_len": 0 },
            "out_dir": "target/x07-wasm/component",
            "component_profile": { "id": "component_release", "v": 1 },
            "wasm_profile": { "id": "wasm_release", "v": 1 },
            "solve_core_wasm": null,
            "artifacts": []
          }
        });
        let diags = store
            .validate(
                "https://x07.io/spec/x07-wasm.component.build.report.schema.json",
                &doc,
            )
            .unwrap();
        assert!(diags.is_empty(), "schema diags: {diags:?}");
    }
}

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters;
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
const HTTP_PROXY_BINDGEN_C_TEMPLATE: &str =
    include_str!("../support/component/http_proxy_bindgen_c_template.c");
const CLI_COMMAND_BINDGEN_C_TEMPLATE: &str =
    include_str!("../support/component/cli_command_bindgen_c_template.c");
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
struct NativeHttpBudgets {
    max_request_body_bytes: u64,
    max_response_body_bytes: u64,
    max_headers: u32,
    max_header_bytes_total: u64,
    max_path_bytes: u64,
    max_query_bytes: u64,
    max_envelope_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeCliBudgets {
    max_stdin_bytes: u64,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeHttpCfg {
    mode: String,
    package: String,
    world: String,
    budgets: NativeHttpBudgets,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeCliCfg {
    mode: String,
    package: String,
    world: String,
    budgets: NativeCliBudgets,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeTargetsCfg {
    http: NativeHttpCfg,
    cli: NativeCliCfg,
}

#[derive(Debug, Clone, Deserialize)]
struct ComponentProfileCfg {
    wit_index_path: String,
    toolchain: ComponentProfileToolchain,
    componentize: ComponentizeCfg,
    compose: ComposeCfg,
    targets: TargetsCfg,
    native_targets: NativeTargetsCfg,
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
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
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
            ComponentBuildEmit::Http => {
                if let Err(err) = build_composed_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    crate::cli::ComponentComposeAdapterKind::Http,
                    &mut meta,
                    &mut diagnostics,
                    &mut solve_core_wasm_digest,
                    &mut artifacts,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }
            }
            ComponentBuildEmit::Cli => {
                if let Err(err) = build_composed_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    crate::cli::ComponentComposeAdapterKind::Cli,
                    &mut meta,
                    &mut diagnostics,
                    &mut solve_core_wasm_digest,
                    &mut artifacts,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }
            }
            ComponentBuildEmit::HttpNative => {
                let built = match build_http_native_component(
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
                        None
                    }
                };
                if let Some(a) = built {
                    artifacts.push(a);
                }
            }
            ComponentBuildEmit::CliNative => {
                let built = match build_cli_native_component(
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
                        None
                    }
                };
                if let Some(a) = built {
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
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
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

                if let Err(err) = build_composed_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    crate::cli::ComponentComposeAdapterKind::Http,
                    &mut meta,
                    &mut diagnostics,
                    &mut solve_core_wasm_digest,
                    &mut artifacts,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }

                if let Err(err) = build_composed_component(
                    &store,
                    &loaded_component_profile.doc,
                    &loaded_wasm_profile.doc,
                    &args.wasm_index,
                    args.wasm_profile_file.as_deref(),
                    &args.project,
                    &args.out_dir,
                    &component_profile_ref,
                    &wasm_profile_ref,
                    crate::cli::ComponentComposeAdapterKind::Cli,
                    &mut meta,
                    &mut diagnostics,
                    &mut solve_core_wasm_digest,
                    &mut artifacts,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
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
fn build_composed_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    wasm_index: &Path,
    wasm_profile_file: Option<&Path>,
    project_path: &Path,
    out_dir: &Path,
    component_profile_ref: &Value,
    wasm_profile_ref: &Value,
    adapter_kind: crate::cli::ComponentComposeAdapterKind,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    solve_core_wasm_digest_out: &mut Option<report::meta::FileDigest>,
    artifacts_out: &mut Vec<Value>,
) -> Result<()> {
    // Build solve component exactly once per invocation; re-use for http+cli in --emit all.
    if solve_core_wasm_digest_out.is_none() {
        let built = build_solve_component(
            store,
            component_profile,
            wasm_profile,
            wasm_index,
            wasm_profile_file,
            project_path,
            out_dir,
            component_profile_ref,
            wasm_profile_ref,
            meta,
            diagnostics,
        )?;
        *solve_core_wasm_digest_out = built.solve_core_wasm;
        if let Some(a) = built.solve_artifact {
            artifacts_out.push(a);
        }
    }

    let adapter_artifact = match adapter_kind {
        crate::cli::ComponentComposeAdapterKind::Http => build_http_adapter_component(
            store,
            component_profile,
            out_dir,
            component_profile_ref,
            meta,
            diagnostics,
        )?,
        crate::cli::ComponentComposeAdapterKind::Cli => build_cli_adapter_component(
            store,
            component_profile,
            out_dir,
            component_profile_ref,
            meta,
            diagnostics,
        )?,
    };
    if let Some(a) = adapter_artifact {
        artifacts_out.push(a);
    }

    let solve_component = out_dir.join("solve.component.wasm");
    let adapter_component = match adapter_kind {
        crate::cli::ComponentComposeAdapterKind::Http => {
            out_dir.join("http-adapter.component.wasm")
        }
        crate::cli::ComponentComposeAdapterKind::Cli => out_dir.join("cli-adapter.component.wasm"),
    };
    let out_component = match adapter_kind {
        crate::cli::ComponentComposeAdapterKind::Http => out_dir.join("http.component.wasm"),
        crate::cli::ComponentComposeAdapterKind::Cli => out_dir.join("cli.component.wasm"),
    };
    let out_manifest = out_dir.join(
        out_component
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("component.wasm"))
            .to_string_lossy()
            .to_string()
            + ".manifest.json",
    );

    if !solve_component.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "missing solve component output for compose: {}",
                solve_component.display()
            ),
        ));
        return Ok(());
    }
    if !adapter_component.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "missing adapter component output for compose: {}",
                adapter_component.display()
            ),
        ));
        return Ok(());
    }

    let compose_report_out = match adapter_kind {
        crate::cli::ComponentComposeAdapterKind::Http => {
            out_dir.join("http.component.compose.report.json")
        }
        crate::cli::ComponentComposeAdapterKind::Cli => {
            out_dir.join("cli.component.compose.report.json")
        }
    };
    if let Some(parent) = compose_report_out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let nested_machine = MachineArgs {
        json: Some("".to_string()),
        report_json: None,
        report_out: Some(compose_report_out.clone()),
        quiet_json: true,
        json_schema: false,
        json_schema_id: false,
    };
    let compose_args = crate::cli::ComponentComposeArgs {
        adapter: adapter_kind,
        solve: solve_component.clone(),
        adapter_component: Some(adapter_component.clone()),
        out: out_component.clone(),
        artifact_out: Some(out_manifest.clone()),
        targets_check: false,
    };

    let code = crate::component::compose::cmd_component_compose(
        &[],
        Scope::ComponentCompose,
        &nested_machine,
        compose_args,
    )?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm component compose failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(compose_report_out.display().to_string()),
        );
        diagnostics.push(d);
        return Ok(());
    }

    if out_component.is_file() {
        if let Ok(d) = util::file_digest(&out_component) {
            meta.outputs.push(d);
        }
    }
    if out_manifest.is_file() {
        if let Ok(d) = util::file_digest(&out_manifest) {
            meta.outputs.push(d);
        }
        if let Ok(bytes) = std::fs::read(&out_manifest) {
            if let Ok(doc) = serde_json::from_slice::<Value>(&bytes) {
                artifacts_out.push(doc);
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_solve_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    wasm_index: &Path,
    wasm_profile_file: Option<&Path>,
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

    if wasm_profile.codegen_backend == arch::CodegenBackend::NativeX07WasmV1 {
        return build_solve_component_native_x07_wasm_v1(
            store,
            component_profile,
            wasm_profile,
            wasm_index,
            wasm_profile_file,
            project_path,
            out_dir,
            component_profile_ref,
            wasm_profile_ref,
            meta,
            diagnostics,
        );
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

#[allow(clippy::too_many_arguments)]
fn build_solve_component_native_x07_wasm_v1(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    wasm_index: &Path,
    wasm_profile_file: Option<&Path>,
    project_path: &Path,
    out_dir: &Path,
    component_profile_ref: &Value,
    wasm_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<SolveBuildOutput> {
    let cfg = &component_profile.cfg;

    let solve_core_raw_wasm = out_dir.join("solve.core.raw.wasm");
    let solve_core_raw_manifest = out_dir.join("solve.core.raw.wasm.manifest.json");
    let solve_core_wasm = out_dir.join("solve.core.wasm");
    let solve_core_embedded_wasm = out_dir.join("solve.core.embedded.wasm");

    let solve_component_wasm = out_dir.join("solve.component.wasm");
    let solve_component_manifest = out_dir.join("solve.component.wasm.manifest.json");

    // Step A: build solve-pure core wasm using the Phase 7 native backend.
    let nested_report_out = out_dir.join("solve.core.raw.wasm.build.report.json");
    if let Some(parent) = nested_report_out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let nested_machine = MachineArgs {
        json: Some("".to_string()),
        report_json: None,
        report_out: Some(nested_report_out.clone()),
        quiet_json: true,
        json_schema: false,
        json_schema_id: false,
    };
    let build_args = crate::cli::BuildArgs {
        project: project_path.to_path_buf(),
        profile: wasm_profile_file.is_none().then(|| wasm_profile.id.clone()),
        profile_file: wasm_profile_file.map(Path::to_path_buf),
        index: wasm_index.to_path_buf(),
        codegen_backend: None,
        emit_dir: Some(out_dir.join("solve.core.emit")),
        out: Some(solve_core_raw_wasm.clone()),
        artifact_out: Some(solve_core_raw_manifest.clone()),
        no_manifest: false,
        check_exports: true,
    };

    let code = crate::wasm::build::cmd_build(&[], Scope::Build, &nested_machine, build_args)?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_INTERNAL_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm wasm build failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(nested_report_out.display().to_string()),
        );
        diagnostics.push(d);
        return Ok(SolveBuildOutput {
            solve_core_wasm: None,
            solve_artifact: None,
        });
    }

    if solve_core_raw_manifest.is_file() {
        meta.outputs
            .push(util::file_digest(&solve_core_raw_manifest)?);
    }
    if solve_core_raw_wasm.is_file() {
        meta.outputs.push(util::file_digest(&solve_core_raw_wasm)?);
    }

    // Step B: inject legacy canonical ABI exports for x07:solve/handler.
    let raw_bytes = std::fs::read(&solve_core_raw_wasm)
        .with_context(|| format!("read: {}", solve_core_raw_wasm.display()))?;

    let arena_cap_bytes: u32 = wasm_profile
        .defaults
        .arena_cap_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("arena_cap_bytes out of range for wasm32"))?;
    let max_output_bytes: u32 = wasm_profile
        .defaults
        .max_output_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("max_output_bytes out of range for wasm32"))?;

    let glued_bytes =
        inject_solve_handler_legacy_abi(&raw_bytes, arena_cap_bytes, max_output_bytes)?;
    std::fs::write(&solve_core_wasm, &glued_bytes)
        .with_context(|| format!("write: {}", solve_core_wasm.display()))?;
    let solve_core_digest = util::file_digest(&solve_core_wasm)?;
    meta.outputs.push(solve_core_digest.clone());

    // Step C: wasm-tools component embed + new (no clang/wasi-sdk path).
    let solve_wit_dir = resolve_wit_package_dir(
        store,
        Path::new(&cfg.wit_index_path),
        &cfg.componentize.solve_package,
        meta,
        diagnostics,
    )?;

    let mut wasm_tools_embed_args = cfg.toolchain.wasm_tools.args.clone();
    wasm_tools_embed_args.extend([
        "component".to_string(),
        "embed".to_string(),
        solve_wit_dir.display().to_string(),
        "--world".to_string(),
        cfg.componentize.solve_world.clone(),
        solve_core_wasm.display().to_string(),
        "-o".to_string(),
        solve_core_embedded_wasm.display().to_string(),
    ]);
    let embed_out = match run_tool_cmd_capture(
        &cfg.toolchain.wasm_tools.cmd,
        &wasm_tools_embed_args,
        &cfg.toolchain.wasm_tools.env,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WASM_TOOLS_SPAWN_FAILED",
                Stage::Link,
                "wasm-tools component embed",
                &err,
            ));
            return Ok(SolveBuildOutput {
                solve_core_wasm: Some(solve_core_digest),
                solve_artifact: None,
            });
        }
    };
    if !embed_out.status.success() {
        diagnostics.push(cmdutil::diag_cmd_failed(
            "X07WASM_WASM_TOOLS_FAILED",
            Stage::Link,
            "wasm-tools component embed",
            embed_out.code,
            &embed_out.stderr,
        ));
        return Ok(SolveBuildOutput {
            solve_core_wasm: Some(solve_core_digest),
            solve_artifact: None,
        });
    }
    if solve_core_embedded_wasm.is_file() {
        meta.outputs
            .push(util::file_digest(&solve_core_embedded_wasm)?);
    }

    let mut wasm_tools_new_args = cfg.toolchain.wasm_tools.args.clone();
    wasm_tools_new_args.extend([
        "component".to_string(),
        "new".to_string(),
        solve_core_embedded_wasm.display().to_string(),
        "-o".to_string(),
        solve_component_wasm.display().to_string(),
    ]);
    let comp_out = match run_tool_cmd_capture(
        &cfg.toolchain.wasm_tools.cmd,
        &wasm_tools_new_args,
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

    // Step D: write component artifact manifest.
    let x07_semver = toolchain::x07_semver().unwrap_or_else(|_| "0.0.0".to_string());
    let wasm_tools_ver =
        toolchain::tool_first_line(&cfg.toolchain.wasm_tools.cmd, &["--version"]).ok();

    let mut toolchain_obj = serde_json::Map::new();
    toolchain_obj.insert("x07_wasm".to_string(), json!(env!("CARGO_PKG_VERSION")));
    toolchain_obj.insert("x07".to_string(), json!(x07_semver));
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

fn inject_solve_handler_legacy_abi(
    core_wasm: &[u8],
    arena_cap_bytes: u32,
    max_output_bytes: u32,
) -> Result<Vec<u8>> {
    use std::borrow::Cow;

    use wasm_encoder::{
        CodeSection, ConstExpr, CustomSection, ExportKind, ExportSection, FunctionSection,
        GlobalSection, GlobalType, MemorySection, MemoryType, Module, TypeSection, ValType,
    };
    use wasmparser::{ExternalKind, Parser, Payload};

    let mut func_types: Vec<wasmparser::FuncType> = Vec::new();
    let mut func_type_indices: Vec<u32> = Vec::new();
    let mut memories: Vec<wasmparser::MemoryType> = Vec::new();
    let mut globals: Vec<wasmparser::Global<'_>> = Vec::new();
    let mut exports: Vec<(String, wasmparser::ExternalKind, u32)> = Vec::new();
    let mut custom_sections: Vec<(String, Vec<u8>)> = Vec::new();

    let mut data_count: Option<u32> = None;
    let mut data_segments_raw: Vec<Vec<u8>> = Vec::new();

    let mut code_bodies_raw: Vec<Vec<u8>> = Vec::new();

    for payload in Parser::new(0).parse_all(core_wasm) {
        match payload.context("parse wasm payload")? {
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types() {
                    func_types.push(ty.context("parse wasm type")?);
                }
            }
            Payload::ImportSection(reader) => {
                if reader.count() != 0 {
                    anyhow::bail!("inject: imports are not supported");
                }
            }
            Payload::FunctionSection(reader) => {
                for f in reader {
                    func_type_indices.push(f.context("parse wasm function")?);
                }
            }
            Payload::MemorySection(reader) => {
                for m in reader {
                    memories.push(m.context("parse wasm memory")?);
                }
            }
            Payload::TableSection(_) => anyhow::bail!("inject: table section not supported"),
            Payload::TagSection(_) => anyhow::bail!("inject: tag section not supported"),
            Payload::GlobalSection(reader) => {
                for g in reader {
                    globals.push(g.context("parse wasm global")?);
                }
            }
            Payload::ExportSection(reader) => {
                for e in reader {
                    let e = e.context("parse wasm export")?;
                    exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            Payload::StartSection { .. } => anyhow::bail!("inject: start section not supported"),
            Payload::ElementSection(_) => anyhow::bail!("inject: element section not supported"),
            Payload::DataCountSection { count, .. } => data_count = Some(count),
            Payload::DataSection(reader) => {
                for s in reader {
                    let s = s.context("parse wasm data segment")?;
                    data_segments_raw.push(core_wasm[s.range.start..s.range.end].to_vec());
                }
            }
            Payload::CodeSectionStart { .. } => {}
            Payload::CodeSectionEntry(body) => {
                let r = body.range();
                code_bodies_raw.push(core_wasm[r.start..r.end].to_vec());
            }
            Payload::CustomSection(reader) => {
                custom_sections.push((reader.name().to_string(), reader.data().to_vec()));
            }
            Payload::UnknownSection { id, .. } => {
                anyhow::bail!("inject: unknown section id={id} not supported")
            }
            Payload::End(_) | Payload::Version { .. } => {}
            _ => anyhow::bail!("inject: unsupported wasm payload"),
        }
    }

    if code_bodies_raw.len() != func_type_indices.len() {
        anyhow::bail!(
            "inject: function section len != code section len ({} != {})",
            func_type_indices.len(),
            code_bodies_raw.len()
        );
    }

    let mut x07_solve_v2_func_idx: Option<u32> = None;
    let mut heap_base_global_idx: Option<u32> = None;

    for (name, kind, index) in &exports {
        match (name.as_str(), kind) {
            ("x07_solve_v2", ExternalKind::Func) => x07_solve_v2_func_idx = Some(*index),
            ("__heap_base", ExternalKind::Global) => heap_base_global_idx = Some(*index),
            _ => {}
        }
    }

    let x07_solve_v2_func_idx = x07_solve_v2_func_idx
        .ok_or_else(|| anyhow::anyhow!("inject: missing export x07_solve_v2"))?;
    let heap_base_global_idx = heap_base_global_idx
        .ok_or_else(|| anyhow::anyhow!("inject: missing export __heap_base"))?;

    // Types appended after the module's original types.
    let type_idx_cabi_realloc =
        u32::try_from(func_types.len()).map_err(|_| anyhow::anyhow!("inject: too many types"))?;
    let type_idx_cabi_post = type_idx_cabi_realloc + 1;
    let type_idx_solve = type_idx_cabi_realloc + 2;

    // Functions appended after the module's original defined functions.
    let existing_defined_funcs = u32::try_from(func_type_indices.len())
        .map_err(|_| anyhow::anyhow!("inject: too many functions"))?;
    let func_idx_cabi_realloc = existing_defined_funcs;
    let func_idx_cabi_post = existing_defined_funcs + 1;
    let func_idx_solve = existing_defined_funcs + 2;

    // Globals appended after the module's original globals.
    let existing_globals =
        u32::try_from(globals.len()).map_err(|_| anyhow::anyhow!("inject: too many globals"))?;
    let global_idx_heap_ptr = existing_globals;
    let global_idx_arena_ptr = existing_globals + 1;
    let global_idx_ret_area_ptr = existing_globals + 2;

    // Re-encode the module with injected glue, copying existing bodies/segments.
    let mut module = Module::new();

    let mut types = TypeSection::new();
    for ty in &func_types {
        let params = ty
            .params()
            .iter()
            .map(wasm_valtype)
            .collect::<Result<Vec<_>>>()?;
        let results = ty
            .results()
            .iter()
            .map(wasm_valtype)
            .collect::<Result<Vec<_>>>()?;
        types.ty().function(params, results);
    }
    // cabi_realloc: (i32, i32, i32, i32) -> i32
    types.ty().function(
        [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        [ValType::I32],
    );
    // cabi_post_*: (i32) -> ()
    types.ty().function([ValType::I32], []);
    // solve: (i32, i32) -> i32
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I32]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    for idx in &func_type_indices {
        functions.function(*idx);
    }
    functions.function(type_idx_cabi_realloc);
    functions.function(type_idx_cabi_post);
    functions.function(type_idx_solve);
    module.section(&functions);

    let mut memories_sec = MemorySection::new();
    for m in &memories {
        if m.memory64 {
            anyhow::bail!("inject: memory64 not supported");
        }
        memories_sec.memory(MemoryType {
            minimum: m.initial,
            maximum: m.maximum,
            memory64: m.memory64,
            shared: m.shared,
            page_size_log2: m.page_size_log2,
        });
    }
    if !memories_sec.is_empty() {
        module.section(&memories_sec);
    }

    let mut globals_sec = GlobalSection::new();
    for g in &globals {
        let init = const_expr_from_wasmparser(&g.init_expr)?;
        globals_sec.global(
            GlobalType {
                val_type: wasm_valtype(&g.ty.content_type)?,
                mutable: g.ty.mutable,
                shared: g.ty.shared,
            },
            &init,
        );
    }
    let zero = ConstExpr::i32_const(0);
    globals_sec.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &zero,
    );
    globals_sec.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &zero,
    );
    globals_sec.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &zero,
    );
    module.section(&globals_sec);

    let mut exports_sec = ExportSection::new();
    for (name, kind, index) in &exports {
        exports_sec.export(name, export_kind(*kind)?, *index);
    }
    // New exports for legacy WIT canonical ABI glue.
    for name in [
        "cabi_realloc",
        "cabi_post_x07:solve/handler@0.1.0#solve",
        "x07:solve/handler@0.1.0#solve",
    ] {
        if exports.iter().any(|(n, _, _)| n == name) {
            anyhow::bail!("inject: export already exists: {name:?}");
        }
    }
    exports_sec.export("cabi_realloc", ExportKind::Func, func_idx_cabi_realloc);
    exports_sec.export(
        "cabi_post_x07:solve/handler@0.1.0#solve",
        ExportKind::Func,
        func_idx_cabi_post,
    );
    exports_sec.export(
        "x07:solve/handler@0.1.0#solve",
        ExportKind::Func,
        func_idx_solve,
    );
    module.section(&exports_sec);

    if let Some(count) = data_count {
        module.section(&wasm_encoder::DataCountSection { count });
    }

    let mut code = CodeSection::new();
    for body in &code_bodies_raw {
        code.raw(body);
    }
    code.function(&emit_cabi_realloc_func(
        heap_base_global_idx,
        global_idx_heap_ptr,
    ));
    code.function(&emit_cabi_post_func());
    code.function(&emit_solve_wrapper_func(
        x07_solve_v2_func_idx,
        func_idx_cabi_realloc,
        arena_cap_bytes,
        max_output_bytes,
        global_idx_arena_ptr,
        global_idx_ret_area_ptr,
    ));
    module.section(&code);

    if !data_segments_raw.is_empty() {
        let mut data = wasm_encoder::DataSection::new();
        for seg in &data_segments_raw {
            data.raw(seg);
        }
        module.section(&data);
    }

    for (name, data) in &custom_sections {
        module.section(&CustomSection {
            name: Cow::Owned(name.clone()),
            data: Cow::Borrowed(data.as_slice()),
        });
    }

    let bytes = module.finish();
    wasmparser::Validator::new()
        .validate_all(&bytes)
        .context("validate injected wasm")?;
    Ok(bytes)
}

fn wasm_valtype(v: &wasmparser::ValType) -> Result<wasm_encoder::ValType> {
    Ok(match v {
        wasmparser::ValType::I32 => wasm_encoder::ValType::I32,
        wasmparser::ValType::I64 => wasm_encoder::ValType::I64,
        wasmparser::ValType::F32 => wasm_encoder::ValType::F32,
        wasmparser::ValType::F64 => wasm_encoder::ValType::F64,
        wasmparser::ValType::V128 => wasm_encoder::ValType::V128,
        wasmparser::ValType::Ref(_) => {
            anyhow::bail!("inject: reference types not supported")
        }
    })
}

fn export_kind(k: wasmparser::ExternalKind) -> Result<wasm_encoder::ExportKind> {
    Ok(match k {
        wasmparser::ExternalKind::Func => wasm_encoder::ExportKind::Func,
        wasmparser::ExternalKind::Table => wasm_encoder::ExportKind::Table,
        wasmparser::ExternalKind::Memory => wasm_encoder::ExportKind::Memory,
        wasmparser::ExternalKind::Global => wasm_encoder::ExportKind::Global,
        wasmparser::ExternalKind::Tag => wasm_encoder::ExportKind::Tag,
    })
}

fn const_expr_from_wasmparser(expr: &wasmparser::ConstExpr<'_>) -> Result<wasm_encoder::ConstExpr> {
    let mut r = expr.get_binary_reader();
    let mut b = r.read_bytes(r.bytes_remaining())?.to_vec();
    if b.last().copied() != Some(0x0b) {
        anyhow::bail!("inject: const expr missing end");
    }
    b.pop();
    Ok(wasm_encoder::ConstExpr::raw(b))
}

fn emit_cabi_realloc_func(
    heap_base_global_idx: u32,
    heap_ptr_global_idx: u32,
) -> wasm_encoder::Function {
    // (func (param ptr old_size align new_size) (result i32) ...)
    let mut f = wasm_encoder::Function::new([(5, wasm_encoder::ValType::I32)]);

    let p_ptr = 0;
    let p_old_size = 1;
    let p_align = 2;
    let p_new_size = 3;

    let l_heap = 4;
    let l_aligned = 5;
    let l_next = 6;
    let l_n = 7;
    let l_i = 8;

    let mut ins = f.instructions();

    // if new_size == 0 { return align }
    ins.local_get(p_new_size)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .local_get(p_align)
        .return_()
        .end();

    // if align == 0 { trap }
    ins.local_get(p_align)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .unreachable()
        .end();

    // heap = global.heap_ptr
    ins.global_get(heap_ptr_global_idx).local_set(l_heap);

    // if heap == 0 { heap = __heap_base; global.heap_ptr = heap }
    ins.local_get(l_heap)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .global_get(heap_base_global_idx)
        .local_set(l_heap)
        .local_get(l_heap)
        .global_set(heap_ptr_global_idx)
        .end();

    // aligned = if align <= 1 { heap } else { (heap + (align-1)) & (-align) }
    ins.local_get(p_align)
        .i32_const(1)
        .i32_le_u()
        .if_(wasm_encoder::BlockType::Empty)
        .local_get(l_heap)
        .local_set(l_aligned)
        .else_()
        .local_get(l_heap)
        .local_get(p_align)
        .i32_const(1)
        .i32_sub()
        .i32_add()
        .i32_const(0)
        .local_get(p_align)
        .i32_sub()
        .i32_and()
        .local_set(l_aligned)
        .end();

    // next = aligned + new_size; global.heap_ptr = next
    ins.local_get(l_aligned)
        .local_get(p_new_size)
        .i32_add()
        .local_set(l_next)
        .local_get(l_next)
        .global_set(heap_ptr_global_idx);

    // if ptr != 0 && old_size != 0 { copy min(old,new) bytes }
    ins.local_get(p_ptr)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .else_()
        .local_get(p_old_size)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .else_();

    // n = old_size
    ins.local_get(p_old_size).local_set(l_n);
    // if new_size < n { n = new_size }
    ins.local_get(p_new_size)
        .local_get(l_n)
        .i32_lt_u()
        .if_(wasm_encoder::BlockType::Empty)
        .local_get(p_new_size)
        .local_set(l_n)
        .end();
    // i = 0
    ins.i32_const(0).local_set(l_i);

    ins.block(wasm_encoder::BlockType::Empty);
    ins.loop_(wasm_encoder::BlockType::Empty);
    // if i >= n break
    ins.local_get(l_i).local_get(l_n).i32_ge_u().br_if(1);

    // *(aligned+i) = *(ptr+i)
    ins.local_get(l_aligned)
        .local_get(l_i)
        .i32_add()
        .local_get(p_ptr)
        .local_get(l_i)
        .i32_add()
        .i32_load8_u(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        })
        .i32_store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        });

    // i++
    ins.local_get(l_i)
        .i32_const(1)
        .i32_add()
        .local_set(l_i)
        .br(0);
    ins.end();
    ins.end();

    ins.end(); // end old_size !=0 if
    ins.end(); // end ptr !=0 if

    ins.local_get(l_aligned).end();

    f
}

fn emit_cabi_post_func() -> wasm_encoder::Function {
    let mut f = wasm_encoder::Function::new([]);
    f.instructions().end();
    f
}

fn emit_solve_wrapper_func(
    x07_solve_v2_func_idx: u32,
    cabi_realloc_func_idx: u32,
    arena_cap_bytes: u32,
    max_output_bytes: u32,
    arena_ptr_global_idx: u32,
    ret_area_ptr_global_idx: u32,
) -> wasm_encoder::Function {
    // (func (param input_ptr input_len) (result i32) ...)
    let mut f = wasm_encoder::Function::new([(6, wasm_encoder::ValType::I32)]);

    let p_input_ptr = 0;
    let p_input_len = 1;

    let l_ret_area = 2;
    let l_arena = 3;
    let l_out_ptr = 4;
    let l_out_len = 5;
    let l_out_end = 6;
    let l_arena_end = 7;

    let mut ins = f.instructions();

    // if input_len < 0 { trap }
    ins.local_get(p_input_len)
        .i32_const(0)
        .i32_lt_s()
        .if_(wasm_encoder::BlockType::Empty)
        .unreachable()
        .end();

    // ret_area = global.ret_area_ptr; if 0 { ret_area = cabi_realloc(NULL,0,4,8); global=ret_area }
    ins.global_get(ret_area_ptr_global_idx)
        .local_set(l_ret_area)
        .local_get(l_ret_area)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .i32_const(0)
        .i32_const(0)
        .i32_const(4)
        .i32_const(8)
        .call(cabi_realloc_func_idx)
        .local_set(l_ret_area)
        .local_get(l_ret_area)
        .global_set(ret_area_ptr_global_idx)
        .end();

    // arena = global.arena_ptr; if 0 { arena = cabi_realloc(NULL,0,8,arena_cap); global=arena }
    ins.global_get(arena_ptr_global_idx)
        .local_set(l_arena)
        .local_get(l_arena)
        .i32_eqz()
        .if_(wasm_encoder::BlockType::Empty)
        .i32_const(0)
        .i32_const(0)
        .i32_const(8)
        .i32_const(arena_cap_bytes as i32)
        .call(cabi_realloc_func_idx)
        .local_set(l_arena)
        .local_get(l_arena)
        .global_set(arena_ptr_global_idx)
        .end();

    // call x07_solve_v2(ret_area, arena, arena_cap, input_ptr, input_len)
    ins.local_get(l_ret_area)
        .local_get(l_arena)
        .i32_const(arena_cap_bytes as i32)
        .local_get(p_input_ptr)
        .local_get(p_input_len)
        .call(x07_solve_v2_func_idx);

    // out_ptr = *(ret_area+0); out_len = *(ret_area+4)
    ins.local_get(l_ret_area)
        .i32_load(wasm_encoder::MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        })
        .local_set(l_out_ptr);
    ins.local_get(l_ret_area)
        .i32_load(wasm_encoder::MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        })
        .local_set(l_out_len);

    // if out_len > max_output { trap }
    ins.local_get(l_out_len)
        .i32_const(max_output_bytes as i32)
        .i32_gt_u()
        .if_(wasm_encoder::BlockType::Empty)
        .unreachable()
        .end();

    // out_end = out_ptr + out_len
    ins.local_get(l_out_ptr)
        .local_get(l_out_len)
        .i32_add()
        .local_set(l_out_end);

    // arena_end = arena + arena_cap
    ins.local_get(l_arena)
        .i32_const(arena_cap_bytes as i32)
        .i32_add()
        .local_set(l_arena_end);

    // if out_ptr < arena { trap }
    ins.local_get(l_out_ptr)
        .local_get(l_arena)
        .i32_lt_u()
        .if_(wasm_encoder::BlockType::Empty)
        .unreachable()
        .end();

    // if out_end > arena_end { trap }
    ins.local_get(l_out_end)
        .local_get(l_arena_end)
        .i32_gt_u()
        .if_(wasm_encoder::BlockType::Empty)
        .unreachable()
        .end();

    // return ret_area
    ins.local_get(l_ret_area).end();

    f
}

#[allow(clippy::too_many_arguments)]
fn build_http_native_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    project_path: &Path,
    out_dir: &Path,
    component_profile_ref: &Value,
    wasm_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Value>> {
    let cfg = &component_profile.cfg;

    if cfg.native_targets.http.mode.trim() != "native-http-proxy_v1" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_NATIVE_HTTP_MODE_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!(
                "unsupported cfg.native_targets.http.mode: {:?}",
                cfg.native_targets.http.mode
            ),
        ));
        return Ok(None);
    }

    let program_c = out_dir.join("program.c");
    let x07_h = out_dir.join("x07.h");
    let shim_c = out_dir.join("phase0_shim.c");
    let shim_o = out_dir.join("phase0_shim.o");

    let wit_out_dir = out_dir.join("wit-bindgen-http");
    let wit_proxy_c = wit_out_dir.join("proxy.c");
    let wit_proxy_o = out_dir.join("proxy_bindgen.o");
    let wit_component_type_o = wit_out_dir.join("proxy_component_type.o");

    let http_glue_c = out_dir.join("http_glue.c");
    let http_glue_o = out_dir.join("http_glue.o");

    let http_core_wasm = out_dir.join("http.core.wasm");
    let http_component_wasm = out_dir.join("http.component.wasm");
    let http_component_manifest = out_dir.join("http.component.wasm.manifest.json");

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
        return Ok(None);
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
            return Ok(None);
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
        return Ok(None);
    }
    meta.outputs.push(util::file_digest(&program_c)?);
    meta.outputs.push(util::file_digest(&x07_h)?);

    // Step B: wit-bindgen c for wasi:http/proxy (bundled deps)
    let http_wit_dir = resolve_wit_package_dir(
        store,
        Path::new(&cfg.wit_index_path),
        &cfg.native_targets.http.package,
        meta,
        diagnostics,
    )?;

    let http_wit_dir_arg = match crate::wit::bundle::bundle_for_wit_path(
        store,
        Path::new(&cfg.wit_index_path),
        &http_wit_dir,
        meta,
        diagnostics,
    ) {
        Ok(Some(bundle)) => bundle.dir,
        Ok(None) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_BUNDLE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to bundle WIT package: {}", http_wit_dir.display()),
            ));
            return Ok(None);
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_BUNDLE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return Ok(None);
        }
    };

    let mut wit_bindgen_args = cfg.toolchain.wit_bindgen.args.clone();
    wit_bindgen_args.extend([
        "c".to_string(),
        http_wit_dir_arg.display().to_string(),
        "--world".to_string(),
        cfg.native_targets.http.world.clone(),
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
            return Ok(None);
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
        return Ok(None);
    }

    meta.outputs.push(util::file_digest(&wit_proxy_c)?);
    meta.outputs
        .push(util::file_digest(&wit_out_dir.join("proxy.h"))?);
    meta.outputs.push(util::file_digest(&wit_component_type_o)?);

    // Step C: write shims + glue
    std::fs::write(&shim_c, PHASE0_SHIM_C)
        .with_context(|| format!("write: {}", shim_c.display()))?;
    meta.outputs.push(util::file_digest(&shim_c)?);

    let b = &cfg.native_targets.http.budgets;
    let glue = format!(
        "#define X07_SOLVE_ARENA_CAP_BYTES ({arena}u)\n#define X07_SOLVE_MAX_OUTPUT_BYTES ({max_out}u)\n#define X07_NATIVE_HTTP_MAX_REQUEST_BODY_BYTES ({max_req}ull)\n#define X07_NATIVE_HTTP_MAX_RESPONSE_BODY_BYTES ({max_resp}ull)\n#define X07_NATIVE_HTTP_MAX_HEADERS ({max_hdrs}u)\n#define X07_NATIVE_HTTP_MAX_HEADER_BYTES_TOTAL ({max_hdr_bytes}ull)\n#define X07_NATIVE_HTTP_MAX_PATH_BYTES ({max_path}ull)\n#define X07_NATIVE_HTTP_MAX_QUERY_BYTES ({max_query}ull)\n#define X07_NATIVE_HTTP_MAX_ENVELOPE_BYTES ({max_env}ull)\n{template}",
        arena = wasm_profile.defaults.arena_cap_bytes,
        max_out = wasm_profile.defaults.max_output_bytes,
        max_req = b.max_request_body_bytes,
        max_resp = b.max_response_body_bytes,
        max_hdrs = b.max_headers,
        max_hdr_bytes = b.max_header_bytes_total,
        max_path = b.max_path_bytes,
        max_query = b.max_query_bytes,
        max_env = b.max_envelope_bytes,
        template = HTTP_PROXY_BINDGEN_C_TEMPLATE
    );
    std::fs::write(&http_glue_c, glue)
        .with_context(|| format!("write: {}", http_glue_c.display()))?;
    meta.outputs.push(util::file_digest(&http_glue_c)?);

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
        &wit_proxy_c,
        &wit_proxy_o,
        &[format!("-I{}", wit_out_dir.display())],
        diagnostics,
    )?;
    compile_one_c(
        &cc,
        &cflags,
        &http_glue_c,
        &http_glue_o,
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
    ldflags.push(wit_proxy_o.display().to_string());
    ldflags.push(http_glue_o.display().to_string());
    ldflags.push(wit_component_type_o.display().to_string());
    ldflags.extend(["-o".to_string(), http_core_wasm.display().to_string()]);
    let ld_out = match cmdutil::run_cmd_capture(&linker, &ldflags) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WASM_LD_SPAWN_FAILED",
                Stage::Link,
                "wasm-ld",
                &err,
            ));
            return Ok(None);
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
        return Ok(None);
    }

    let http_core_digest = util::file_digest(&http_core_wasm)?;
    meta.outputs.push(http_core_digest);

    // Step F: wasm-tools component new (component)
    let mut wasm_tools_args = cfg.toolchain.wasm_tools.args.clone();
    wasm_tools_args.extend([
        "component".to_string(),
        "new".to_string(),
        http_core_wasm.display().to_string(),
        "-o".to_string(),
        http_component_wasm.display().to_string(),
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
            return Ok(None);
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
        return Ok(None);
    }

    let http_component_digest = util::file_digest(&http_component_wasm)?;
    meta.outputs.push(http_component_digest.clone());

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
    if let Some(v) = clang_ver {
        toolchain_obj.insert("clang".to_string(), json!(v));
    }
    if let Some(v) = wasm_ld_ver {
        toolchain_obj.insert("wasm_ld".to_string(), json!(v));
    }
    if let Some(v) = wit_bindgen_ver {
        toolchain_obj.insert("wit_bindgen".to_string(), json!(v));
    }
    if let Some(v) = wasm_tools_ver {
        toolchain_obj.insert("wasm_tools".to_string(), json!(v));
    }

    let artifact_doc = json!({
      "schema_version": "x07.wasm.component.artifact@0.1.0",
      "artifact_id": format!("http-{}", &http_component_digest.sha256[..16]),
      "kind": "http",
      "component": http_component_digest,
      "wit": { "package": cfg.native_targets.http.package.clone(), "world": cfg.native_targets.http.world.clone() },
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
        return Ok(None);
    }

    let bytes = report::canon::canonical_json_bytes(&artifact_doc)?;
    std::fs::write(&http_component_manifest, &bytes)
        .with_context(|| format!("write: {}", http_component_manifest.display()))?;
    meta.outputs
        .push(util::file_digest(&http_component_manifest)?);

    Ok(Some(artifact_doc))
}

#[allow(clippy::too_many_arguments)]
fn build_cli_native_component(
    store: &SchemaStore,
    component_profile: &ComponentProfileDoc,
    wasm_profile: &arch::WasmProfileDoc,
    project_path: &Path,
    out_dir: &Path,
    component_profile_ref: &Value,
    wasm_profile_ref: &Value,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Value>> {
    let cfg = &component_profile.cfg;

    if cfg.native_targets.cli.mode.trim() != "native-cli-command_v1" {
        diagnostics.push(Diagnostic::new(
            "X07WASM_NATIVE_CLI_MODE_UNSUPPORTED",
            Severity::Error,
            Stage::Parse,
            format!(
                "unsupported cfg.native_targets.cli.mode: {:?}",
                cfg.native_targets.cli.mode
            ),
        ));
        return Ok(None);
    }

    let program_c = out_dir.join("program.c");
    let x07_h = out_dir.join("x07.h");
    let shim_c = out_dir.join("phase0_shim.c");
    let shim_o = out_dir.join("phase0_shim.o");

    let wit_out_dir = out_dir.join("wit-bindgen-cli");
    let wit_command_c = wit_out_dir.join("command.c");
    let wit_command_o = out_dir.join("command_bindgen.o");
    let wit_component_type_o = wit_out_dir.join("command_component_type.o");

    let cli_glue_c = out_dir.join("cli_glue.c");
    let cli_glue_o = out_dir.join("cli_glue.o");

    let cli_core_wasm = out_dir.join("cli.core.wasm");
    let cli_component_wasm = out_dir.join("cli.component.wasm");
    let cli_component_manifest = out_dir.join("cli.component.wasm.manifest.json");

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
        return Ok(None);
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
            return Ok(None);
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
        return Ok(None);
    }
    meta.outputs.push(util::file_digest(&program_c)?);
    meta.outputs.push(util::file_digest(&x07_h)?);

    // Step B: wit-bindgen c for wasi:cli/command (bundled deps)
    let cli_wit_dir = resolve_wit_package_dir(
        store,
        Path::new(&cfg.wit_index_path),
        &cfg.native_targets.cli.package,
        meta,
        diagnostics,
    )?;

    let cli_wit_dir_arg = match crate::wit::bundle::bundle_for_wit_path(
        store,
        Path::new(&cfg.wit_index_path),
        &cli_wit_dir,
        meta,
        diagnostics,
    ) {
        Ok(Some(bundle)) => bundle.dir,
        Ok(None) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_BUNDLE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to bundle WIT package: {}", cli_wit_dir.display()),
            ));
            return Ok(None);
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_BUNDLE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("{err:#}"),
            ));
            return Ok(None);
        }
    };

    let mut wit_bindgen_args = cfg.toolchain.wit_bindgen.args.clone();
    wit_bindgen_args.extend([
        "c".to_string(),
        cli_wit_dir_arg.display().to_string(),
        "--world".to_string(),
        cfg.native_targets.cli.world.clone(),
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
            return Ok(None);
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
        return Ok(None);
    }

    meta.outputs.push(util::file_digest(&wit_command_c)?);
    meta.outputs
        .push(util::file_digest(&wit_out_dir.join("command.h"))?);
    meta.outputs.push(util::file_digest(&wit_component_type_o)?);

    // Step C: write shims + glue
    std::fs::write(&shim_c, PHASE0_SHIM_C)
        .with_context(|| format!("write: {}", shim_c.display()))?;
    meta.outputs.push(util::file_digest(&shim_c)?);

    let b = &cfg.native_targets.cli.budgets;
    let glue = format!(
        "#define X07_SOLVE_ARENA_CAP_BYTES ({arena}u)\n#define X07_SOLVE_MAX_OUTPUT_BYTES ({max_out}u)\n#define X07_NATIVE_CLI_MAX_STDIN_BYTES ({max_in}ull)\n#define X07_NATIVE_CLI_MAX_STDOUT_BYTES ({max_out_bytes}ull)\n#define X07_NATIVE_CLI_MAX_STDERR_BYTES ({max_err}ull)\n{template}",
        arena = wasm_profile.defaults.arena_cap_bytes,
        max_out = wasm_profile.defaults.max_output_bytes,
        max_in = b.max_stdin_bytes,
        max_out_bytes = b.max_stdout_bytes,
        max_err = b.max_stderr_bytes,
        template = CLI_COMMAND_BINDGEN_C_TEMPLATE
    );
    std::fs::write(&cli_glue_c, glue)
        .with_context(|| format!("write: {}", cli_glue_c.display()))?;
    meta.outputs.push(util::file_digest(&cli_glue_c)?);

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
        &wit_command_c,
        &wit_command_o,
        &[format!("-I{}", wit_out_dir.display())],
        diagnostics,
    )?;
    compile_one_c(
        &cc,
        &cflags,
        &cli_glue_c,
        &cli_glue_o,
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
    ldflags.push(wit_command_o.display().to_string());
    ldflags.push(cli_glue_o.display().to_string());
    ldflags.push(wit_component_type_o.display().to_string());
    ldflags.extend(["-o".to_string(), cli_core_wasm.display().to_string()]);
    let ld_out = match cmdutil::run_cmd_capture(&linker, &ldflags) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WASM_LD_SPAWN_FAILED",
                Stage::Link,
                "wasm-ld",
                &err,
            ));
            return Ok(None);
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
        return Ok(None);
    }

    let cli_core_digest = util::file_digest(&cli_core_wasm)?;
    meta.outputs.push(cli_core_digest);

    // Step F: wasm-tools component new (component)
    let mut wasm_tools_args = cfg.toolchain.wasm_tools.args.clone();
    wasm_tools_args.extend([
        "component".to_string(),
        "new".to_string(),
        cli_core_wasm.display().to_string(),
        "-o".to_string(),
        cli_component_wasm.display().to_string(),
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
            return Ok(None);
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
        return Ok(None);
    }

    let cli_component_digest = util::file_digest(&cli_component_wasm)?;
    meta.outputs.push(cli_component_digest.clone());

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
    if let Some(v) = clang_ver {
        toolchain_obj.insert("clang".to_string(), json!(v));
    }
    if let Some(v) = wasm_ld_ver {
        toolchain_obj.insert("wasm_ld".to_string(), json!(v));
    }
    if let Some(v) = wit_bindgen_ver {
        toolchain_obj.insert("wit_bindgen".to_string(), json!(v));
    }
    if let Some(v) = wasm_tools_ver {
        toolchain_obj.insert("wasm_tools".to_string(), json!(v));
    }

    let artifact_doc = json!({
      "schema_version": "x07.wasm.component.artifact@0.1.0",
      "artifact_id": format!("cli-{}", &cli_component_digest.sha256[..16]),
      "kind": "cli",
      "component": cli_component_digest,
      "wit": { "package": cfg.native_targets.cli.package.clone(), "world": cfg.native_targets.cli.world.clone() },
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
        return Ok(None);
    }

    let bytes = report::canon::canonical_json_bytes(&artifact_doc)?;
    std::fs::write(&cli_component_manifest, &bytes)
        .with_context(|| format!("write: {}", cli_component_manifest.display()))?;
    meta.outputs
        .push(util::file_digest(&cli_component_manifest)?);

    Ok(Some(artifact_doc))
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
    let out_component = out_dir.join(format!("{kind}.component.wasm"));
    let use_source = adapters::adapters_from_source_enabled();

    let bytes = if use_source {
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

        let built_path = Path::new(built_component_wasm);
        let Some(bytes) = adapters::build_wasm32_wasip2_release_bytes(
            manifest_path,
            built_path,
            diagnostics,
            "cargo build (adapter)",
        ) else {
            return Ok(None);
        };
        std::borrow::Cow::Owned(bytes)
    } else {
        let embedded = match kind {
            "http-adapter" => adapters::EMBEDDED_HTTP_ADAPTER_COMPONENT_WASM,
            "cli-adapter" => adapters::EMBEDDED_CLI_ADAPTER_COMPONENT_WASM,
            other => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_ADAPTER_KIND_UNSUPPORTED",
                    Severity::Error,
                    Stage::Run,
                    format!("unsupported embedded adapter kind: {other:?}"),
                ));
                return Ok(None);
            }
        };
        meta.inputs.push(adapters::embedded_digest(
            &format!("embedded:{kind}.component.wasm"),
            embedded,
        ));
        std::borrow::Cow::Borrowed(embedded)
    };

    if let Err(err) = adapters::write_bytes(&out_component, bytes.as_ref()) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_ADAPTER_COMPONENT_COPY_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to write adapter component {}: {err:#}",
                out_component.display()
            ),
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
          ComponentBuildEmit::Http => "http",
          ComponentBuildEmit::Cli => "cli",
          ComponentBuildEmit::HttpNative => "http-native",
          ComponentBuildEmit::CliNative => "cli-native",
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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adapters;
use crate::cli::{MachineArgs, Scope, WebUiBuildArgs, WebUiBuildFormat};
use crate::cmdutil;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_WEB_UI_PROFILE_ID: &str = "web_ui_release";

const DEFAULT_COMPONENT_PROFILE_INDEX: &str = "arch/wasm/component/index.x07wasm.component.json";

const HOST_INDEX_HTML: &str = "index.html";
const HOST_BOOTSTRAP_JS: &str = "bootstrap.js";
const HOST_APP_HOST_MJS: &str = "app-host.mjs";
const HOST_MAIN_MJS: &str = "main.mjs";
const HOST_SNAPSHOT_JSON: &[u8] = include_bytes!("../../../../vendor/x07-web-ui/snapshot.json");
const HOST_INDEX_HTML_BYTES: &[u8] =
    include_bytes!("../../../../vendor/x07-web-ui/host/index.html");
const HOST_BOOTSTRAP_JS_BYTES: &[u8] =
    include_bytes!("../../../../vendor/x07-web-ui/host/bootstrap.js");
const HOST_APP_HOST_MJS_BYTES: &[u8] =
    include_bytes!("../../../../vendor/x07-web-ui/host/app-host.mjs");
const HOST_MAIN_MJS_BYTES: &[u8] = include_bytes!("../../../../vendor/x07-web-ui/host/main.mjs");

const WIT_WEB_UI_APP_DIR: &str = "wit/x07/web_ui/0.2.0";
const WIT_WEB_UI_APP_WORLD: &str = "web-ui-app";

const WEB_UI_ADAPTER_MANIFEST: &str = "guest/web-ui-adapter/Cargo.toml";
const WEB_UI_ADAPTER_COMPONENT_WASM: &str =
    "guest/web-ui-adapter/target/wasm32-wasip2/release/x07_wasm_web_ui_adapter.wasm";

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexDoc {
    profiles: Vec<WebUiIndexProfileRef>,
    #[serde(default)]
    defaults: Option<WebUiIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexDefaults {
    default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WebUiIndexProfileRef {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebUiProfileServeDefaults {
    port: u16,
    strict_mime: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebUiProfileBuildDefaults {
    format: String,
    emit_host_assets: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebUiProfileDefaults {
    arena_cap_bytes: u64,
    max_input_bytes: u64,
    max_output_bytes: u64,
    serve: WebUiProfileServeDefaults,
    build: WebUiProfileBuildDefaults,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebUiProfileDoc {
    schema_version: String,
    id: String,
    v: u64,
    wasm_profile_id: String,
    defaults: WebUiProfileDefaults,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Debug, Clone)]
struct LoadedWebUiProfile {
    digest: report::meta::FileDigest,
    doc: WebUiProfileDoc,
    index_digest: Option<report::meta::FileDigest>,
}

#[derive(Debug, Clone, Deserialize)]
struct VendoredSnapshotDoc {
    source: String,
    git_sha: String,
}

pub fn cmd_web_ui_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WebUiBuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let loaded_web_ui_profile = match load_web_ui_profile(
        &store,
        &args.index,
        args.profile.as_deref(),
        args.profile_file.as_ref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WEB_UI_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            let report_doc = web_ui_build_report_doc(
                &meta,
                &diagnostics,
                None,
                "core",
                &args.out_dir,
                Vec::new(),
                None,
            );
            let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };
    meta.inputs.push(loaded_web_ui_profile.digest.clone());
    if let Some(d) = loaded_web_ui_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let web_ui_profile_ref = json!({
      "id": loaded_web_ui_profile.doc.id.clone(),
      "v": loaded_web_ui_profile.doc.v,
    });

    let loaded_wasm_profile = match crate::arch::load_profile(
        &store,
        &args.wasm_index,
        Some(&loaded_web_ui_profile.doc.wasm_profile_id),
        None,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WASM_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            let report_doc = web_ui_build_report_doc(
                &meta,
                &diagnostics,
                Some(&web_ui_profile_ref),
                "core",
                &args.out_dir,
                Vec::new(),
                None,
            );
            let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
            store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
            return Ok(exit_code);
        }
    };
    meta.inputs.push(loaded_wasm_profile.digest.clone());
    if let Some(d) = loaded_wasm_profile.index_digest.clone() {
        meta.inputs.push(d);
    }

    let format = match args.format {
        Some(WebUiBuildFormat::Core) => WebUiBuildFormat::Core,
        Some(WebUiBuildFormat::Component) => WebUiBuildFormat::Component,
        None => match loaded_web_ui_profile.doc.defaults.build.format.as_str() {
            "core" => WebUiBuildFormat::Core,
            "component" => WebUiBuildFormat::Component,
            other => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_PROFILE_BUILD_FORMAT_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("unsupported defaults.build.format: {other:?}"),
                ));
                WebUiBuildFormat::Core
            }
        },
    };

    if args.clean {
        if let Err(err) = std::fs::remove_dir_all(&args.out_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_BUILD_CLEAN_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to clean out dir {}: {err}", args.out_dir.display()),
                ));
            }
        }
    }

    if let Err(err) = std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("create dir: {}", args.out_dir.display()))
    {
        diagnostics.push(cmdutil::diag_io_failed(
            "X07WASM_WEB_UI_BUILD_OUTDIR_CREATE_FAILED",
            Stage::Run,
            format!("failed to create out dir: {}", args.out_dir.display()),
            &err,
        ));
    }

    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    // Always emit the resolved web-ui profile into dist for reproducible test/serve defaults.
    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        let out_profile = args.out_dir.join("web-ui.profile.json");
        let doc_json = serde_json::to_value(&loaded_web_ui_profile.doc)?;
        let bytes = report::canon::canonical_pretty_json_bytes(&doc_json)?;
        if let Err(err) = std::fs::write(&out_profile, &bytes) {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_WEB_UI_BUILD_PROFILE_WRITE_FAILED",
                Stage::Run,
                format!("failed to write profile: {}", out_profile.display()),
                &anyhow::Error::new(err),
            ));
        } else if let Ok(d) = util::file_digest(&out_profile) {
            meta.outputs.push(d.clone());
            artifacts.push(d);
        }
    }

    // Always emit the resolved wasm profile into dist so replay/test can enforce the exact runtime
    // limits that were used to build/run the reducer wasm.
    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        let out_profile = args.out_dir.join("wasm.profile.json");
        let doc_json = serde_json::to_value(&loaded_wasm_profile.doc)?;
        let bytes = report::canon::canonical_pretty_json_bytes(&doc_json)?;
        if let Err(err) = std::fs::write(&out_profile, &bytes) {
            diagnostics.push(cmdutil::diag_io_failed(
                "X07WASM_WEB_UI_BUILD_WASM_PROFILE_WRITE_FAILED",
                Stage::Run,
                format!("failed to write wasm profile: {}", out_profile.display()),
                &anyhow::Error::new(err),
            ));
        } else if let Ok(d) = util::file_digest(&out_profile) {
            meta.outputs.push(d.clone());
            artifacts.push(d);
        }
    }

    let mut host_snapshot: Option<Value> = None;
    let mut format_str = match format {
        WebUiBuildFormat::Core => "core".to_string(),
        WebUiBuildFormat::Component => "component".to_string(),
    };

    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        match format {
            WebUiBuildFormat::Core => {
                let (step_diags, step_artifacts, step_host_snapshot) =
                    build_core_bundle(&store, &args, &loaded_web_ui_profile, &loaded_wasm_profile)?;
                diagnostics.extend(step_diags);
                for d in step_artifacts {
                    meta.outputs.push(d.clone());
                    artifacts.push(d);
                }
                host_snapshot = step_host_snapshot;
            }
            WebUiBuildFormat::Component => {
                let (step_diags, step_artifacts, step_host_snapshot) = build_component_bundle(
                    &store,
                    &args,
                    &loaded_web_ui_profile,
                    &loaded_wasm_profile,
                )?;
                diagnostics.extend(step_diags);
                for d in step_artifacts {
                    meta.outputs.push(d.clone());
                    artifacts.push(d);
                }
                host_snapshot = step_host_snapshot;
                format_str = "component".to_string();
            }
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);
    let report_doc = web_ui_build_report_doc(
        &meta,
        &diagnostics,
        Some(&web_ui_profile_ref),
        &format_str,
        &args.out_dir,
        artifacts,
        host_snapshot,
    );
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn web_ui_build_report_doc(
    meta: &report::meta::ReportMeta,
    diagnostics: &[Diagnostic],
    profile_ref: Option<&Value>,
    format: &str,
    out_dir: &Path,
    artifacts: Vec<report::meta::FileDigest>,
    host_snapshot: Option<Value>,
) -> Value {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(diagnostics);

    let mut result = json!({
      "profile": profile_ref.cloned().unwrap_or_else(|| json!({"id":"web_ui_release","v":1})),
      "format": format,
      "dist_dir": out_dir.display().to_string(),
      "artifacts": artifacts,
    });
    if let Some(hs) = host_snapshot {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("host_snapshot".to_string(), hs);
        }
    }

    json!({
      "schema_version": "x07.wasm.web_ui.build.report@0.1.0",
      "command": "x07-wasm.web-ui.build",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": result,
    })
}

fn build_core_bundle(
    _store: &SchemaStore,
    args: &WebUiBuildArgs,
    web_ui_profile: &LoadedWebUiProfile,
    _wasm_profile: &crate::arch::LoadedProfile,
) -> Result<(
    Vec<Diagnostic>,
    Vec<report::meta::FileDigest>,
    Option<Value>,
)> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    let project_name = project_name_for_manifest(args.project.as_path());
    let emit_dir = PathBuf::from("target")
        .join("x07-wasm")
        .join("web_ui")
        .join(&project_name)
        .join(&web_ui_profile.doc.wasm_profile_id);

    let nested_report_out = emit_dir.join("wasm.build.report.json");
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
        project: args.project.clone(),
        profile: Some(web_ui_profile.doc.wasm_profile_id.clone()),
        profile_file: None,
        index: args.wasm_index.clone(),
        codegen_backend: None,
        emit_dir: Some(emit_dir.clone()),
        out: Some(args.out_dir.join("app.wasm")),
        artifact_out: Some(args.out_dir.join("app.wasm.manifest.json")),
        no_manifest: false,
        check_exports: true,
    };

    let code = crate::wasm::build::cmd_build(&[], Scope::Build, &nested_machine, build_args)?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_WASM_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm build failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(nested_report_out.display().to_string()),
        );
        diagnostics.push(d);
    }

    for p in [
        args.out_dir.join("app.wasm"),
        args.out_dir.join("app.wasm.manifest.json"),
    ] {
        if p.is_file() {
            match util::file_digest(&p) {
                Ok(d) => artifacts.push(d),
                Err(err) => diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_BUILD_DIGEST_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to digest artifact {}: {err:#}", p.display()),
                )),
            }
        }
    }

    let host_snapshot = if web_ui_profile.doc.defaults.build.emit_host_assets {
        match emit_host_assets(args.out_dir.as_path()) {
            Ok((hs, mut hs_artifacts)) => {
                artifacts.append(&mut hs_artifacts);
                Some(hs)
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_BUILD_HOST_ASSETS_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                None
            }
        }
    } else {
        None
    };

    Ok((diagnostics, artifacts, host_snapshot))
}

fn build_component_bundle(
    _store: &SchemaStore,
    args: &WebUiBuildArgs,
    web_ui_profile: &LoadedWebUiProfile,
    _wasm_profile: &crate::arch::LoadedProfile,
) -> Result<(
    Vec<Diagnostic>,
    Vec<report::meta::FileDigest>,
    Option<Value>,
)> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    let project_name = project_name_for_manifest(&args.project);
    let component_out_dir = PathBuf::from("target")
        .join("x07-wasm")
        .join("web_ui")
        .join(&project_name)
        .join("component");

    let nested_report_out = component_out_dir.join("component.build.report.json");
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

    let component_build_args = crate::cli::ComponentBuildArgs {
        project: args.project.clone(),
        profile: None,
        profile_file: None,
        index: PathBuf::from(DEFAULT_COMPONENT_PROFILE_INDEX),
        wasm_profile: Some(web_ui_profile.doc.wasm_profile_id.clone()),
        wasm_profile_file: None,
        wasm_index: args.wasm_index.clone(),
        out_dir: component_out_dir.clone(),
        emit: crate::cli::ComponentBuildEmit::Solve,
        clean: true,
    };

    let code = crate::component::build::cmd_component_build(
        &[],
        Scope::ComponentBuild,
        &nested_machine,
        component_build_args,
    )?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm component build failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(nested_report_out.display().to_string()),
        );
        diagnostics.push(d);
    }

    let solve_component = component_out_dir.join("solve.component.wasm");
    let solve_core = component_out_dir.join("solve.core.wasm");
    if !solve_component.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_SOLVE_COMPONENT_MISSING",
            Severity::Error,
            Stage::Run,
            format!("missing solve component: {}", solve_component.display()),
        ));
    }
    if !solve_core.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_SOLVE_CORE_MISSING",
            Severity::Error,
            Stage::Run,
            format!("missing solve core wasm: {}", solve_core.display()),
        ));
    }

    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Ok((diagnostics, artifacts, None));
    }

    // Copy core module into dist for the core-wasm host path (fallback / debugging).
    let out_app_wasm = args.out_dir.join("app.wasm");
    if let Err(err) = std::fs::copy(&solve_core, &out_app_wasm) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_COPY_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to copy solve.core.wasm -> {}: {err}",
                out_app_wasm.display()
            ),
        ));
        return Ok((diagnostics, artifacts, None));
    }
    artifacts.push(util::file_digest(&out_app_wasm)?);

    // Build web-ui adapter component.
    let adapter_component = component_out_dir.join("web-ui-adapter.component.wasm");
    build_web_ui_adapter_component(&adapter_component, &mut diagnostics)?;

    // Compose the runnable web-ui component.
    let out_component = args.out_dir.join("app.component.wasm");
    let wac_args = vec![
        "plug".to_string(),
        "--plug".to_string(),
        solve_component.display().to_string(),
        adapter_component.display().to_string(),
        "-o".to_string(),
        out_component.display().to_string(),
    ];
    let wac_out = match cmdutil::run_cmd_capture("wac", &wac_args) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_WAC_PLUG_SPAWN_FAILED",
                Stage::Run,
                "wac plug",
                &err,
            ));
            None
        }
    };
    if let Some(out) = wac_out.as_ref() {
        if !out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_WAC_PLUG_FAILED",
                Stage::Run,
                "wac plug",
                out.code,
                &out.stderr,
            ));
        }
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Ok((diagnostics, artifacts, None));
    }
    artifacts.push(util::file_digest(&out_component)?);

    // Optional sanity: verify the composed component targets the web-ui world.
    let wac_targets_args = vec![
        "targets".to_string(),
        "--wit".to_string(),
        WIT_WEB_UI_APP_DIR.to_string(),
        "--world".to_string(),
        WIT_WEB_UI_APP_WORLD.to_string(),
        out_component.display().to_string(),
    ];
    if let Ok(out) = cmdutil::run_cmd_capture("wac", &wac_targets_args) {
        if !out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_WAC_TARGETS_FAILED",
                Stage::Run,
                "wac targets",
                out.code,
                &out.stderr,
            ));
        }
    }

    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Ok((diagnostics, artifacts, None));
    }

    // Transpile the component into browser/Node-runnable ESM + core wasm modules.
    let transpiled_dir = args.out_dir.join("transpiled");
    if let Err(err) = std::fs::create_dir_all(&transpiled_dir) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_IO_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to create transpiled dir: {err}"),
        ));
        return Ok((diagnostics, artifacts, None));
    }

    // Ensure jco exists before running transpile.
    if crate::toolchain::tool_first_line("jco", &["--version"]).is_err() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_TOOL_MISSING_JCO",
            Severity::Error,
            Stage::Run,
            "jco not found on PATH (required for --format component)".to_string(),
        ));
        return Ok((diagnostics, artifacts, None));
    }

    let jco_args = vec![
        "transpile".to_string(),
        out_component.display().to_string(),
        "-o".to_string(),
        transpiled_dir.display().to_string(),
        "--name".to_string(),
        "app".to_string(),
        "-q".to_string(),
    ];
    let jco_out = match cmdutil::run_cmd_capture("jco", &jco_args) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(cmdutil::diag_cmd_spawn_failed(
                "X07WASM_JCO_TRANSPILE_SPAWN_FAILED",
                Stage::Run,
                "jco transpile",
                &err,
            ));
            None
        }
    };
    if let Some(out) = jco_out.as_ref() {
        if !out.status.success() {
            diagnostics.push(cmdutil::diag_cmd_failed(
                "X07WASM_JCO_TRANSPILE_FAILED",
                Stage::Run,
                "jco transpile",
                out.code,
                &out.stderr,
            ));
        }
    }

    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Ok((diagnostics, artifacts, None));
    }

    // Provide a stable import path expected by the canonical host (index.html tries ./transpiled/app.mjs).
    let wrapper = transpiled_dir.join("app.mjs");
    let wrapper_bytes = b"export * from \"./app.js\";\n";
    if let Err(err) = std::fs::write(&wrapper, wrapper_bytes) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WEB_UI_BUILD_IO_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to write wrapper {}: {err}", wrapper.display()),
        ));
        return Ok((diagnostics, artifacts, None));
    }

    // Record all transpiled artifacts (stable ordering).
    let mut transpiled_files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&transpiled_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() {
                transpiled_files.push(p);
            }
        }
    }
    transpiled_files.sort();
    for p in transpiled_files {
        if let Ok(d) = util::file_digest(&p) {
            artifacts.push(d);
        }
    }

    let host_snapshot = if web_ui_profile.doc.defaults.build.emit_host_assets {
        match emit_host_assets(args.out_dir.as_path()) {
            Ok((hs, mut hs_artifacts)) => {
                artifacts.append(&mut hs_artifacts);
                Some(hs)
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WEB_UI_BUILD_HOST_ASSETS_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                None
            }
        }
    } else {
        None
    };

    Ok((diagnostics, artifacts, host_snapshot))
}

fn load_web_ui_profile(
    store: &SchemaStore,
    index_path: &PathBuf,
    profile_id: Option<&str>,
    profile_file: Option<&PathBuf>,
) -> Result<LoadedWebUiProfile> {
    if let Some(path) = profile_file {
        return load_web_ui_profile_file(store, path, None);
    }

    let index_digest = util::file_digest(index_path)?;
    let index_bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let index_doc_json: Value = serde_json::from_slice(&index_bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;

    let diags = store.validate(
        "https://x07.io/spec/x07-arch.web_ui.index.schema.json",
        &index_doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid web-ui index: {}", index_digest.path);
    }

    let idx: WebUiIndexDoc = serde_json::from_value(index_doc_json)
        .with_context(|| format!("parse index doc: {}", index_path.display()))?;
    let default_id = idx
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
        .unwrap_or_else(|| DEFAULT_WEB_UI_PROFILE_ID.to_string());
    let wanted = profile_id.unwrap_or(&default_id);
    let entry = idx
        .profiles
        .iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| anyhow::anyhow!("profile id not found in index: {wanted:?}"))?;
    let profile_path = PathBuf::from(&entry.path);
    load_web_ui_profile_file(store, &profile_path, Some(index_digest))
}

fn load_web_ui_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_digest: Option<report::meta::FileDigest>,
) -> Result<LoadedWebUiProfile> {
    let digest = util::file_digest(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-web_ui.profile.schema.json",
        &doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid web-ui profile: {}", digest.path);
    }
    let doc: WebUiProfileDoc = serde_json::from_value(doc_json)
        .with_context(|| format!("parse web-ui profile doc: {}", path.display()))?;
    Ok(LoadedWebUiProfile {
        digest,
        doc,
        index_digest,
    })
}

fn project_name_for_manifest(project: &Path) -> String {
    let dir = project.parent().unwrap_or_else(|| Path::new("."));
    dir.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "app".to_string())
}

fn emit_host_assets(dist_dir: &Path) -> Result<(Value, Vec<report::meta::FileDigest>)> {
    let snapshot_doc: Value =
        serde_json::from_slice(HOST_SNAPSHOT_JSON).context("parse embedded host snapshot JSON")?;
    let snapshot: VendoredSnapshotDoc =
        serde_json::from_value(snapshot_doc).context("parse vendored snapshot doc")?;

    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    let dst_index = dist_dir.join(HOST_INDEX_HTML);
    std::fs::write(&dst_index, HOST_INDEX_HTML_BYTES)
        .with_context(|| format!("write host asset {}", dst_index.display()))?;
    artifacts.push(util::file_digest(&dst_index)?);

    let dst_bootstrap = dist_dir.join(HOST_BOOTSTRAP_JS);
    std::fs::write(&dst_bootstrap, HOST_BOOTSTRAP_JS_BYTES)
        .with_context(|| format!("write host asset {}", dst_bootstrap.display()))?;
    artifacts.push(util::file_digest(&dst_bootstrap)?);

    let dst_host = dist_dir.join(HOST_APP_HOST_MJS);
    std::fs::write(&dst_host, HOST_APP_HOST_MJS_BYTES)
        .with_context(|| format!("write host asset {}", dst_host.display()))?;
    artifacts.push(util::file_digest(&dst_host)?);

    let dst_main = dist_dir.join(HOST_MAIN_MJS);
    std::fs::write(&dst_main, HOST_MAIN_MJS_BYTES)
        .with_context(|| format!("write host asset {}", dst_main.display()))?;
    artifacts.push(util::file_digest(&dst_main)?);

    let files = artifacts.clone();
    let host_snapshot = json!({
      "source": snapshot.source,
      "git_sha": snapshot.git_sha,
      "files": files,
    });
    Ok((host_snapshot, artifacts))
}

fn build_web_ui_adapter_component(
    out_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let bytes = if adapters::adapters_from_source_enabled() {
        let manifest_path = Path::new(WEB_UI_ADAPTER_MANIFEST);
        let built_path = Path::new(WEB_UI_ADAPTER_COMPONENT_WASM);
        let Some(bytes) = adapters::build_wasm32_wasip2_release_bytes(
            manifest_path,
            built_path,
            diagnostics,
            "cargo build (web-ui-adapter)",
        ) else {
            return Ok(());
        };
        std::borrow::Cow::Owned(bytes)
    } else {
        std::borrow::Cow::Borrowed(adapters::EMBEDDED_WEB_UI_ADAPTER_COMPONENT_WASM)
    };

    if let Err(err) = adapters::write_bytes(out_path, bytes.as_ref()) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_ADAPTER_COMPONENT_COPY_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to write adapter component {}: {err:#}",
                out_path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("x07-wasm-web-ui-build-{tag}-{}-{n}", std::process::id());
        std::env::temp_dir().join(name)
    }

    #[test]
    fn emit_host_assets_uses_embedded_assets_outside_repo_root() {
        let tmp = tmp_dir("embedded_host_assets");
        let dist = tmp.join("dist");
        std::fs::create_dir_all(&dist).expect("create dist");

        let (host_snapshot, artifacts) = emit_host_assets(&dist).expect("emit_host_assets");

        assert_eq!(artifacts.len(), 4, "expected four emitted host assets");
        assert!(dist.join(HOST_INDEX_HTML).is_file(), "missing index.html");
        assert!(
            dist.join(HOST_BOOTSTRAP_JS).is_file(),
            "missing bootstrap.js"
        );
        assert!(
            dist.join(HOST_APP_HOST_MJS).is_file(),
            "missing app-host.mjs"
        );
        assert!(dist.join(HOST_MAIN_MJS).is_file(), "missing main.mjs");
        let source = host_snapshot.get("source").and_then(Value::as_str);
        assert!(
            matches!(source, Some(value) if !value.trim().is_empty()),
            "embedded host snapshot should preserve a non-empty source"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }
}

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::app::backend::AppBackendAdapter;
use crate::cli::{
    AppBuildArgs, AppBuildEmit, ComponentBuildArgs, MachineArgs, Scope, WebUiBuildArgs,
    WebUiBuildFormat,
};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::web_ui::runtime_manifest::load_runtime_manifest_from_profile;

pub fn cmd_app_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppBuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let loaded_profile = match load_app_profile(&store, &args, &mut meta, &mut diagnostics) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            None
        }
    };

    let out_dir = args.out_dir.clone();
    if args.clean {
        if let Err(err) = std::fs::remove_dir_all(&out_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_BUILD_CLEAN_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to clean out dir {}: {err}", out_dir.display()),
                ));
            }
        }
    }
    if let Err(err) = std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create dir: {}", out_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BUILD_OUTDIR_CREATE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
    }

    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    if let Some(profile) = loaded_profile.as_ref() {
        if diagnostics.iter().all(|d| d.severity != Severity::Error) {
            if matches!(args.emit, AppBuildEmit::All | AppBuildEmit::Frontend) {
                let frontend_dir = out_dir.join(&profile.frontend.out_dir_rel);
                if let Err(err) = build_frontend(
                    &store,
                    profile,
                    &frontend_dir,
                    &mut meta,
                    &mut diagnostics,
                    args.strict,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_BUILD_FRONTEND_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }

                if let Err(err) = write_frontend_app_manifest(&frontend_dir, profile) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_BUILD_APP_MANIFEST_WRITE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                } else if let Ok(d) =
                    file_digest_rel(&out_dir, &frontend_dir.join("app.manifest.json"))
                {
                    meta.outputs.push(d.clone());
                    artifacts.push(d);
                }
            }

            if matches!(args.emit, AppBuildEmit::All | AppBuildEmit::Backend) {
                if let Err(err) = build_backend(
                    &store,
                    profile,
                    &out_dir,
                    &mut meta,
                    &mut diagnostics,
                    args.strict,
                ) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_BUILD_BACKEND_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    ));
                }
            }
        }
    }

    let mut bundle_digest = report::meta::FileDigest {
        path: out_dir.join("app.bundle.json").display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    if let Some(profile) = loaded_profile.as_ref() {
        if matches!(
            args.emit,
            AppBuildEmit::All
                | AppBuildEmit::Frontend
                | AppBuildEmit::Backend
                | AppBuildEmit::Bundle
        ) {
            match write_app_bundle_manifest(&store, &out_dir, profile, &mut artifacts) {
                Ok(d) => {
                    bundle_digest = d.clone();
                    meta.outputs.push(d);
                }
                Err(err) => diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_BUILD_BUNDLE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                )),
            }
        }
    } else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_PROFILE_NOT_AVAILABLE",
            Severity::Error,
            Stage::Parse,
            "app profile unavailable".to_string(),
        ));
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    if artifacts.is_empty() {
        artifacts.push(bundle_digest.clone());
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let stdout_json = json!({
      "profile_id": loaded_profile.as_ref().map(|p| p.id.clone()).unwrap_or_else(|| args.profile.clone()),
      "out_dir": out_dir.display().to_string(),
      "bundle_manifest": bundle_digest,
      "artifacts": artifacts,
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.app.build.report@0.1.0",
      "command": "x07-wasm.app.build",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "stdout": { "bytes_len": 0 },
        "stderr": { "bytes_len": 0 },
        "stdout_json": stdout_json
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

struct LoadedAppProfileForBuild {
    id: String,
    frontend: AppProfileFrontendForBuild,
    backend: AppProfileBackendForBuild,
    routing_api_prefix: String,
}

struct AppProfileFrontendForBuild {
    format: String,
    project: PathBuf,
    web_ui_profile_id: String,
    out_dir_rel: PathBuf,
}

struct AppProfileBackendForBuild {
    adapter: AppBackendAdapter,
    project: PathBuf,
    component_profile_id: String,
    out_rel: PathBuf,
}

fn load_app_profile(
    store: &SchemaStore,
    args: &AppBuildArgs,
    meta: &mut report::meta::ReportMeta,
    _diagnostics: &mut Vec<Diagnostic>,
) -> Result<LoadedAppProfileForBuild> {
    let loaded = crate::app::load::load_app_profile(
        store,
        &args.index,
        Some(args.profile.as_str()),
        args.profile_file.as_ref(),
    )?;
    meta.inputs.push(loaded.digest.clone());
    if let Some(d) = loaded.index_digest.as_ref() {
        meta.inputs.push(d.clone());
    }

    Ok(LoadedAppProfileForBuild {
        id: loaded.doc.id,
        frontend: AppProfileFrontendForBuild {
            format: loaded.doc.frontend.format,
            project: PathBuf::from(loaded.doc.frontend.project),
            web_ui_profile_id: loaded.doc.frontend.web_ui_profile_id,
            out_dir_rel: PathBuf::from(loaded.doc.frontend.out_dir_rel),
        },
        backend: AppProfileBackendForBuild {
            adapter: loaded.doc.backend.adapter,
            project: PathBuf::from(loaded.doc.backend.project),
            component_profile_id: loaded.doc.backend.component_profile_id,
            out_rel: PathBuf::from(loaded.doc.backend.out_rel),
        },
        routing_api_prefix: loaded.doc.routing.api_prefix,
    })
}

fn build_frontend(
    _store: &SchemaStore,
    profile: &LoadedAppProfileForBuild,
    frontend_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    strict: bool,
) -> Result<()> {
    if let Ok(d) = util::file_digest(&profile.frontend.project) {
        meta.inputs.push(d);
    }

    let format = match profile.frontend.format.as_str() {
        "core_wasm_v1" => WebUiBuildFormat::Core,
        "component_jco_v1" => WebUiBuildFormat::Component,
        other => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_FRONTEND_FORMAT_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("unsupported frontend.format: {other:?}"),
            ));
            WebUiBuildFormat::Core
        }
    };

    let nested_report_out = PathBuf::from("target")
        .join("x07-wasm")
        .join("app")
        .join(&profile.id)
        .join("frontend")
        .join("web-ui.build.report.json");
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

    let web_ui_args = WebUiBuildArgs {
        project: profile.frontend.project.clone(),
        profile: Some(profile.frontend.web_ui_profile_id.clone()),
        profile_file: None,
        index: PathBuf::from("arch/web_ui/index.x07webui.json"),
        wasm_index: PathBuf::from("arch/wasm/index.x07wasm.json"),
        format: Some(format),
        out_dir: frontend_dir.to_path_buf(),
        clean: true,
        strict,
    };
    let code = crate::web_ui::build::cmd_web_ui_build(
        &[],
        Scope::WebUiBuild,
        &nested_machine,
        web_ui_args,
    )?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_APP_WEB_UI_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm web-ui build failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(nested_report_out.display().to_string()),
        );
        diagnostics.push(d);
    }
    Ok(())
}

fn build_backend(
    _store: &SchemaStore,
    profile: &LoadedAppProfileForBuild,
    out_dir: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
    _strict: bool,
) -> Result<()> {
    if let Ok(d) = util::file_digest(&profile.backend.project) {
        meta.inputs.push(d);
    }

    let emit_dir = PathBuf::from("target")
        .join("x07-wasm")
        .join("app")
        .join(&profile.id)
        .join("backend");
    std::fs::create_dir_all(&emit_dir).ok();

    let component_out_dir = emit_dir.join("component");
    let component_build_report = emit_dir.join("component.build.report.json");

    let nested_machine_build = MachineArgs {
        json: Some("".to_string()),
        report_json: None,
        report_out: Some(component_build_report.clone()),
        quiet_json: true,
        json_schema: false,
        json_schema_id: false,
    };
    let build_args = ComponentBuildArgs {
        project: profile.backend.project.clone(),
        profile: Some(profile.backend.component_profile_id.clone()),
        profile_file: None,
        index: PathBuf::from("arch/wasm/component/index.x07wasm.component.json"),
        wasm_profile: None,
        wasm_profile_file: None,
        wasm_index: PathBuf::from("arch/wasm/index.x07wasm.json"),
        out_dir: component_out_dir.clone(),
        emit: profile.backend.adapter.component_emit(),
        clean: true,
    };

    let code = crate::component::build::cmd_component_build(
        &[],
        Scope::ComponentBuild,
        &nested_machine_build,
        build_args,
    )?;
    if code != 0 {
        let mut d = Diagnostic::new(
            "X07WASM_APP_COMPONENT_BUILD_FAILED",
            Severity::Error,
            Stage::Run,
            format!("x07-wasm component build failed (exit_code={code})"),
        );
        d.data.insert(
            "report_out".to_string(),
            json!(component_build_report.display().to_string()),
        );
        diagnostics.push(d);
        return Ok(());
    }

    let out_component = out_dir.join(&profile.backend.out_rel);
    let out_manifest = out_dir.join("backend").join("backend.manifest.json");

    if let Some(parent) = out_component.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(out_manifest.parent().unwrap_or(out_dir));

    let built_component = component_out_dir.join("http.component.wasm");
    let built_manifest = component_out_dir.join("http.component.wasm.manifest.json");

    if let Err(err) = std::fs::copy(&built_component, &out_component) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BACKEND_COMPONENT_COPY_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to copy backend component {} -> {}: {err:#}",
                built_component.display(),
                out_component.display()
            ),
        ));
        return Ok(());
    }

    if let Err(err) = std::fs::copy(&built_manifest, &out_manifest) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_BACKEND_MANIFEST_COPY_FAILED",
            Severity::Error,
            Stage::Run,
            format!(
                "failed to copy backend manifest {} -> {}: {err:#}",
                built_manifest.display(),
                out_manifest.display()
            ),
        ));
    }

    Ok(())
}

fn write_frontend_app_manifest(
    frontend_dir: &Path,
    profile: &LoadedAppProfileForBuild,
) -> Result<()> {
    let wasm_url = "app.wasm".to_string();
    let runtime = load_runtime_manifest_from_profile(&frontend_dir.join("web-ui.profile.json"))?;
    let component_esm_url = frontend_dir.join("transpiled").join("app.mjs");
    let doc = if component_esm_url.is_file() {
        json!({
          "wasmUrl": wasm_url,
          "componentEsmUrl": "transpiled/app.mjs",
          "apiPrefix": profile.routing_api_prefix,
          "webUi": runtime,
        })
    } else {
        json!({
          "wasmUrl": wasm_url,
          "apiPrefix": profile.routing_api_prefix,
          "webUi": runtime,
        })
    };
    let bytes = report::canon::canonical_pretty_json_bytes(&doc)?;
    std::fs::create_dir_all(frontend_dir)
        .with_context(|| format!("create dir: {}", frontend_dir.display()))?;
    std::fs::write(frontend_dir.join("app.manifest.json"), bytes).with_context(|| {
        format!(
            "write: {}",
            frontend_dir.join("app.manifest.json").display()
        )
    })?;
    Ok(())
}

fn write_app_bundle_manifest(
    store: &SchemaStore,
    out_dir: &Path,
    profile: &LoadedAppProfileForBuild,
    artifacts: &mut Vec<report::meta::FileDigest>,
) -> Result<report::meta::FileDigest> {
    let frontend_dir = out_dir.join(&profile.frontend.out_dir_rel);

    let mut frontend_artifacts = collect_file_digests_rel(out_dir, &frontend_dir)?;
    frontend_artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    if frontend_artifacts.len() > 256 {
        anyhow::bail!(
            "too many frontend artifacts ({}), max is 256",
            frontend_artifacts.len()
        );
    }

    let backend_artifact_path = out_dir.join(&profile.backend.out_rel);
    let backend_artifact = file_digest_rel(out_dir, &backend_artifact_path)?;

    let frontend_format = profile.frontend.format.as_str();
    let frontend_format = match frontend_format {
        "core_wasm_v1" => "core_wasm_v1",
        "component_jco_v1" => "component_jco_v1",
        _ => "core_wasm_v1",
    };

    let bundle_doc = json!({
      "schema_version": "x07.app.bundle@0.1.0",
      "profile_id": profile.id,
      "frontend": {
        "format": frontend_format,
        "dir_rel": profile.frontend.out_dir_rel.to_string_lossy().to_string(),
        "artifacts": frontend_artifacts,
      },
      "backend": {
        "adapter": profile.backend.adapter.as_str(),
        "artifact": backend_artifact,
      }
    });

    let diags = store.validate(
        "https://x07.io/spec/x07-app.bundle.schema.json",
        &bundle_doc,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("internal error: bundle manifest failed schema validation: {diags:?}");
    }

    let out_path = out_dir.join("app.bundle.json");
    let bytes = report::canon::canonical_pretty_json_bytes(&bundle_doc)?;
    std::fs::write(&out_path, bytes).with_context(|| format!("write: {}", out_path.display()))?;
    let digest = util::file_digest(&out_path)?;

    artifacts.push(digest.clone());
    artifacts.extend(frontend_artifacts);
    artifacts.push(backend_artifact);

    Ok(digest)
}

fn collect_file_digests_rel(root: &Path, dir: &Path) -> Result<Vec<report::meta::FileDigest>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files_recursively(dir, &mut paths)?;
    paths.sort();

    let mut out = Vec::new();
    for p in paths {
        out.push(file_digest_rel(root, &p)?);
    }
    Ok(out)
}

fn file_digest_rel(root: &Path, path: &Path) -> Result<report::meta::FileDigest> {
    let mut d = util::file_digest(path)?;
    if let Ok(rel) = path.strip_prefix(root) {
        d.path = rel.to_string_lossy().to_string();
    }
    Ok(d)
}

fn collect_files_recursively(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir: {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read_dir entry: {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type: {}", path.display()))?;
        if ft.is_dir() {
            collect_files_recursively(&path, out)?;
        } else if ft.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static CWD_LOCK: Mutex<()> = Mutex::new(());
    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("x07-wasm-app-build-{tag}-{}-{n}", std::process::id());
        std::env::temp_dir().join(name)
    }

    struct CwdGuard {
        prev: PathBuf,
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.prev);
        }
    }

    fn enter_tmp_cwd(tag: &str) -> (PathBuf, CwdGuard) {
        let prev = std::env::current_dir().expect("current_dir");
        let dir = tmp_dir(tag);
        std::fs::create_dir_all(&dir).expect("create_dir_all tmp");
        std::env::set_current_dir(&dir).expect("set_current_dir tmp");
        (dir, CwdGuard { prev })
    }

    #[test]
    fn app_build_web_ui_failure_includes_nested_report_out() {
        let _guard = CWD_LOCK.lock().unwrap();
        let (tmp, cwd) = enter_tmp_cwd("web_ui_report_out");

        let store = SchemaStore::new().expect("SchemaStore::new");
        let mut meta = report::meta::tool_meta(&[], std::time::Instant::now());
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let profile = LoadedAppProfileForBuild {
            id: "test_profile".to_string(),
            frontend: AppProfileFrontendForBuild {
                format: "core_wasm_v1".to_string(),
                project: PathBuf::from("frontend/x07.json"),
                web_ui_profile_id: "web_ui_release".to_string(),
                out_dir_rel: PathBuf::from("frontend"),
            },
            backend: AppProfileBackendForBuild {
                adapter: AppBackendAdapter::WasiHttpProxyV1,
                project: PathBuf::from("backend/x07.json"),
                component_profile_id: "component_release".to_string(),
                out_rel: PathBuf::from("backend/app.http.component.wasm"),
            },
            routing_api_prefix: "/api".to_string(),
        };

        build_frontend(
            &store,
            &profile,
            Path::new("dist/frontend"),
            &mut meta,
            &mut diagnostics,
            false,
        )
        .expect("build_frontend");

        let d = diagnostics
            .iter()
            .find(|d| d.code == "X07WASM_APP_WEB_UI_BUILD_FAILED")
            .expect("expected X07WASM_APP_WEB_UI_BUILD_FAILED diagnostic");
        let expected = PathBuf::from("target")
            .join("x07-wasm")
            .join("app")
            .join(&profile.id)
            .join("frontend")
            .join("web-ui.build.report.json")
            .display()
            .to_string();
        assert_eq!(
            d.data.get("report_out").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );

        drop(cwd);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn app_build_component_failure_includes_nested_report_out() {
        let _guard = CWD_LOCK.lock().unwrap();
        let (tmp, cwd) = enter_tmp_cwd("component_report_out");

        let store = SchemaStore::new().expect("SchemaStore::new");
        let mut meta = report::meta::tool_meta(&[], std::time::Instant::now());
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let profile = LoadedAppProfileForBuild {
            id: "test_profile".to_string(),
            frontend: AppProfileFrontendForBuild {
                format: "core_wasm_v1".to_string(),
                project: PathBuf::from("frontend/x07.json"),
                web_ui_profile_id: "web_ui_release".to_string(),
                out_dir_rel: PathBuf::from("frontend"),
            },
            backend: AppProfileBackendForBuild {
                adapter: AppBackendAdapter::WasiHttpProxyV1,
                project: PathBuf::from("backend/x07.json"),
                component_profile_id: "component_release".to_string(),
                out_rel: PathBuf::from("backend/app.http.component.wasm"),
            },
            routing_api_prefix: "/api".to_string(),
        };

        build_backend(
            &store,
            &profile,
            Path::new("dist"),
            &mut meta,
            &mut diagnostics,
            false,
        )
        .expect("build_backend");

        let d = diagnostics
            .iter()
            .find(|d| d.code == "X07WASM_APP_COMPONENT_BUILD_FAILED")
            .expect("expected X07WASM_APP_COMPONENT_BUILD_FAILED diagnostic");
        let expected = PathBuf::from("target")
            .join("x07-wasm")
            .join("app")
            .join(&profile.id)
            .join("backend")
            .join("component.build.report.json")
            .display()
            .to_string();
        assert_eq!(
            d.data.get("report_out").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );

        drop(cwd);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

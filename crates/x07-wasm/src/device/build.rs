use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{DeviceBuildArgs, MachineArgs, Scope, WebUiBuildArgs};
use crate::device::contracts::{DeviceIndexDoc, DeviceProfileDoc};
use crate::device::host_abi;
use crate::device::sidecars::load_profile_sidecars;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::web_ui::runtime_manifest::{load_runtime_manifest_from_profile, WebUiRuntimeManifest};

const DEFAULT_DEVICE_PROFILE_ID: &str = "device_dev";
const DEVICE_BUNDLE_MANIFEST_FILE: &str = "bundle.manifest.json";

#[derive(Debug, Clone)]
struct LoadedDeviceProfile {
    digest: report::meta::FileDigest,
    doc: DeviceProfileDoc,
    index_digest: Option<report::meta::FileDigest>,
    path: PathBuf,
}

pub fn cmd_device_build(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DeviceBuildArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let loaded_profile = match load_device_profile(&store, &args) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_LOAD_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            None
        }
    };

    if let Some(p) = loaded_profile.as_ref() {
        meta.inputs.push(p.digest.clone());
        if let Some(d) = p.index_digest.clone() {
            meta.inputs.push(d);
        }
        if let Ok(d) = util::file_digest(&p.doc.ui.project) {
            meta.inputs.push(d);
        }
    }

    let out_dir = args.out_dir.clone();
    if args.clean {
        if let Err(err) = std::fs::remove_dir_all(&out_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_CLEAN_FAILED",
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
            "X07WASM_DEVICE_BUILD_OUTDIR_CREATE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
    }

    let mut artifacts: Vec<report::meta::FileDigest> = Vec::new();

    let mut bundle_manifest_digest = report::meta::FileDigest {
        path: out_dir
            .join(DEVICE_BUNDLE_MANIFEST_FILE)
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let mut ui_wasm_digest = report::meta::FileDigest {
        path: out_dir.join("ui/reducer.wasm").display().to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let mut profile_file_digest = report::meta::FileDigest {
        path: out_dir
            .join("profile")
            .join("device.profile.json")
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let mut capabilities_file_digest = report::meta::FileDigest {
        path: out_dir
            .join("profile")
            .join("device.capabilities.json")
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };
    let mut telemetry_profile_file_digest = report::meta::FileDigest {
        path: out_dir
            .join("profile")
            .join("device.telemetry.profile.json")
            .display()
            .to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    if let Some(profile) = loaded_profile.as_ref() {
        let loaded_sidecars =
            load_profile_sidecars(&store, &profile.doc, &mut meta, &mut diagnostics);

        if diagnostics.iter().all(|d| d.severity != Severity::Error) {
            let target = profile.doc.target.as_str();
            let target_ok = matches!(target, "desktop" | "ios" | "android");
            if !target_ok {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PROFILE_TARGET_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("unsupported device profile target: {target:?}"),
                ));
            }

            let internal_dist_dir = PathBuf::from("target")
                .join("x07-wasm")
                .join("device")
                .join(&profile.doc.id)
                .join("web_ui_dist");

            let nested_report_out = PathBuf::from("target")
                .join("x07-wasm")
                .join("device")
                .join(&profile.doc.id)
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
                project: profile.doc.ui.project.clone(),
                profile: Some(profile.doc.ui.web_ui_profile_id.clone()),
                profile_file: None,
                index: PathBuf::from("arch/web_ui/index.x07webui.json"),
                wasm_index: PathBuf::from("arch/wasm/index.x07wasm.json"),
                format: None,
                out_dir: internal_dist_dir.clone(),
                clean: true,
                strict: args.strict,
            };

            let code = crate::web_ui::build::cmd_web_ui_build(
                &[],
                Scope::WebUiBuild,
                &nested_machine,
                web_ui_args,
            )?;
            if code != 0 {
                let mut d = Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_WEB_UI_BUILD_FAILED",
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

            let src_wasm = internal_dist_dir.join("app.wasm");
            let src_web_ui_profile = internal_dist_dir.join("web-ui.profile.json");
            let src_transpiled_dir = internal_dist_dir.join("transpiled");
            if !src_wasm.is_file() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_WEB_UI_WASM_MISSING",
                    Severity::Error,
                    Stage::Run,
                    format!("missing web-ui wasm output: {}", src_wasm.display()),
                ));
            }

            let dst_dir = out_dir.join("ui");
            if let Err(err) = std::fs::create_dir_all(&dst_dir)
                .with_context(|| format!("create dir: {}", dst_dir.display()))
            {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_OUTDIR_CREATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
            }

            let dst_wasm = dst_dir.join("reducer.wasm");
            if let Err(err) = std::fs::copy(&src_wasm, &dst_wasm) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_COPY_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "failed to copy reducer wasm {} -> {}: {err}",
                        src_wasm.display(),
                        dst_wasm.display()
                    ),
                ));
            }

            if dst_wasm.is_file() {
                match file_digest_rel(&out_dir, &dst_wasm) {
                    Ok(d) => {
                        ui_wasm_digest = d.clone();
                        meta.outputs.push(d.clone());
                        artifacts.push(d);
                    }
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to digest reducer wasm {}: {err:#}",
                            dst_wasm.display()
                        ),
                    )),
                }
            }

            if diagnostics.iter().all(|d| d.severity != Severity::Error) {
                if src_transpiled_dir.is_dir() {
                    let dst_transpiled_dir = out_dir.join("transpiled");
                    match copy_dir_tree(&src_transpiled_dir, &dst_transpiled_dir) {
                        Ok(digests) => {
                            for digest in digests {
                                meta.outputs.push(digest.clone());
                                artifacts.push(digest);
                            }
                        }
                        Err(err) => diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_BUILD_COPY_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!(
                                "failed to copy transpiled dir {} -> {}: {err:#}",
                                src_transpiled_dir.display(),
                                dst_transpiled_dir.display()
                            ),
                        )),
                    }
                }
            }

            if diagnostics.iter().all(|d| d.severity != Severity::Error) {
                let has_component_esm = src_transpiled_dir.join("app.mjs").is_file();
                match load_runtime_manifest_from_profile(&src_web_ui_profile)
                    .and_then(|runtime| {
                        write_device_app_manifest(&out_dir, &runtime, has_component_esm)
                    })
                {
                    Ok(()) => match file_digest_rel(&out_dir, &out_dir.join("app.manifest.json")) {
                        Ok(d) => {
                            meta.outputs.push(d.clone());
                            artifacts.push(d);
                        }
                        Err(err) => diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!(
                                "failed to digest device app manifest {}: {err:#}",
                                out_dir.join("app.manifest.json").display()
                            ),
                        )),
                    },
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUILD_APP_MANIFEST_WRITE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!("{err:#}"),
                    )),
                }
            }

            // Copy the resolved device profile into the bundle so it is self-contained.
            let dst_profile_dir = out_dir.join("profile");
            if let Err(err) = std::fs::create_dir_all(&dst_profile_dir)
                .with_context(|| format!("create dir: {}", dst_profile_dir.display()))
            {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_OUTDIR_CREATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
            }

            let dst_profile = dst_profile_dir.join("device.profile.json");
            if let Err(err) = std::fs::copy(&profile.path, &dst_profile) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_BUILD_COPY_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "failed to copy device profile {} -> {}: {err}",
                        profile.path.display(),
                        dst_profile.display()
                    ),
                ));
            }

            if dst_profile.is_file() {
                match file_digest_rel(&out_dir, &dst_profile) {
                    Ok(d) => {
                        profile_file_digest = d.clone();
                        meta.outputs.push(d.clone());
                        artifacts.push(d);
                    }
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to digest device profile copy {}: {err:#}",
                            dst_profile.display()
                        ),
                    )),
                }
            }

            if let Some(sidecars) = loaded_sidecars.as_ref() {
                let dst_capabilities = dst_profile_dir.join("device.capabilities.json");
                if let Err(err) = std::fs::copy(&sidecars.capabilities.path, &dst_capabilities) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUILD_COPY_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to copy device capabilities {} -> {}: {err}",
                            sidecars.capabilities.path.display(),
                            dst_capabilities.display()
                        ),
                    ));
                } else if dst_capabilities.is_file() {
                    match file_digest_rel(&out_dir, &dst_capabilities) {
                        Ok(d) => {
                            capabilities_file_digest = d.clone();
                            meta.outputs.push(d.clone());
                            artifacts.push(d);
                        }
                        Err(err) => diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!(
                                "failed to digest device capabilities copy {}: {err:#}",
                                dst_capabilities.display()
                            ),
                        )),
                    }
                }

                let dst_telemetry_profile = dst_profile_dir.join("device.telemetry.profile.json");
                if let Err(err) =
                    std::fs::copy(&sidecars.telemetry_profile.path, &dst_telemetry_profile)
                {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_BUILD_COPY_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to copy device telemetry profile {} -> {}: {err}",
                            sidecars.telemetry_profile.path.display(),
                            dst_telemetry_profile.display()
                        ),
                    ));
                } else if dst_telemetry_profile.is_file() {
                    match file_digest_rel(&out_dir, &dst_telemetry_profile) {
                        Ok(d) => {
                            telemetry_profile_file_digest = d.clone();
                            meta.outputs.push(d.clone());
                            artifacts.push(d);
                        }
                        Err(err) => diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!(
                                "failed to digest device telemetry profile copy {}: {err:#}",
                                dst_telemetry_profile.display()
                            ),
                        )),
                    }
                }
            }

            let host_abi_hash = host_abi::HOST_ABI_HASH_HEX;
            if host_abi_hash.len() != 64 {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_HOST_ABI_HASH_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    "host abi hash is invalid".to_string(),
                ));
            }

            if diagnostics.iter().all(|d| d.severity != Severity::Error) {
                let mut bundle_doc = json!({
                  "schema_version": "x07.device.bundle.manifest@0.1.0",
                  "kind": "device_bundle",
                  "target": profile.doc.target,
                  "profile": {
                    "id": profile.doc.id,
                    "v": profile.doc.v,
                    "file": profile_file_digest,
                  },
                  "capabilities": capabilities_file_digest,
                  "telemetry_profile": telemetry_profile_file_digest,
                  "ui_wasm": ui_wasm_digest,
                  "host": {
                    "kind": host_abi::HOST_KIND,
                    "abi_name": host_abi::ABI_NAME,
                    "abi_version": host_abi::ABI_VERSION,
                    "host_abi_hash": host_abi_hash,
                  },
                  "bundle_digest": "0".repeat(64),
                });

                let tmp_bytes = report::canon::canonical_json_bytes(&bundle_doc)?;
                let bundle_digest = util::sha256_hex(&tmp_bytes);
                if let Some(obj) = bundle_doc.as_object_mut() {
                    obj.insert("bundle_digest".to_string(), json!(bundle_digest));
                }

                let diags = store.validate(
                    "https://x07.io/spec/x07-device.bundle.manifest.schema.json",
                    &bundle_doc,
                )?;
                if diags.iter().any(|d| d.severity == Severity::Error) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_INTERNAL_DEVICE_BUNDLE_SCHEMA_INVALID",
                        Severity::Error,
                        Stage::Run,
                        format!("internal error: device bundle manifest schema invalid: {diags:?}"),
                    ));
                } else {
                    let out_path = out_dir.join(DEVICE_BUNDLE_MANIFEST_FILE);
                    let bytes = report::canon::canonical_pretty_json_bytes(&bundle_doc)?;
                    if let Err(err) = std::fs::write(&out_path, bytes)
                        .with_context(|| format!("write: {}", out_path.display()))
                    {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_BUNDLE_MANIFEST_WRITE_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("{err:#}"),
                        ));
                    } else {
                        match file_digest_rel(&out_dir, &out_path) {
                            Ok(d) => {
                                bundle_manifest_digest = d.clone();
                                meta.outputs.push(d.clone());
                                artifacts.push(d);
                            }
                            Err(err) => diagnostics.push(Diagnostic::new(
                                "X07WASM_DEVICE_BUILD_DIGEST_FAILED",
                                Severity::Error,
                                Stage::Run,
                                format!(
                                    "failed to digest bundle manifest {}: {err:#}",
                                    out_path.display()
                                ),
                            )),
                        }
                    }
                }
            }
        }
    } else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PROFILE_NOT_AVAILABLE",
            Severity::Error,
            Stage::Parse,
            "device profile unavailable".to_string(),
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
        artifacts.push(bundle_manifest_digest.clone());
        artifacts.push(ui_wasm_digest.clone());
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let profile_ref = loaded_profile.as_ref().map_or_else(
        || {
            let fallback_id = if args.profile_file.is_some() && args.profile.is_none() {
                "unknown".to_string()
            } else {
                args.profile
                    .clone()
                    .unwrap_or_else(|| DEFAULT_DEVICE_PROFILE_ID.to_string())
            };
            json!({
              "id": fallback_id,
              "v": 1,
            })
        },
        |p| json!({ "id": p.doc.id, "v": p.doc.v }),
    );
    let target = loaded_profile
        .as_ref()
        .map(|p| p.doc.target.clone())
        .unwrap_or_else(|| "desktop".to_string());

    let report_doc = json!({
      "schema_version": "x07.wasm.device.build.report@0.1.0",
      "command": "x07-wasm.device.build",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "profile": profile_ref,
        "target": target,
        "out_dir": out_dir.display().to_string(),
        "bundle_manifest": bundle_manifest_digest,
        "ui_wasm": ui_wasm_digest,
        "host": {
          "kind": host_abi::HOST_KIND,
          "abi_name": host_abi::ABI_NAME,
          "abi_version": host_abi::ABI_VERSION,
          "host_abi_hash": host_abi::HOST_ABI_HASH_HEX,
        },
        "artifacts": artifacts,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn load_device_profile(store: &SchemaStore, args: &DeviceBuildArgs) -> Result<LoadedDeviceProfile> {
    if let Some(path) = args.profile_file.as_ref() {
        return load_device_profile_file(store, path, None);
    }

    let index_digest = util::file_digest(&args.index)?;
    let index_bytes =
        std::fs::read(&args.index).with_context(|| format!("read: {}", args.index.display()))?;
    let index_doc_json: Value = serde_json::from_slice(&index_bytes)
        .with_context(|| format!("parse JSON: {}", args.index.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-arch.device.index.schema.json",
        &index_doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid device index: {}", index_digest.path);
    }

    let idx: DeviceIndexDoc = serde_json::from_value(index_doc_json)
        .with_context(|| format!("parse index doc: {}", args.index.display()))?;
    let default_id = idx
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
        .unwrap_or_else(|| DEFAULT_DEVICE_PROFILE_ID.to_string());
    let wanted = args.profile.as_deref().unwrap_or(&default_id);
    let entry = idx
        .profiles
        .iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| anyhow::anyhow!("profile id not found in device index: {wanted:?}"))?;
    let profile_path = PathBuf::from(&entry.path);
    load_device_profile_file(store, &profile_path, Some(index_digest))
}

fn load_device_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_digest: Option<report::meta::FileDigest>,
) -> Result<LoadedDeviceProfile> {
    let digest = util::file_digest(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate(
        "https://x07.io/spec/x07-device.profile.schema.json",
        &doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid device profile: {}", digest.path);
    }
    let doc: DeviceProfileDoc = serde_json::from_value(doc_json)
        .with_context(|| format!("parse doc: {}", path.display()))?;
    Ok(LoadedDeviceProfile {
        digest,
        doc,
        index_digest,
        path: path.clone(),
    })
}

fn write_device_app_manifest(
    out_dir: &Path,
    runtime: &WebUiRuntimeManifest,
    has_component_esm: bool,
) -> Result<()> {
    let mut doc = json!({
      "wasmUrl": "./ui/reducer.wasm",
      "webUi": runtime,
    });
    if has_component_esm {
        doc["componentEsmUrl"] = json!("./transpiled/app.mjs");
    }
    let bytes = report::canon::canonical_pretty_json_bytes(&doc)?;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("create dir: {}", out_dir.display()))?;
    std::fs::write(out_dir.join("app.manifest.json"), bytes)
        .with_context(|| format!("write: {}", out_dir.join("app.manifest.json").display()))?;
    Ok(())
}

fn file_digest_rel(root: &Path, path: &Path) -> Result<report::meta::FileDigest> {
    let mut d = util::file_digest(path)?;
    if let Ok(rel) = path.strip_prefix(root) {
        d.path = rel.to_string_lossy().to_string();
    }
    Ok(d)
}

fn copy_dir_tree(src_dir: &Path, dst_dir: &Path) -> Result<Vec<report::meta::FileDigest>> {
    if dst_dir.exists() {
        std::fs::remove_dir_all(dst_dir)
            .with_context(|| format!("remove dir: {}", dst_dir.display()))?;
    }
    std::fs::create_dir_all(dst_dir)
        .with_context(|| format!("create dir: {}", dst_dir.display()))?;

    let mut digests = Vec::new();
    copy_dir_tree_inner(src_dir, src_dir, dst_dir, &mut digests)?;
    Ok(digests)
}

fn copy_dir_tree_inner(
    root_src: &Path,
    current_src: &Path,
    dst_dir: &Path,
    digests: &mut Vec<report::meta::FileDigest>,
) -> Result<()> {
    for entry in std::fs::read_dir(current_src)
        .with_context(|| format!("read dir: {}", current_src.display()))?
    {
        let entry = entry.with_context(|| format!("read dir entry: {}", current_src.display()))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root_src)
            .with_context(|| format!("strip prefix: {} from {}", root_src.display(), path.display()))?;
        let dst = dst_dir.join(rel);
        let file_type = entry
            .file_type()
            .with_context(|| format!("file type: {}", path.display()))?;
        if file_type.is_dir() {
            std::fs::create_dir_all(&dst)
                .with_context(|| format!("create dir: {}", dst.display()))?;
            copy_dir_tree_inner(root_src, &path, dst_dir, digests)?;
            continue;
        }
        std::fs::copy(&path, &dst)
            .with_context(|| format!("copy {} -> {}", path.display(), dst.display()))?;
        digests.push(file_digest_rel(dst_dir.parent().unwrap_or(dst_dir), &dst)?);
    }
    Ok(())
}

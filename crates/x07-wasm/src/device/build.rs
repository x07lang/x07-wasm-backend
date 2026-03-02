use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{DeviceBuildArgs, MachineArgs, Scope, WebUiBuildArgs, WebUiBuildFormat};
use crate::device::contracts::{DeviceIndexDoc, DeviceProfileDoc};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

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

    if let Some(profile) = loaded_profile.as_ref() {
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
                format: Some(WebUiBuildFormat::Core),
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

            let host_abi_hash = x07_device_host_abi::host_abi_hash_hex();
            if host_abi_hash.len() != 64 {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_HOST_ABI_HASH_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    "host abi hash is invalid".to_string(),
                ));
            }

            if diagnostics.iter().all(|d| d.severity != Severity::Error) {
                let profile_file = file_digest_for_manifest(&profile.path, profile.digest.clone());

                let mut bundle_doc = json!({
                  "schema_version": "x07.device.bundle.manifest@0.1.0",
                  "kind": "device_bundle",
                  "target": profile.doc.target,
                  "profile": {
                    "id": profile.doc.id,
                    "v": profile.doc.v,
                    "file": profile_file,
                  },
                  "ui_wasm": ui_wasm_digest,
                  "host": {
                    "kind": "webview_v1",
                    "abi_name": x07_device_host_abi::ABI_NAME,
                    "abi_version": x07_device_host_abi::ABI_VERSION,
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
          "kind": "webview_v1",
          "abi_name": x07_device_host_abi::ABI_NAME,
          "abi_version": x07_device_host_abi::ABI_VERSION,
          "host_abi_hash": x07_device_host_abi::host_abi_hash_hex(),
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

fn file_digest_for_manifest(path: &Path, digest: report::meta::FileDigest) -> Value {
    json!({
      "path": path.display().to_string(),
      "sha256": digest.sha256,
      "bytes_len": digest.bytes_len,
    })
}

fn file_digest_rel(root: &Path, path: &Path) -> Result<report::meta::FileDigest> {
    let mut d = util::file_digest(path)?;
    if let Ok(rel) = path.strip_prefix(root) {
        d.path = rel.to_string_lossy().to_string();
    }
    Ok(d)
}

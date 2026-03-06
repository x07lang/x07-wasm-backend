use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use zip::write::FileOptions;

use crate::cli::{DevicePackageArgs, MachineArgs, Scope};
use crate::device::contracts::{DeviceBundleManifestDoc, DeviceProfileDoc};
use crate::device::{package_android, package_ios};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEVICE_BUNDLE_MANIFEST_FILE: &str = "bundle.manifest.json";
const DEVICE_PACKAGE_MANIFEST_FILE: &str = "package.manifest.json";

pub fn cmd_device_package(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: DevicePackageArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = true;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let DevicePackageArgs {
        bundle: bundle_dir,
        target: target_arg,
        out_dir,
    } = args;

    let mut profile_ref = json!({ "id": "unknown", "v": 1 });
    let mut package_info = json!({ "kind": "dir", "path": "unknown" });
    let mut package_manifest_digest = report::meta::FileDigest {
        path: DEVICE_PACKAGE_MANIFEST_FILE.to_string(),
        sha256: "0".repeat(64),
        bytes_len: 0,
    };

    let target = target_arg.trim();
    let target_ok = matches!(target, "desktop" | "ios" | "android");
    if !target_ok {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_FAILED",
            Severity::Error,
            Stage::Parse,
            format!("unsupported device package target: {target:?}"),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            "desktop",
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }
    meta.nondeterminism.uses_process = target == "desktop";

    let bundle_manifest_path = bundle_dir.join(DEVICE_BUNDLE_MANIFEST_FILE);
    let bundle_manifest_sha256 = match util::file_digest(&bundle_manifest_path) {
        Ok(d) => {
            meta.inputs.push(d.clone());
            d.sha256
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read bundle manifest {}: {err:#}",
                    bundle_manifest_path.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let bundle_manifest_bytes = match std::fs::read(&bundle_manifest_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read bundle manifest {}: {err}",
                    bundle_manifest_path.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let bundle_manifest_json: Value = match serde_json::from_slice(&bundle_manifest_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("bundle manifest is not JSON: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-device.bundle.manifest.schema.json",
        &bundle_manifest_json,
    )?;
    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_BUNDLE_MANIFEST_SCHEMA_INVALID",
            Severity::Error,
            Stage::Parse,
            "device bundle manifest schema invalid".to_string(),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        diagnostics.push(d);
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let bundle_doc: DeviceBundleManifestDoc = match serde_json::from_value(bundle_manifest_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_MANIFEST_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse device bundle manifest doc: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    profile_ref = json!({ "id": bundle_doc.profile.id, "v": bundle_doc.profile.v });

    if bundle_doc.target != target {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_FAILED",
            Severity::Error,
            Stage::Parse,
            format!("bundle target mismatch: {:?}", bundle_doc.target),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let profile_rel = bundle_doc.profile.file.path.clone();
    let profile_path = match util::safe_join_under_dir(&bundle_dir, &profile_rel) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_PATH_UNSAFE",
                Severity::Error,
                Stage::Parse,
                format!("unsafe bundle path: {profile_rel:?} ({})", err.detail),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let profile_bytes = match std::fs::read(&profile_path) {
        Ok(v) => v,
        Err(err) => {
            let mut d = Diagnostic::new(
                "X07WASM_DEVICE_BUNDLE_FILE_MISSING",
                Severity::Error,
                Stage::Parse,
                format!("missing bundle file {}: {err}", profile_path.display()),
            );
            d.data.insert("path".to_string(), json!(profile_rel));
            diagnostics.push(d);
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let got_sha = util::sha256_hex(&profile_bytes);
    let got_len = profile_bytes.len() as u64;
    meta.inputs.push(report::meta::FileDigest {
        path: profile_path.display().to_string(),
        sha256: got_sha.clone(),
        bytes_len: got_len,
    });
    if got_sha != bundle_doc.profile.file.sha256 || got_len != bundle_doc.profile.file.bytes_len {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_BUNDLE_SHA256_MISMATCH",
            Severity::Error,
            Stage::Parse,
            "bundle file digest mismatch".to_string(),
        );
        d.data.insert("path".to_string(), json!(profile_rel));
        d.data.insert(
            "want_sha256".to_string(),
            json!(bundle_doc.profile.file.sha256.clone()),
        );
        d.data.insert("got_sha256".to_string(), json!(got_sha));
        d.data.insert(
            "want_bytes_len".to_string(),
            json!(bundle_doc.profile.file.bytes_len),
        );
        d.data.insert("got_bytes_len".to_string(), json!(got_len));
        diagnostics.push(d);
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let profile_json: Value = match serde_json::from_slice(&profile_bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("device profile is not JSON: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-device.profile.schema.json",
        &profile_json,
    )?;
    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_PROFILE_SCHEMA_INVALID",
            Severity::Error,
            Stage::Parse,
            "device profile schema invalid".to_string(),
        );
        d.data.insert("errors".to_string(), json!(schema_diags));
        diagnostics.push(d);
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let profile_doc: DeviceProfileDoc = match serde_json::from_value(profile_json) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse device profile doc: {err}"),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
                target,
                meta,
                diagnostics,
                profile_ref,
                &bundle_dir,
                &out_dir,
                package_manifest_digest,
                package_info,
            );
        }
    };

    if profile_doc.id != bundle_doc.profile.id || profile_doc.v != bundle_doc.profile.v {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PROFILE_ID_MISMATCH",
            Severity::Error,
            Stage::Parse,
            "device profile id/v mismatch".to_string(),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    if profile_doc.target != target {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_FAILED",
            Severity::Error,
            Stage::Parse,
            format!("profile target mismatch: {:?}", profile_doc.target),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    };

    if let Err(err) = std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create dir: {}", out_dir.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    match target {
        "desktop" => {
            let Some(desktop) = profile_doc.desktop.as_ref() else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    "device profile missing desktop config".to_string(),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            };

            let package_kind = desktop.package.kind.as_str();
            let is_archive = match package_kind {
                "dir" => false,
                "archive" => true,
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_PACKAGE_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!("unsupported desktop.package.kind: {package_kind:?}"),
                    ));
                    return emit_report(
                        &store,
                        scope,
                        machine,
                        started,
                        raw_argv,
                        target,
                        meta,
                        diagnostics,
                        profile_ref,
                        &bundle_dir,
                        &out_dir,
                        package_manifest_digest,
                        package_info,
                    );
                }
            };

            if is_archive && desktop.package.format.as_deref() != Some("zip") {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    "desktop.package.kind=archive requires desktop.package.format=zip".to_string(),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            let host_tool_src = match resolve_host_tool_path() {
                Ok(p) => p,
                Err(d) => {
                    diagnostics.push(*d);
                    return emit_report(
                        &store,
                        scope,
                        machine,
                        started,
                        raw_argv,
                        target,
                        meta,
                        diagnostics,
                        profile_ref,
                        &bundle_dir,
                        &out_dir,
                        package_manifest_digest,
                        package_info,
                    );
                }
            };
            if let Err(d) = check_host_tool_abi_hash(&host_tool_src, &bundle_doc.host.host_abi_hash)
            {
                diagnostics.push(*d);
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            let app_name = safe_app_name(&profile_doc.identity.display_name, &profile_doc.id);
            let app_bundle_name = format!("{app_name}.app");
            let app_dir = out_dir.join(&app_bundle_name);

            if app_dir.exists() {
                if let Err(err) = std::fs::remove_dir_all(&app_dir) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to remove existing app bundle {}: {err}",
                            app_dir.display()
                        ),
                    ));
                    return emit_report(
                        &store,
                        scope,
                        machine,
                        started,
                        raw_argv,
                        target,
                        meta,
                        diagnostics,
                        profile_ref,
                        &bundle_dir,
                        &out_dir,
                        package_manifest_digest,
                        package_info,
                    );
                }
            }

            let contents_dir = app_dir.join("Contents");
            let macos_dir = contents_dir.join("MacOS");
            let bundle_dst = contents_dir.join("Resources").join("bundle");

            if let Err(err) = std::fs::create_dir_all(&bundle_dst) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to create app bundle dirs: {err}"),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }
            if let Err(err) = std::fs::create_dir_all(&macos_dir) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to create app bundle dirs: {err}"),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            let host_tool_dst = macos_dir.join("x07-device-host-desktop");
            if let Err(err) = std::fs::copy(&host_tool_src, &host_tool_dst) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "failed to copy host tool {} -> {}: {err}",
                        host_tool_src.display(),
                        host_tool_dst.display()
                    ),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if let Ok(mut perms) = std::fs::metadata(&host_tool_dst).map(|m| m.permissions()) {
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(&host_tool_dst, perms);
                }
            }

            let info_plist_path = contents_dir.join("Info.plist");
            let info_plist = info_plist_xml(
                &profile_doc.identity.display_name,
                &profile_doc.identity.app_id,
                &profile_doc.version.version,
                profile_doc.version.build,
            );
            if let Err(err) = std::fs::write(&info_plist_path, info_plist.as_bytes()) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "failed to write Info.plist {}: {err}",
                        info_plist_path.display()
                    ),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            if let Err(err) = util::copy_dir_recursive(&bundle_dir, &bundle_dst) {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            if is_archive {
                let zip_name = format!("{app_name}.zip");
                let zip_path = out_dir.join(&zip_name);
                let _ = std::fs::remove_file(&zip_path);
                match write_deterministic_zip(&app_dir, &zip_path) {
                    Ok(sha256) => {
                        package_info =
                            json!({ "kind": "archive", "path": zip_name, "sha256": sha256 });
                        if let Ok(d) = file_digest_rel(&out_dir, &zip_path) {
                            meta.outputs.push(d);
                        }
                    }
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
                            Severity::Error,
                            Stage::Run,
                            format!("failed to write zip: {err:#}"),
                        ));
                        return emit_report(
                            &store,
                            scope,
                            machine,
                            started,
                            raw_argv,
                            target,
                            meta,
                            diagnostics,
                            profile_ref,
                            &bundle_dir,
                            &out_dir,
                            package_manifest_digest,
                            package_info,
                        );
                    }
                }
            } else {
                package_info = json!({ "kind": "dir", "path": app_bundle_name });
            }
        }
        "ios" => {
            let Some(ios) = profile_doc.ios.as_ref() else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    "device profile missing ios config".to_string(),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            };

            let payload_name = "ios_project";
            let payload_dir = out_dir.join(payload_name);
            if payload_dir.exists() {
                if let Err(err) = std::fs::remove_dir_all(&payload_dir) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_PACKAGE_TEMPLATE_RENDER_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to remove existing iOS project dir {}: {err}",
                            payload_dir.display()
                        ),
                    ));
                    return emit_report(
                        &store,
                        scope,
                        machine,
                        started,
                        raw_argv,
                        target,
                        meta,
                        diagnostics,
                        profile_ref,
                        &bundle_dir,
                        &out_dir,
                        package_manifest_digest,
                        package_info,
                    );
                }
            }

            let tokens = package_ios::IosPackageTokens {
                display_name: profile_doc.identity.display_name.clone(),
                bundle_id: ios.bundle_id.clone(),
                version: profile_doc.version.version.clone(),
                build: profile_doc.version.build,
            };
            if let Err(d) = package_ios::write_ios_project(&bundle_dir, &payload_dir, tokens) {
                diagnostics.push(*d);
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            package_info = json!({ "kind": "dir", "path": payload_name });
        }
        "android" => {
            let Some(android) = profile_doc.android.as_ref() else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_DEVICE_PACKAGE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    "device profile missing android config".to_string(),
                ));
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            };

            let payload_name = "android_project";
            let payload_dir = out_dir.join(payload_name);
            if payload_dir.exists() {
                if let Err(err) = std::fs::remove_dir_all(&payload_dir) {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_DEVICE_PACKAGE_TEMPLATE_RENDER_FAILED",
                        Severity::Error,
                        Stage::Run,
                        format!(
                            "failed to remove existing Android project dir {}: {err}",
                            payload_dir.display()
                        ),
                    ));
                    return emit_report(
                        &store,
                        scope,
                        machine,
                        started,
                        raw_argv,
                        target,
                        meta,
                        diagnostics,
                        profile_ref,
                        &bundle_dir,
                        &out_dir,
                        package_manifest_digest,
                        package_info,
                    );
                }
            }

            let tokens = package_android::AndroidPackageTokens {
                display_name: profile_doc.identity.display_name.clone(),
                application_id: android.application_id.clone(),
                min_sdk: android.min_sdk,
                version: profile_doc.version.version.clone(),
                build: profile_doc.version.build,
            };
            if let Err(d) =
                package_android::write_android_project(&bundle_dir, &payload_dir, tokens)
            {
                diagnostics.push(*d);
                return emit_report(
                    &store,
                    scope,
                    machine,
                    started,
                    raw_argv,
                    target,
                    meta,
                    diagnostics,
                    profile_ref,
                    &bundle_dir,
                    &out_dir,
                    package_manifest_digest,
                    package_info,
                );
            }

            package_info = json!({ "kind": "dir", "path": payload_name });
        }
        _ => {}
    }

    let mut package_doc = json!({
      "schema_version": "x07.device.package.manifest@0.1.0",
      "kind": "device_package",
      "target": target,
      "bundle_manifest_sha256": bundle_manifest_sha256,
      "package": package_info,
    });

    if let Some(obj) = package_doc.as_object_mut() {
        if let Some(p) = obj.get_mut("package") {
            util::canon_value_jcs(p);
        }
    }

    let schema_diags = store.validate(
        "https://x07.io/spec/x07-device.package.manifest.schema.json",
        &package_doc,
    )?;
    if schema_diags.iter().any(|d| d.severity == Severity::Error) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_INTERNAL_DEVICE_PACKAGE_SCHEMA_INVALID",
            Severity::Error,
            Stage::Run,
            format!("internal error: device package manifest schema invalid: {schema_diags:?}"),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let package_manifest_path = out_dir.join(DEVICE_PACKAGE_MANIFEST_FILE);
    let bytes = report::canon::canonical_pretty_json_bytes(&package_doc)?;
    if let Err(err) = std::fs::write(&package_manifest_path, bytes)
        .with_context(|| format!("write: {}", package_manifest_path.display()))
    {
        diagnostics.push(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_WRITE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            target,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    package_manifest_digest = file_digest_rel(&out_dir, &package_manifest_path)?;
    meta.outputs.push(package_manifest_digest.clone());

    emit_report(
        &store,
        scope,
        machine,
        started,
        raw_argv,
        target,
        meta,
        diagnostics,
        profile_ref,
        &bundle_dir,
        &out_dir,
        package_manifest_digest,
        package_info,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_report(
    store: &SchemaStore,
    scope: Scope,
    machine: &MachineArgs,
    started: std::time::Instant,
    raw_argv: &[OsString],
    target: &str,
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    profile: Value,
    bundle_dir: &Path,
    out_dir: &Path,
    package_manifest: report::meta::FileDigest,
    package: Value,
) -> Result<u8> {
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
      "schema_version": "x07.wasm.device.package.report@0.1.0",
      "command": "x07-wasm.device.package",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "target": target,
        "profile": profile,
        "bundle_dir": bundle_dir.display().to_string(),
        "out_dir": out_dir.display().to_string(),
        "package_manifest": package_manifest,
        "package": package,
      }
    });

    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

fn safe_app_name(display_name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in display_name.chars() {
        let ok = ch.is_ascii_alphanumeric()
            || ch == ' '
            || ch == '-'
            || ch == '_'
            || ch == '.'
            || ch == '('
            || ch == ')';
        if ok {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.to_string()
}

fn escape_plist(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn info_plist_xml(display_name: &str, app_id: &str, version: &str, build: u64) -> String {
    let display_name = escape_plist(display_name);
    let app_id = escape_plist(app_id);
    let version = escape_plist(version);
    let build = build.to_string();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{display_name}</string>
  <key>CFBundleExecutable</key>
  <string>x07-device-host-desktop</string>
  <key>CFBundleIdentifier</key>
  <string>{app_id}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{display_name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>{build}</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#
    )
}

fn resolve_host_tool_path() -> std::result::Result<PathBuf, Box<Diagnostic>> {
    if let Some(p) = std::env::var_os("X07_DEVICE_HOST_DESKTOP") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }

    let name = if cfg!(windows) {
        "x07-device-host-desktop.exe"
    } else {
        "x07-device-host-desktop"
    };
    if let Some(p) = find_in_path(name) {
        return Ok(p);
    }

    Err(Box::new(Diagnostic::new(
        "X07WASM_DEVICE_PACKAGE_HOST_TOOL_MISSING",
        Severity::Error,
        Stage::Run,
        "missing host tool: x07-device-host-desktop (set X07_DEVICE_HOST_DESKTOP or ensure it is on PATH)".to_string(),
    )))
}

fn check_host_tool_abi_hash(
    host_tool: &Path,
    expected: &str,
) -> std::result::Result<(), Box<Diagnostic>> {
    let out = Command::new(host_tool)
        .arg("--host-abi-hash")
        .output()
        .map_err(|err| {
            Box::new(Diagnostic::new(
                "X07WASM_DEVICE_PACKAGE_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to run host tool --host-abi-hash: {err}"),
            ))
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Box::new(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_FAILED",
            Severity::Error,
            Stage::Run,
            if stderr.is_empty() {
                format!(
                    "host tool --host-abi-hash exited with status {}",
                    out.status
                )
            } else {
                format!(
                    "host tool --host-abi-hash exited with status {}: {}",
                    out.status, stderr
                )
            },
        )));
    }
    let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if got != expected {
        let mut d = Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_HOST_ABI_HASH_MISMATCH",
            Severity::Error,
            Stage::Run,
            "host tool ABI hash does not match bundle host_abi_hash".to_string(),
        );
        d.data
            .insert("expected_host_abi_hash".to_string(), json!(expected));
        d.data.insert("got_host_abi_hash".to_string(), json!(got));
        return Err(Box::new(d));
    }
    Ok(())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn write_deterministic_zip(app_dir: &Path, zip_path: &Path) -> Result<String> {
    let prefix = app_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| anyhow::anyhow!("app_dir missing file_name: {}", app_dir.display()))?;

    let f = std::fs::File::create(zip_path)
        .with_context(|| format!("create: {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(f);

    let fixed_time =
        zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).expect("valid zip timestamp");

    let dir_opts = FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(fixed_time)
        .unix_permissions(0o755);

    zip.add_directory(format!("{prefix}/"), dir_opts)?;

    let mut entries = Vec::new();
    collect_entries(app_dir, PathBuf::new(), &mut entries)?;
    entries.sort();

    let exec_prefix = format!("{prefix}/Contents/MacOS/");

    for rel in entries {
        let src_path = app_dir.join(&rel);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let name = format!("{prefix}/{rel_str}");

        if src_path.is_dir() {
            let dir_name = format!("{name}/");
            zip.add_directory(dir_name, dir_opts)?;
            continue;
        }

        let bytes =
            std::fs::read(&src_path).with_context(|| format!("read: {}", src_path.display()))?;
        let perm = if name.starts_with(&exec_prefix) {
            0o755
        } else {
            0o644
        };
        let file_opts = FileOptions::<()>::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(fixed_time)
            .unix_permissions(perm);
        zip.start_file(name, file_opts)?;
        zip.write_all(&bytes)?;
    }

    zip.finish()?;

    let zip_bytes =
        std::fs::read(zip_path).with_context(|| format!("read: {}", zip_path.display()))?;
    Ok(util::sha256_hex(&zip_bytes))
}

fn collect_entries(root: &Path, rel: PathBuf, out: &mut Vec<PathBuf>) -> Result<()> {
    let path = root.join(&rel);
    let mut entries = std::fs::read_dir(&path)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let ty = e.file_type()?;
        let name = e.file_name();
        let mut child = rel.clone();
        child.push(name);
        out.push(child.clone());
        if ty.is_dir() {
            collect_entries(root, child, out)?;
        }
    }
    Ok(())
}

fn file_digest_rel(root: &Path, path: &Path) -> Result<report::meta::FileDigest> {
    let mut d = util::file_digest(path)?;
    if let Ok(rel) = path.strip_prefix(root) {
        d.path = rel.to_string_lossy().to_string();
    }
    Ok(d)
}

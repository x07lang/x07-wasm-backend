use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use zip::write::FileOptions;

use crate::cli::{DevicePackageArgs, MachineArgs, Scope};
use crate::device::contracts::{DeviceBundleManifestDoc, DeviceProfileDoc};
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
    if target != "desktop" {
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
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

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

    if bundle_doc.target != "desktop" {
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
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    let profile_path = PathBuf::from(&bundle_doc.profile.file.path);
    match util::file_digest(&profile_path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read device profile {}: {err:#}",
                    profile_path.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
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

    let profile_bytes = match std::fs::read(&profile_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_DEVICE_PROFILE_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to read device profile {}: {err}",
                    profile_path.display()
                ),
            ));
            return emit_report(
                &store,
                scope,
                machine,
                started,
                raw_argv,
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
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

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
    if let Err(d) = check_host_tool_abi_hash(&host_tool_src, &bundle_doc.host.host_abi_hash) {
        diagnostics.push(*d);
        return emit_report(
            &store,
            scope,
            machine,
            started,
            raw_argv,
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

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
            meta,
            diagnostics,
            profile_ref,
            &bundle_dir,
            &out_dir,
            package_manifest_digest,
            package_info,
        );
    }

    if let Err(err) = copy_dir_recursive(&bundle_dir, &bundle_dst) {
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
                package_info = json!({ "kind": "archive", "path": zip_name, "sha256": sha256 });
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

    let mut package_doc = json!({
      "schema_version": "x07.device.package.manifest@0.1.0",
      "kind": "device_package",
      "target": "desktop",
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
        "target": "desktop",
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    let mut entries = std::fs::read_dir(src)
        .with_context(|| format!("read dir: {}", src.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let ty = e.file_type()?;
        let name = e.file_name();
        let src_path = e.path();
        let dst_path = dst.join(name);
        if ty.is_dir() {
            std::fs::create_dir_all(&dst_path)
                .with_context(|| format!("create dir: {}", dst_path.display()))?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            std::fs::copy(&src_path, &dst_path).with_context(|| {
                format!("copy {} -> {}", src_path.display(), dst_path.display())
            })?;
        }
    }
    Ok(())
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

use std::path::Path;

use anyhow::Context as _;

use crate::device::mobile_templates;
use crate::device::template_render::{self, Replacement};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::util;

#[derive(Debug, Clone)]
pub(crate) struct AndroidPackageTokens {
    pub(crate) display_name: String,
    pub(crate) application_id: String,
    pub(crate) min_sdk: u64,
    pub(crate) version: String,
    pub(crate) build: u64,
}

const REQUIRED_TEMPLATE_FILES: &[&str] = &[
    "settings.gradle.kts",
    "app/build.gradle.kts",
    "app/src/main/AndroidManifest.xml",
    "app/src/main/assets/x07/index.html",
];

pub(crate) fn write_android_project(
    bundle_dir: &Path,
    dst_project_dir: &Path,
    tokens: AndroidPackageTokens,
) -> std::result::Result<(), Box<Diagnostic>> {
    let files = mobile_templates::ANDROID_TEMPLATE_FILES;

    for want in REQUIRED_TEMPLATE_FILES {
        if !files.iter().any(|f| f.path == *want) {
            return Err(Box::new(Diagnostic::new(
                "X07WASM_DEVICE_PACKAGE_ANDROID_TEMPLATE_MISSING",
                Severity::Error,
                Stage::Run,
                format!("internal Android template missing required file: {want}"),
            )));
        }
    }

    let display_name_xml = template_render::escape_xml(&tokens.display_name);
    let min_sdk_str = tokens.min_sdk.to_string();
    let build_str = tokens.build.to_string();
    let replacements = vec![
        Replacement {
            needle: "__X07_DISPLAY_NAME__",
            value: display_name_xml,
        },
        Replacement {
            needle: "__X07_ANDROID_APPLICATION_ID__",
            value: tokens.application_id,
        },
        Replacement {
            needle: "__X07_ANDROID_MIN_SDK__",
            value: min_sdk_str,
        },
        Replacement {
            needle: "__X07_VERSION__",
            value: tokens.version,
        },
        Replacement {
            needle: "__X07_BUILD__",
            value: build_str,
        },
    ];

    template_render::render_template_dir("android_template", files, dst_project_dir, &replacements)
        .map_err(|err| {
            Box::new(Diagnostic::new(
                "X07WASM_DEVICE_PACKAGE_TEMPLATE_RENDER_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to render Android template: {err:#}"),
            ))
        })?;

    let x07_dir = dst_project_dir
        .join("app")
        .join("src")
        .join("main")
        .join("assets")
        .join("x07");
    std::fs::create_dir_all(&x07_dir)
        .with_context(|| format!("create dir: {}", x07_dir.display()))
        .map_err(|err| {
            Box::new(Diagnostic::new(
                "X07WASM_DEVICE_PACKAGE_TEMPLATE_RENDER_FAILED",
                Severity::Error,
                Stage::Run,
                format!("failed to prepare Android x07 assets dir: {err:#}"),
            ))
        })?;

    util::copy_dir_recursive(bundle_dir, &x07_dir).map_err(|err| {
        Box::new(Diagnostic::new(
            "X07WASM_DEVICE_PACKAGE_TEMPLATE_RENDER_FAILED",
            Severity::Error,
            Stage::Run,
            format!("failed to embed device bundle into Android project: {err:#}"),
        ))
    })?;

    Ok(())
}

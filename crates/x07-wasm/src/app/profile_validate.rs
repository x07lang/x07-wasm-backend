use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::app::load::{AppIndexDoc, AppProfileDoc};
use crate::cli::{AppProfileValidateArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn cmd_app_profile_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: AppProfileValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut checked_web_ui_profiles: u64 = 0;
    let mut checked_component_profiles: u64 = 0;
    let mut checked_projects: u64 = 0;

    let web_ui_profiles = read_profile_index(
        &store,
        &args.web_ui_index,
        "https://x07.io/spec/x07-arch.web_ui.index.schema.json",
        "X07WASM_APP_WEB_UI_INDEX_READ_FAILED",
        "X07WASM_APP_WEB_UI_INDEX_JSON_INVALID",
        &mut meta,
        &mut diagnostics,
    )
    .unwrap_or_default();

    let component_profiles = read_profile_index(
        &store,
        &args.component_index,
        "https://x07.io/spec/x07-arch.wasm.component.index.schema.json",
        "X07WASM_APP_COMPONENT_INDEX_READ_FAILED",
        "X07WASM_APP_COMPONENT_INDEX_JSON_INVALID",
        &mut meta,
        &mut diagnostics,
    )
    .unwrap_or_default();

    let app_profiles: Vec<(PathBuf, AppProfileDoc)> = if let Some(path) = &args.profile_file {
        let digest = match util::file_digest(path) {
            Ok(d) => {
                meta.inputs.push(d.clone());
                Some(d)
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PROFILE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to digest profile file {}: {err:#}", path.display()),
                ));
                None
            }
        };
        let bytes = match std::fs::read(path) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PROFILE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to read profile file {}: {err}", path.display()),
                ));
                Vec::new()
            }
        };
        let doc_json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PROFILE_JSON_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse profile JSON {}: {err}", path.display()),
                ));
                json!(null)
            }
        };
        diagnostics
            .extend(store.validate("https://x07.io/spec/x07-app.profile.schema.json", &doc_json)?);
        let parsed: Option<AppProfileDoc> = match serde_json::from_value(doc_json) {
            Ok(v) => Some(v),
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_PROFILE_PARSE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse profile doc {}: {err}", path.display()),
                ));
                None
            }
        };

        match (digest, parsed) {
            (Some(d), Some(p)) => vec![(PathBuf::from(&d.path), p)],
            _ => Vec::new(),
        }
    } else {
        let idx = read_app_index(&store, &args.index, &mut meta, &mut diagnostics);
        if let Some(idx) = idx {
            let wanted = args.profile.as_deref();
            let mut out: Vec<(PathBuf, AppProfileDoc)> = Vec::new();
            for p in idx.profiles {
                if let Some(w) = wanted {
                    if p.id != w {
                        continue;
                    }
                }
                let path = PathBuf::from(p.path);
                let bytes = match std::fs::read(&path) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_APP_PROFILE_READ_FAILED",
                            Severity::Error,
                            Stage::Parse,
                            format!("failed to read profile {}: {err}", path.display()),
                        ));
                        continue;
                    }
                };
                if let Ok(d) = util::file_digest(&path) {
                    meta.inputs.push(d);
                }
                let doc_json: serde_json::Value = match serde_json::from_slice(&bytes) {
                    Ok(v) => v,
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_APP_PROFILE_JSON_INVALID",
                            Severity::Error,
                            Stage::Parse,
                            format!("failed to parse profile JSON {}: {err}", path.display()),
                        ));
                        continue;
                    }
                };
                diagnostics.extend(
                    store.validate("https://x07.io/spec/x07-app.profile.schema.json", &doc_json)?,
                );
                match serde_json::from_value::<AppProfileDoc>(doc_json) {
                    Ok(doc) => out.push((path, doc)),
                    Err(err) => diagnostics.push(Diagnostic::new(
                        "X07WASM_APP_PROFILE_PARSE_FAILED",
                        Severity::Error,
                        Stage::Parse,
                        format!("failed to parse profile doc {}: {err}", path.display()),
                    )),
                }
            }
            if out.is_empty() && wanted.is_some() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_INDEX_PROFILE_NOT_FOUND",
                    Severity::Error,
                    Stage::Parse,
                    format!("profile id not found: {:?}", args.profile),
                ));
            }
            out
        } else {
            Vec::new()
        }
    };

    let checked_profiles: u64 = app_profiles.len() as u64;
    for (_path, profile) in &app_profiles {
        checked_projects = checked_projects.saturating_add(2);
        check_project_exists(&profile.frontend.project, &mut meta, &mut diagnostics);
        check_project_exists(&profile.backend.project, &mut meta, &mut diagnostics);

        let web_ui_profile_path =
            find_profile_path_by_id(&web_ui_profiles, &profile.frontend.web_ui_profile_id);
        match web_ui_profile_path {
            Some(p) => {
                checked_web_ui_profiles = checked_web_ui_profiles.saturating_add(1);
                validate_profile_doc_file(
                    &store,
                    &p,
                    "https://x07.io/spec/x07-web_ui.profile.schema.json",
                    "X07WASM_APP_WEB_UI_PROFILE_READ_FAILED",
                    "X07WASM_APP_WEB_UI_PROFILE_JSON_INVALID",
                    &mut meta,
                    &mut diagnostics,
                )?;
            }
            None => diagnostics.push(Diagnostic::new(
                "X07WASM_APP_WEB_UI_PROFILE_ID_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!(
                    "web_ui_profile_id not found in index: {:?}",
                    profile.frontend.web_ui_profile_id
                ),
            )),
        }

        let component_profile_path =
            find_profile_path_by_id(&component_profiles, &profile.backend.component_profile_id);
        match component_profile_path {
            Some(p) => {
                checked_component_profiles = checked_component_profiles.saturating_add(1);
                validate_profile_doc_file(
                    &store,
                    &p,
                    "https://x07.io/spec/x07-wasm.component.profile.schema.json",
                    "X07WASM_APP_COMPONENT_PROFILE_READ_FAILED",
                    "X07WASM_APP_COMPONENT_PROFILE_JSON_INVALID",
                    &mut meta,
                    &mut diagnostics,
                )?;
            }
            None => diagnostics.push(Diagnostic::new(
                "X07WASM_APP_COMPONENT_PROFILE_ID_NOT_FOUND",
                Severity::Error,
                Stage::Parse,
                format!(
                    "component_profile_id not found in index: {:?}",
                    profile.backend.component_profile_id
                ),
            )),
        }
    }

    if args.strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count() as u64;
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count() as u64;

    let ok = errors == 0;
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let stdout_json = json!({
      "index": args.index.display().to_string(),
      "checked": {
        "profiles": checked_profiles,
        "web_ui_profiles": checked_web_ui_profiles,
        "component_profiles": checked_component_profiles,
        "projects": checked_projects
      },
      "warnings": warnings,
      "errors": errors
    });

    let report_doc = json!({
      "schema_version": "x07.wasm.app.profile.validate.report@0.1.0",
      "command": "x07-wasm.app.profile.validate",
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

#[derive(Debug, Clone, Deserialize)]
struct ProfileRefDoc {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProfilesIndexDoc {
    profiles: Vec<ProfileRefDoc>,
}

fn read_profile_index(
    store: &SchemaStore,
    index_path: &PathBuf,
    schema_id: &str,
    diag_read_failed: &str,
    diag_json_invalid: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<ProfileRefDoc>> {
    let digest = match util::file_digest(index_path) {
        Ok(d) => Some(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_read_failed,
                Severity::Error,
                Stage::Parse,
                format!("failed to digest index {}: {err:#}", index_path.display()),
            ));
            None
        }
    };
    if let Some(d) = digest.as_ref() {
        meta.inputs.push(d.clone());
    }
    let bytes = match std::fs::read(index_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_read_failed,
                Severity::Error,
                Stage::Parse,
                format!("failed to read index {}: {err}", index_path.display()),
            ));
            Vec::new()
        }
    };
    let doc_json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_json_invalid,
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index JSON {}: {err}", index_path.display()),
            ));
            json!(null)
        }
    };
    diagnostics.extend(store.validate(schema_id, &doc_json)?);
    let parsed: Option<ProfilesIndexDoc> = match serde_json::from_value(doc_json.clone()) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index doc {}: {err}", index_path.display()),
            ));
            None
        }
    };
    let _ = digest;
    Ok(parsed.map(|p| p.profiles).unwrap_or_default())
}

fn read_app_index(
    store: &SchemaStore,
    index_path: &PathBuf,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<AppIndexDoc> {
    let digest = match util::file_digest(index_path) {
        Ok(d) => Some(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to digest index {}: {err:#}", index_path.display()),
            ));
            None
        }
    };
    if let Some(d) = digest.as_ref() {
        meta.inputs.push(d.clone());
    }

    let bytes = match std::fs::read(index_path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to read index {}: {err}", index_path.display()),
            ));
            Vec::new()
        }
    };

    let doc_json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_JSON_INVALID",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index JSON {}: {err}", index_path.display()),
            ));
            json!(null)
        }
    };
    let index_diags = match store.validate(
        "https://x07.io/spec/x07-arch.app.index.schema.json",
        &doc_json,
    ) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_SCHEMA_VALIDATE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("{err:#}"),
            ));
            Vec::new()
        }
    };
    diagnostics.extend(index_diags);
    let parsed: Option<AppIndexDoc> = match serde_json::from_value(doc_json.clone()) {
        Ok(v) => Some(v),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_INDEX_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse index doc {}: {err}", index_path.display()),
            ));
            None
        }
    };
    let _ = doc_json;
    let _ = digest;
    parsed
}

fn find_profile_path_by_id(profiles: &[ProfileRefDoc], id: &str) -> Option<PathBuf> {
    profiles
        .iter()
        .find(|p| p.id == id)
        .map(|p| PathBuf::from(&p.path))
}

fn validate_profile_doc_file(
    store: &SchemaStore,
    path: &Path,
    schema_id: &str,
    diag_read_failed: &str,
    diag_json_invalid: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let digest = match util::file_digest(path) {
        Ok(d) => {
            meta.inputs.push(d);
            true
        }
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_read_failed,
                Severity::Error,
                Stage::Parse,
                format!("failed to digest {}: {err:#}", path.display()),
            ));
            false
        }
    };
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_read_failed,
                Severity::Error,
                Stage::Parse,
                format!("failed to read {}: {err}", path.display()),
            ));
            Vec::new()
        }
    };
    let doc_json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                diag_json_invalid,
                Severity::Error,
                Stage::Parse,
                format!("failed to parse JSON {}: {err}", path.display()),
            ));
            json!(null)
        }
    };
    diagnostics.extend(store.validate(schema_id, &doc_json)?);
    let _ = digest;
    Ok(())
}

fn check_project_exists(
    project: &str,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let path = Path::new(project);
    if !path.is_file() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_APP_PROJECT_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("project manifest not found: {}", path.display()),
        ));
        return;
    }

    match util::file_digest(path) {
        Ok(d) => meta.inputs.push(d),
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_APP_PROJECT_READ_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to digest project {}: {err:#}", path.display()),
            ));
        }
    }
}

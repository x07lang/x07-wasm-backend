use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::diag::Severity;
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

const DEFAULT_PROFILE_ID: &str = "app_dev";

#[derive(Debug, Clone, Deserialize)]
pub struct AppIndexDoc {
    pub profiles: Vec<AppIndexProfileRef>,
    #[serde(default)]
    pub defaults: Option<AppIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppIndexDefaults {
    pub default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppIndexProfileRef {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileFrontend {
    pub format: String,
    pub project: String,
    pub web_ui_profile_id: String,
    pub out_dir_rel: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileBackend {
    pub adapter: String,
    pub project: String,
    pub component_profile_id: String,
    pub out_rel: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileRouting {
    pub api_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileBudgets {
    pub max_dispatch_bytes: u64,
    pub max_frame_bytes: u64,
    pub max_http_body_bytes: u64,
    pub max_concurrency: u32,
    pub max_request_wall_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileDevserver {
    pub addr: String,
    pub strict_wasm_mime: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfileDoc {
    pub id: String,

    pub frontend: AppProfileFrontend,
    pub backend: AppProfileBackend,
    pub routing: AppProfileRouting,
    pub budgets: AppProfileBudgets,
    pub devserver: AppProfileDevserver,
}

pub struct LoadedAppProfile {
    pub digest: report::meta::FileDigest,
    pub doc: AppProfileDoc,
    pub index_digest: Option<report::meta::FileDigest>,
}

pub fn read_app_index(
    store: &SchemaStore,
    index_path: &PathBuf,
) -> Result<(report::meta::FileDigest, Value, AppIndexDoc)> {
    let digest = util::file_digest(index_path)?;
    let bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;

    let diags = store.validate(
        "https://x07.io/spec/x07-arch.app.index.schema.json",
        &doc_json,
    )?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid app index: {}", digest.path);
    }

    let doc: AppIndexDoc = serde_json::from_value(doc_json.clone()).context("parse app index")?;
    Ok((digest, doc_json, doc))
}

pub fn load_app_profile(
    store: &SchemaStore,
    index_path: &PathBuf,
    profile_id: Option<&str>,
    profile_file: Option<&PathBuf>,
) -> Result<LoadedAppProfile> {
    if let Some(path) = profile_file {
        return load_app_profile_file(store, path, None);
    }

    let (index_digest, _index_doc_json, idx) = read_app_index(store, index_path)?;

    let default_id = idx
        .defaults
        .as_ref()
        .and_then(|d| d.default_profile_id.clone())
        .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
    let wanted = profile_id.unwrap_or(&default_id);
    let entry = idx
        .profiles
        .iter()
        .find(|p| p.id == wanted)
        .ok_or_else(|| anyhow::anyhow!("profile id not found in index: {wanted:?}"))?;
    let profile_path = PathBuf::from(&entry.path);
    load_app_profile_file(store, &profile_path, Some(index_digest))
}

fn load_app_profile_file(
    store: &SchemaStore,
    path: &PathBuf,
    index_digest: Option<report::meta::FileDigest>,
) -> Result<LoadedAppProfile> {
    let digest = util::file_digest(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("read: {}", path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    let diags = store.validate("https://x07.io/spec/x07-app.profile.schema.json", &doc_json)?;
    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("invalid app profile: {}", digest.path);
    }
    let doc: AppProfileDoc = serde_json::from_value(doc_json)
        .with_context(|| format!("parse app profile: {}", path.display()))?;
    Ok(LoadedAppProfile {
        digest,
        doc,
        index_digest,
    })
}

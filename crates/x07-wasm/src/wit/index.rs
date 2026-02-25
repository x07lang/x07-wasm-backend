use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use wit_parser::{PackageName, UnresolvedPackageGroup};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
pub struct WitIndexDoc {
    pub packages: Vec<WitIndexPackageRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WitIndexPackageRef {
    pub id: String,
    pub path: String,
    pub kind: String,
    #[serde(default)]
    pub sha256_tree: Option<String>,
}

pub fn read_index(
    store: &SchemaStore,
    index_path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(report::meta::FileDigest, Value, WitIndexDoc)> {
    let digest = util::file_digest(index_path)?;
    meta.inputs.push(digest.clone());

    let bytes =
        std::fs::read(index_path).with_context(|| format!("read: {}", index_path.display()))?;
    let doc_json: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", index_path.display()))?;

    diagnostics.extend(store.validate(
        "https://x07.io/spec/x07-arch.wit.index.schema.json",
        &doc_json,
    )?);

    let doc: WitIndexDoc =
        serde_json::from_value(doc_json.clone()).context("parse arch/wit index")?;

    let mut seen = BTreeSet::new();
    for p in &doc.packages {
        if !seen.insert(p.id.clone()) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_INDEX_DUPLICATE_PACKAGE_ID",
                Severity::Error,
                Stage::Parse,
                format!("duplicate package id in index: {:?}", p.id),
            ));
        }
    }

    Ok((digest, doc_json, doc))
}

pub fn package_map(doc: &WitIndexDoc) -> BTreeMap<String, WitIndexPackageRef> {
    doc.packages
        .iter()
        .cloned()
        .map(|p| (p.id.clone(), p))
        .collect()
}

pub struct ParsedWitPackage {
    pub name: PackageName,
    pub worlds: Vec<String>,
    pub deps: Vec<PackageName>,
}

pub fn parse_wit_package(dir: &Path) -> Result<ParsedWitPackage> {
    let group = UnresolvedPackageGroup::parse_dir(dir).with_context(|| {
        format!(
            "wit_parser::UnresolvedPackageGroup::parse_dir({})",
            dir.display()
        )
    })?;

    let name = group.main.name.clone();

    let mut worlds: Vec<String> = group
        .main
        .worlds
        .iter()
        .map(|(_, w)| w.name.clone())
        .filter(|w| !w.is_empty())
        .collect();
    worlds.sort();

    let mut deps: Vec<PackageName> = group.main.foreign_deps.keys().cloned().collect();
    deps.sort_by_key(format_pkg_id);

    Ok(ParsedWitPackage { name, worlds, deps })
}

pub fn format_pkg_id(name: &PackageName) -> String {
    let Some(v) = name.version.as_ref() else {
        return format!("{}:{}", name.namespace, name.name);
    };
    format!("{}:{}@{}", name.namespace, name.name, v)
}

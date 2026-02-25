use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::wit::index::{format_pkg_id, package_map, parse_wit_package, WitIndexDoc};

pub struct WitBundle {
    pub dir: PathBuf,
}

pub fn bundle_for_wit_path(
    store: &SchemaStore,
    wit_index_path: &Path,
    wit_path: &Path,
    meta: &mut report::meta::ReportMeta,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<WitBundle>> {
    let main_dir = if wit_path.is_dir() {
        wit_path.to_path_buf()
    } else if wit_path.is_file() {
        match wit_path.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_BUNDLE_WIT_PATH_INVALID",
                    Severity::Error,
                    Stage::Parse,
                    format!("wit path has no parent dir: {}", wit_path.display()),
                ));
                return Ok(None);
            }
        }
    } else {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WIT_BUNDLE_WIT_PATH_MISSING",
            Severity::Error,
            Stage::Parse,
            format!("wit path does not exist: {}", wit_path.display()),
        ));
        return Ok(None);
    };

    let main_pkg = match parse_wit_package(&main_dir) {
        Ok(v) => v,
        Err(err) => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_BUNDLE_MAIN_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to parse main WIT package {}: {err:#}",
                    main_dir.display()
                ),
            ));
            return Ok(None);
        }
    };
    let main_id = format_pkg_id(&main_pkg.name);

    let (_index_digest, _index_json, index_doc) =
        match crate::wit::index::read_index(store, wit_index_path, meta, diagnostics) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_BUNDLE_INDEX_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("{err:#}"),
                ));
                return Ok(None);
            }
        };

    let index_map = package_map(&index_doc);
    if !index_map.contains_key(&main_id) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WIT_BUNDLE_MAIN_NOT_IN_INDEX",
            Severity::Warning,
            Stage::Parse,
            format!(
                "main WIT package is not present in index: {} (dir={})",
                main_id,
                main_dir.display()
            ),
        ));
    }

    let deps = transitive_deps(&main_id, &main_dir, &index_doc, diagnostics)?;

    let index_digest = util::file_digest(wit_index_path)?;
    let key = util::sha256_hex(format!("{}:{main_id}", index_digest.sha256).as_bytes());
    let bundle_dir = PathBuf::from("target")
        .join("x07-wasm")
        .join("wit-bundles")
        .join(format!("{}-{}", sanitize_pkg_dir(&main_id), &key[..16]));

    if let Err(err) = materialize_bundle(&bundle_dir, &main_dir, &deps) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WIT_BUNDLE_MATERIALIZE_FAILED",
            Severity::Error,
            Stage::Run,
            format!("{err:#}"),
        ));
        return Ok(None);
    }

    Ok(Some(WitBundle { dir: bundle_dir }))
}

fn transitive_deps(
    main_id: &str,
    main_dir: &Path,
    index_doc: &WitIndexDoc,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<(String, PathBuf)>> {
    let base_dir = Path::new(".");
    let index_map: BTreeMap<String, crate::wit::index::WitIndexPackageRef> = package_map(index_doc);

    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<(String, PathBuf)> = Vec::new();
    seen.insert(main_id.to_string());
    queue.push((main_id.to_string(), main_dir.to_path_buf()));

    while let Some((pkg_id, pkg_dir)) = queue.pop() {
        let parsed = parse_wit_package(&pkg_dir).with_context(|| {
            format!(
                "parse WIT package for bundling: {} ({})",
                pkg_id,
                pkg_dir.display()
            )
        })?;
        for dep in parsed.deps {
            let dep_id = format_pkg_id(&dep);
            if !seen.insert(dep_id.clone()) {
                continue;
            }
            let Some(dep_ref) = index_map.get(&dep_id) else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_BUNDLE_DEP_NOT_IN_INDEX",
                    Severity::Warning,
                    Stage::Parse,
                    format!(
                        "missing dependency in registry: {} depends on {}",
                        pkg_id, dep_id
                    ),
                ));
                continue;
            };
            let dep_dir = base_dir.join(&dep_ref.path);
            if !dep_dir.is_dir() {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_BUNDLE_DEP_DIR_MISSING",
                    Severity::Warning,
                    Stage::Parse,
                    format!(
                        "dependency dir missing: {} -> {}",
                        dep_id,
                        dep_dir.display()
                    ),
                ));
                continue;
            }
            out.push((dep_id.clone(), dep_dir.clone()));
            queue.push((dep_id, dep_dir));
        }
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn materialize_bundle(
    bundle_dir: &Path,
    main_dir: &Path,
    deps: &[(String, PathBuf)],
) -> Result<()> {
    if let Err(err) = std::fs::remove_dir_all(bundle_dir) {
        if err.kind() != std::io::ErrorKind::NotFound {
            return Err(anyhow::Error::new(err))
                .with_context(|| format!("remove existing bundle dir: {}", bundle_dir.display()));
        }
    }
    std::fs::create_dir_all(bundle_dir)
        .with_context(|| format!("create bundle dir: {}", bundle_dir.display()))?;

    copy_wit_files(main_dir, bundle_dir)?;

    let deps_dir = bundle_dir.join("deps");
    std::fs::create_dir_all(&deps_dir)
        .with_context(|| format!("create deps dir: {}", deps_dir.display()))?;

    for (pkg_id, pkg_dir) in deps {
        let dst = deps_dir.join(sanitize_pkg_dir(pkg_id));
        std::fs::create_dir_all(&dst)
            .with_context(|| format!("create dep dir: {}", dst.display()))?;
        copy_wit_files(pkg_dir, &dst)?;
    }

    Ok(())
}

fn copy_wit_files(src_dir: &Path, dst_dir: &Path) -> Result<()> {
    let rd =
        std::fs::read_dir(src_dir).with_context(|| format!("read_dir: {}", src_dir.display()))?;
    for entry in rd {
        let entry = entry.with_context(|| format!("read_dir entry: {}", src_dir.display()))?;
        let p = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("stat: {}", p.display()))?;
        if !ft.is_file() {
            continue;
        }
        if p.extension().and_then(|s| s.to_str()) != Some("wit") {
            continue;
        }
        let Some(name) = p.file_name() else {
            continue;
        };
        let out = dst_dir.join(name);
        std::fs::copy(&p, &out)
            .with_context(|| format!("copy {} -> {}", p.display(), out.display()))?;
    }
    Ok(())
}

fn sanitize_pkg_dir(pkg_id: &str) -> String {
    pkg_id.replace([':', '/', '@'], "-")
}

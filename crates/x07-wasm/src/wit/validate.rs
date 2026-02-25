use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{MachineArgs, Scope, WitValidateArgs};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;
use crate::wit::index::{
    format_pkg_id, package_map, parse_wit_package, read_index, WitIndexDoc, WitIndexPackageRef,
};

pub fn cmd_wit_validate(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: WitValidateArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();
    let store = SchemaStore::new()?;

    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_network = false;
    meta.nondeterminism.uses_os_time = false;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let (index_digest, index_doc_json, index_parsed) =
        match read_index(&store, &args.index, &mut meta, &mut diagnostics) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_INDEX_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("{err:#}"),
                ));
                let report_doc = build_report_doc(
                    meta,
                    diagnostics,
                    false,
                    1,
                    args.strict,
                    "validate",
                    json!({
                      "schema_version": "x07.wasm.wit.validate.report@0.1.0",
                      "command": "x07-wasm.wit.validate",
                    }),
                    report::meta::FileDigest {
                        path: args.index.display().to_string(),
                        sha256: "0".repeat(64),
                        bytes_len: 0,
                    },
                    Vec::new(),
                );
                store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
                return Ok(1);
            }
        };

    let index_pkg_map: BTreeMap<String, WitIndexPackageRef> = package_map(&index_parsed);

    let selected_ids = selected_package_ids(&index_parsed, &args.package);
    let mut packages_summary: Vec<Value> = Vec::new();
    let mut packages_ok = 0u32;
    let mut packages_failed = 0u32;

    let base_dir = Path::new(".");

    for pkg_id in selected_ids {
        let Some(pkg) = index_pkg_map.get(&pkg_id) else {
            continue;
        };

        let pkg_summary =
            validate_one_package(&args, base_dir, pkg, &index_pkg_map, &mut diagnostics);
        let ok = pkg_summary
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if ok {
            packages_ok += 1;
        } else {
            packages_failed += 1;
        }
        packages_summary.push(pkg_summary);
    }

    let strict = args.strict;
    if strict {
        for d in diagnostics.iter_mut() {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code: u8 = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let mode = if args.list { "list" } else { "validate" };
    let report_doc = json!({
      "schema_version": "x07.wasm.wit.validate.report@0.1.0",
      "command": "x07-wasm.wit.validate",
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "strict": strict,
        "index": index_digest,
        "packages_total": (packages_ok + packages_failed),
        "packages_ok": packages_ok,
        "packages_failed": packages_failed,
        "packages": packages_summary,
      }
    });

    let _ = index_doc_json;
    store.validate_report_and_emit(scope, machine, started, raw_argv, report_doc)?;
    Ok(exit_code)
}

#[allow(clippy::too_many_arguments)]
fn build_report_doc(
    meta: report::meta::ReportMeta,
    diagnostics: Vec<Diagnostic>,
    ok: bool,
    exit_code: u8,
    strict: bool,
    mode: &str,
    header: Value,
    index_digest: report::meta::FileDigest,
    packages: Vec<Value>,
) -> Value {
    let _ = (ok, exit_code);
    json!({
      "schema_version": header.get("schema_version").and_then(Value::as_str).unwrap_or("x07.wasm.wit.validate.report@0.1.0"),
      "command": header.get("command").and_then(Value::as_str).unwrap_or("x07-wasm.wit.validate"),
      "ok": ok,
      "exit_code": exit_code,
      "diagnostics": diagnostics,
      "meta": meta,
      "result": {
        "mode": mode,
        "strict": strict,
        "index": index_digest,
        "packages_total": packages.len() as u32,
        "packages_ok": 0,
        "packages_failed": packages.len() as u32,
        "packages": packages,
      }
    })
}

fn selected_package_ids(index: &WitIndexDoc, filter: &[String]) -> Vec<String> {
    if filter.is_empty() {
        return index.packages.iter().map(|p| p.id.clone()).collect();
    }
    let wanted: BTreeSet<String> = filter.iter().cloned().collect();
    index
        .packages
        .iter()
        .filter(|p| wanted.contains(&p.id))
        .map(|p| p.id.clone())
        .collect()
}

fn validate_one_package(
    args: &WitValidateArgs,
    base_dir: &Path,
    pkg: &WitIndexPackageRef,
    all: &BTreeMap<String, WitIndexPackageRef>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Value {
    let pkg_path = base_dir.join(&pkg.path);

    let mut ok = true;

    if !pkg_path.is_dir() {
        diagnostics.push(Diagnostic::new(
            "X07WASM_WIT_PACKAGE_PATH_NOT_DIR",
            Severity::Error,
            Stage::Parse,
            format!("package path is not a directory: {}", pkg_path.display()),
        ));
        ok = false;
    }

    let tree = if ok {
        match sha256_tree_wit_dir(&pkg_path) {
            Ok(v) => v,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_SHA256_TREE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "failed to compute sha256_tree for {}: {err:#}",
                        pkg_path.display()
                    ),
                ));
                ok = false;
                "0".repeat(64)
            }
        }
    } else {
        "0".repeat(64)
    };

    match pkg.kind.as_str() {
        "vendored" => match pkg.sha256_tree.as_deref() {
            Some(expected) if expected == tree => {}
            Some(expected) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_SHA256_TREE_MISMATCH",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "sha256_tree mismatch for {}: expected {expected}, got {tree}",
                        pkg.id
                    ),
                ));
                ok = false;
            }
            None => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_SHA256_TREE_MISSING",
                    Severity::Error,
                    Stage::Parse,
                    format!("vendored package missing sha256_tree pin: {}", pkg.id),
                ));
                ok = false;
            }
        },
        "local" => match pkg.sha256_tree.as_deref() {
            Some(expected) if expected == tree => {}
            Some(expected) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_SHA256_TREE_MISMATCH",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "sha256_tree mismatch for {}: expected {expected}, got {tree}",
                        pkg.id
                    ),
                ));
                ok = false;
            }
            None => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_SHA256_TREE_UNPINNED",
                    Severity::Warning,
                    Stage::Parse,
                    format!("local package is not sha256_tree pinned: {}", pkg.id),
                ));
                if args.strict {
                    ok = false;
                }
            }
        },
        other => {
            diagnostics.push(Diagnostic::new(
                "X07WASM_WIT_PACKAGE_KIND_UNKNOWN",
                Severity::Error,
                Stage::Parse,
                format!("unknown wit package kind in index: {other:?}"),
            ));
            ok = false;
        }
    }

    let mut worlds: Vec<String> = Vec::new();
    let mut deps: Vec<String> = Vec::new();
    if ok {
        match parse_wit_package(&pkg_path) {
            Ok(parsed) => {
                let parsed_id = format_pkg_id(&parsed.name);
                if parsed_id != pkg.id {
                    diagnostics.push(Diagnostic::new(
                        "X07WASM_WIT_PACKAGE_ID_MISMATCH",
                        Severity::Error,
                        Stage::Parse,
                        format!(
                            "package id mismatch for {}: index has {:?}, WIT declares {:?}",
                            pkg.path, pkg.id, parsed_id
                        ),
                    ));
                    ok = false;
                }
                worlds = parsed.worlds;
                deps = parsed.deps.into_iter().map(|n| format_pkg_id(&n)).collect();

                for dep in deps.iter() {
                    if !all.contains_key(dep) {
                        diagnostics.push(Diagnostic::new(
                            "X07WASM_WIT_PACKAGE_DEP_MISSING",
                            Severity::Warning,
                            Stage::Parse,
                            format!(
                                "missing dependency in registry: {} depends on {}",
                                pkg.id, dep
                            ),
                        ));
                        if args.strict {
                            ok = false;
                        }
                    }
                }
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_WIT_PACKAGE_PARSE_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to parse WIT package {}: {err:#}", pkg.path),
                ));
                ok = false;
            }
        }
    }

    json!({
      "id": pkg.id,
      "path": pkg.path,
      "kind": pkg.kind,
      "ok": ok,
      "sha256_tree": tree,
      "worlds": worlds,
      "dependencies": deps,
    })
}

fn sha256_tree_wit_dir(dir: &Path) -> Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    gather_wit_files(dir, dir, &mut files)?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for path in files {
        let rel = relpath_slash(dir, &path)?;
        let bytes = std::fs::read(&path).with_context(|| format!("read: {}", path.display()))?;
        let sha = util::sha256_hex(&bytes);
        entries.push((rel, sha));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut buf = Vec::new();
    for (rel, sha) in entries {
        buf.extend_from_slice(sha.as_bytes());
        buf.extend_from_slice(b"  ");
        buf.extend_from_slice(rel.as_bytes());
        buf.push(b'\n');
    }
    Ok(util::sha256_hex(&buf))
}

fn gather_wit_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let rd = std::fs::read_dir(dir).with_context(|| format!("read_dir: {}", dir.display()))?;
    for entry in rd {
        let entry = entry.with_context(|| format!("read_dir entry: {}", dir.display()))?;
        let p = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("stat: {}", p.display()))?;
        if ft.is_dir() {
            gather_wit_files(root, &p, out)?;
            continue;
        }
        if ft.is_file() && p.extension().and_then(|s| s.to_str()) == Some("wit") {
            out.push(p);
        }
    }
    let _ = root;
    Ok(())
}

fn relpath_slash(root: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(root).with_context(|| {
        format!(
            "strip_prefix root={} path={}",
            root.display(),
            path.display()
        )
    })?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn vendored_wasi_worlds_are_nonempty() {
        let base = repo_root().join("wit/deps/wasi");
        assert!(
            base.is_dir(),
            "missing vendored WIT dir: {}",
            base.display()
        );
        for pkg_dir in std::fs::read_dir(base).unwrap() {
            let pkg_dir = pkg_dir.unwrap().path();
            let v = pkg_dir.join("0.2.8");
            if !v.is_dir() {
                continue;
            }
            let parsed = parse_wit_package(&v).unwrap();
            assert!(
                parsed.worlds.iter().all(|w| !w.trim().is_empty()),
                "empty world name in {}: {:?}",
                v.display(),
                parsed.worlds
            );
        }
    }

    #[test]
    fn wit_validate_summary_worlds_match_schema_constraints() {
        let index_path = repo_root().join("arch/wit/index.x07wit.json");
        let index_bytes = std::fs::read(&index_path).unwrap();
        let index: WitIndexDoc = serde_json::from_slice(&index_bytes).unwrap();
        let map: BTreeMap<String, WitIndexPackageRef> = package_map(&index);

        let args = WitValidateArgs {
            index: index_path.clone(),
            strict: false,
            package: Vec::new(),
            list: false,
        };

        let cli = map.get("wasi:cli@0.2.8").unwrap();
        let mut diags = Vec::new();
        let summary = validate_one_package(&args, Path::new("."), cli, &map, &mut diags);

        let worlds = summary
            .get("worlds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            worlds.iter().all(|v| !v.as_str().unwrap_or("").is_empty()),
            "unexpected worlds: {worlds:?}"
        );
    }
}

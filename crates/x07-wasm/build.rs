use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let ios_root = manifest_dir.join("src/support/mobile/ios/template");
    let android_root = manifest_dir.join("src/support/mobile/android/template");

    let ios_files = collect_files(&ios_root).expect("collect iOS template files");
    let android_files = collect_files(&android_root).expect("collect Android template files");

    for rel in ios_files.iter() {
        println!("cargo:rerun-if-changed={}", ios_root.join(rel).display());
    }
    for rel in android_files.iter() {
        println!(
            "cargo:rerun-if-changed={}",
            android_root.join(rel).display()
        );
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("x07_wasm_mobile_templates.rs");
    let mut f = fs::File::create(&out_path).expect("create template output file");

    write_template_module(
        &mut f,
        "IOS_TEMPLATE_FILES",
        "/src/support/mobile/ios/template/",
        &ios_files,
    )
    .expect("write iOS template module");
    write_template_module(
        &mut f,
        "ANDROID_TEMPLATE_FILES",
        "/src/support/mobile/android/template/",
        &android_files,
    )
    .expect("write Android template module");
}

fn collect_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("template root not found: {}", root.display()),
        ));
    }
    let mut out = Vec::new();
    collect_files_rec(root, PathBuf::new(), &mut out)?;
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(out)
}

fn collect_files_rec(root: &Path, rel: PathBuf, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let path = root.join(&rel);
    let mut entries = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let name = e.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".DS_Store" || name_str.starts_with("._") {
            continue;
        }

        let ty = e.file_type()?;
        let mut child_rel = rel.clone();
        child_rel.push(&name);
        if ty.is_dir() {
            collect_files_rec(root, child_rel, out)?;
        } else if ty.is_file() {
            out.push(child_rel);
        }
    }
    Ok(())
}

fn write_template_module(
    f: &mut fs::File,
    const_name: &str,
    cargo_manifest_rel_prefix: &str,
    rel_paths: &[PathBuf],
) -> std::io::Result<()> {
    writeln!(f, "pub(crate) const {const_name}: &[TemplateFile] = &[")?;
    for rel in rel_paths.iter() {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        writeln!(
            f,
            "  TemplateFile {{ path: {rel_str:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {cargo_manifest_rel_prefix:?}, {rel_str:?})) }},",
        )?;
    }
    writeln!(f, "];")?;
    writeln!(f)?;
    Ok(())
}

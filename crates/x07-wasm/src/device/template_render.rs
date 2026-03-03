use std::path::Path;

use anyhow::{Context, Result};

use crate::device::mobile_templates::TemplateFile;

#[derive(Debug, Clone)]
pub(crate) struct Replacement<'a> {
    pub(crate) needle: &'a str,
    pub(crate) value: String,
}

pub(crate) fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn render_template_dir(
    template_label: &str,
    files: &[TemplateFile],
    dst_root: &Path,
    replacements: &[Replacement<'_>],
) -> Result<()> {
    for f in files.iter() {
        let dst_path = dst_root.join(f.path);
        if let Some(parent) = dst_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("{template_label}: create dir: {}", parent.display()))?;
        }

        let mut out = f.bytes.to_vec();
        for r in replacements.iter() {
            out = replace_all_bytes(&out, r.needle.as_bytes(), r.value.as_bytes());
        }

        std::fs::write(&dst_path, out).with_context(|| {
            format!(
                "{template_label}: write template file: {}",
                dst_path.display()
            )
        })?;
    }
    Ok(())
}

fn replace_all_bytes(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return input.to_vec();
    }
    if input.len() < needle.len() {
        return input.to_vec();
    }

    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i + needle.len() <= input.len() {
        if &input[i..i + needle.len()] == needle {
            out.extend_from_slice(replacement);
            i += needle.len();
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out.extend_from_slice(&input[i..]);
    out
}

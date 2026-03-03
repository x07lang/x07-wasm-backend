#[derive(Debug, Clone, Copy)]
pub(crate) struct TemplateFile {
    pub(crate) path: &'static str,
    pub(crate) bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/x07_wasm_mobile_templates.rs"));

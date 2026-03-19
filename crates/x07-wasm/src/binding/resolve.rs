use anyhow::Result;

pub fn cmd_binding_resolve() -> Result<u8> {
    crate::cmdutil::emit_scaffold_report(
        "binding.resolve",
        "binding resolution scaffolding is wired; provider-neutral binding plans land on top of this command surface.",
    )
}

use anyhow::Result;

pub fn cmd_workload_inspect() -> Result<u8> {
    crate::cmdutil::emit_scaffold_report(
        "workload.inspect",
        "workload inspect scaffolding is wired; deterministic inspection output lands on top of this command surface.",
    )
}

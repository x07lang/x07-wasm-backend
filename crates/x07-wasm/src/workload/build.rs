use anyhow::Result;

pub fn cmd_workload_build() -> Result<u8> {
    crate::cmdutil::emit_scaffold_report(
        "workload.build",
        "workload build scaffolding is wired; artifact packing lands on top of this command surface.",
    )
}

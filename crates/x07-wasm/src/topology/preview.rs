use anyhow::Result;

pub fn cmd_topology_preview() -> Result<u8> {
    crate::cmdutil::emit_scaffold_report(
        "topology.preview",
        "topology preview scaffolding is wired; workload grouping and placement previews land on top of this command surface.",
    )
}

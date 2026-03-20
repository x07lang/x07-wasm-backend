use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct TopologyArgs {
    #[command(subcommand)]
    pub cmd: Option<TopologyCommand>,
}

#[derive(Debug, Clone, Args)]
pub struct TopologyPreviewArgs {
    #[arg(long, default_value = "x07.json")]
    pub project: PathBuf,

    #[arg(long, default_value = "arch/service/index.x07service.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub pack_manifest: Option<PathBuf>,

    #[arg(long)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TopologyCommand {
    Preview(TopologyPreviewArgs),
}

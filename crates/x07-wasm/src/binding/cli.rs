use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct BindingArgs {
    #[command(subcommand)]
    pub cmd: Option<BindingCommand>,
}

#[derive(Debug, Clone, Args)]
pub struct BindingResolveArgs {
    #[arg(long, default_value = "x07.json")]
    pub project: PathBuf,

    #[arg(long, default_value = "arch/service/index.x07service.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub pack_manifest: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BindingCommand {
    Resolve(BindingResolveArgs),
}

use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct TopologyArgs {
    #[command(subcommand)]
    pub cmd: Option<TopologyCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TopologyCommand {
    Preview,
}

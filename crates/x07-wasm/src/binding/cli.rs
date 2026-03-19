use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct BindingArgs {
    #[command(subcommand)]
    pub cmd: Option<BindingCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BindingCommand {
    Resolve,
}

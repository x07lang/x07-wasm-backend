use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct WorkloadArgs {
    #[command(subcommand)]
    pub cmd: Option<WorkloadCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WorkloadCommand {
    Build,
    Pack,
    Inspect,
    #[command(name = "contracts-validate")]
    ContractsValidate,
}

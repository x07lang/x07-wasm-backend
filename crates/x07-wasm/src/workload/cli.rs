use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub struct WorkloadArgs {
    #[command(subcommand)]
    pub cmd: Option<WorkloadCommand>,
}

#[derive(Debug, Clone, Args)]
pub struct WorkloadBuildArgs {
    #[arg(long, default_value = "x07.json")]
    pub project: PathBuf,

    #[arg(long, default_value = "arch/service/index.x07service.json")]
    pub manifest: PathBuf,

    #[arg(long, default_value = "dist/workload-build")]
    pub out_dir: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct WorkloadPackArgs {
    #[arg(long, default_value = "x07.json")]
    pub project: PathBuf,

    #[arg(long, default_value = "arch/service/index.x07service.json")]
    pub manifest: PathBuf,

    #[arg(long, default_value = "dist/workload")]
    pub out_dir: PathBuf,

    #[arg(long)]
    pub runtime_image: Option<String>,

    #[arg(long, default_value_t = 8080)]
    pub container_port: u16,
}

#[derive(Debug, Clone, Args)]
pub struct WorkloadInspectArgs {
    #[arg(long, default_value = "dist/workload/workload.pack.json")]
    pub pack_manifest: PathBuf,

    #[arg(long, default_value = "full")]
    pub view: String,
}

#[derive(Debug, Clone, Args)]
pub struct WorkloadContractsValidateArgs {
    #[arg(long, default_value = "x07.json")]
    pub project: PathBuf,

    #[arg(long, default_value = "arch/service/index.x07service.json")]
    pub manifest: PathBuf,

    #[arg(long)]
    pub pack_manifest: Option<PathBuf>,

    #[arg(long)]
    pub profile: Option<String>,

    #[arg(long)]
    pub schema_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WorkloadCommand {
    Build(WorkloadBuildArgs),
    Pack(WorkloadPackArgs),
    Inspect(WorkloadInspectArgs),
    #[command(name = "contracts-validate")]
    ContractsValidate(WorkloadContractsValidateArgs),
}

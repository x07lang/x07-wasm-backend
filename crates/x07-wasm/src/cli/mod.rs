use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
#[command(name = "x07-wasm")]
#[command(version)]
#[command(
    about = "x07-wasm: build x07 solve-pure programs to wasm32 and run them deterministically (Phase 0)."
)]
pub struct RootCli {
    /// Emit deterministic CLI surface table for agents (x07cli.specrows@0.1.0).
    #[arg(long, global = true)]
    pub cli_specrows: bool,

    #[command(flatten)]
    pub machine: MachineArgs,

    #[command(subcommand)]
    pub cmd: Option<Command>,
}

#[derive(Debug, Clone, Args)]
pub struct MachineArgs {
    /// Emit command report JSON to stdout (values: \"\" or \"pretty\").
    #[arg(
        long,
        global = true,
        num_args(0..=1),
        default_missing_value = "",
        value_name = "MODE"
    )]
    pub json: Option<String>,

    /// Hidden alias for --json.
    #[arg(
        long,
        global = true,
        hide = true,
        num_args(0..=1),
        default_missing_value = "",
        value_name = "MODE"
    )]
    pub report_json: Option<String>,

    /// Write the JSON report to a file (in addition to stdout unless `--quiet-json` is set).
    #[arg(long, global = true, value_name = "PATH")]
    pub report_out: Option<PathBuf>,

    /// Suppress JSON on stdout (use with --report-out).
    #[arg(long, global = true, requires = "report_out")]
    pub quiet_json: bool,

    /// Print the JSON Schema for the selected command scope and exit 0.
    #[arg(long, global = true)]
    pub json_schema: bool,

    /// Print the schema id/version string for the selected command scope and exit 0.
    #[arg(long, global = true)]
    pub json_schema_id: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Build an x07 project to a wasm32 reactor module (exports x07_solve_v2).
    Build(BuildArgs),
    /// Run a wasm module exporting x07_solve_v2 under Wasmtime; emit output bytes + JSON report.
    Run(RunArgs),
    /// Check wasm toolchain prerequisites and emit a machine report.
    Doctor(DoctorArgs),

    /// Validate arch/wasm/index.x07wasm.json and referenced profile files.
    Profile(ProfileArgs),
    /// Alias for `x07-wasm profile validate`.
    #[command(name = "profile-validate")]
    ProfileValidate(ProfileValidateArgs),

    /// CLI discovery tooling.
    Cli(CliArgs),
    /// Alias for `x07-wasm cli specrows check`.
    #[command(name = "cli-specrows-check")]
    CliSpecrowsCheck(CliSpecrowsCheckArgs),
}

impl Command {
    pub fn run(self, raw_argv: &[OsString], scope: Scope, machine: &MachineArgs) -> Result<u8> {
        match self {
            Command::Build(args) => crate::wasm::build::cmd_build(raw_argv, scope, machine, args),
            Command::Run(args) => crate::wasm::run::cmd_run(raw_argv, scope, machine, args),
            Command::Doctor(args) => crate::toolchain::cmd_doctor(raw_argv, scope, machine, args),
            Command::Profile(args) => match args.cmd {
                ProfileCommand::Validate(v) => {
                    crate::arch::cmd_profile_validate(raw_argv, scope, machine, v)
                }
            },
            Command::ProfileValidate(v) => {
                crate::arch::cmd_profile_validate(raw_argv, scope, machine, v)
            }
            Command::Cli(args) => match args.cmd {
                CliCommand::Specrows(spec) => match spec.cmd {
                    CliSpecrowsCommand::Check(v) => {
                        crate::cli::specrows::cmd_cli_specrows_check(raw_argv, scope, machine, v)
                    }
                },
                CliCommand::ValidateSpecrows(v) => {
                    crate::cli::specrows::cmd_cli_specrows_check(raw_argv, scope, machine, v)
                }
            },
            Command::CliSpecrowsCheck(v) => {
                crate::cli::specrows::cmd_cli_specrows_check(raw_argv, scope, machine, v)
            }
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct BuildArgs {
    /// Path to x07 project manifest.
    #[arg(long, value_name = "PATH", default_value = "x07.json")]
    pub project: PathBuf,

    /// Profile id (loaded from arch/wasm/index.x07wasm.json).
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,

    /// Validate and use this profile JSON file directly (bypass registry).
    #[arg(long, value_name = "PATH", conflicts_with = "profile")]
    pub profile_file: Option<PathBuf>,

    /// Path to wasm profile registry.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/index.x07wasm.json"
    )]
    pub index: PathBuf,

    /// Directory for intermediate artifacts.
    #[arg(long, value_name = "DIR")]
    pub emit_dir: Option<PathBuf>,

    /// Output wasm path.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Artifact manifest output path.
    #[arg(long, value_name = "PATH")]
    pub artifact_out: Option<PathBuf>,

    /// Do not write the artifact manifest file.
    #[arg(long)]
    pub no_manifest: bool,

    /// Validate required exports exist (set false to disable).
    #[arg(long, value_name = "BOOL", default_value_t = true, action = clap::ArgAction::Set)]
    pub check_exports: bool,
}

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Path to wasm module.
    #[arg(long, value_name = "PATH")]
    pub wasm: PathBuf,

    /// Input bytes file path.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["input_hex", "input_base64"])]
    pub input: Option<PathBuf>,

    /// Input bytes as hex.
    #[arg(long, value_name = "HEX", conflicts_with_all = ["input", "input_base64"])]
    pub input_hex: Option<String>,

    /// Input bytes as base64.
    #[arg(long, value_name = "B64", conflicts_with_all = ["input", "input_hex"])]
    pub input_base64: Option<String>,

    /// Profile id (for defaults like arena/max-output).
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,

    /// Validate and use this profile JSON file directly (bypass registry).
    #[arg(long, value_name = "PATH", conflicts_with = "profile")]
    pub profile_file: Option<PathBuf>,

    /// Path to wasm profile registry.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/index.x07wasm.json"
    )]
    pub index: PathBuf,

    /// Arena capacity passed to x07_solve_v2 (bytes).
    #[arg(long, value_name = "N")]
    pub arena_cap_bytes: Option<u64>,

    /// Hard cap enforced on returned bytes_t.len.
    #[arg(long, value_name = "N")]
    pub max_output_bytes: Option<u64>,

    /// Write output bytes to a file.
    #[arg(long, value_name = "PATH")]
    pub output_out: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub cmd: ProfileCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProfileCommand {
    Validate(ProfileValidateArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ProfileValidateArgs {
    /// Path to wasm profile registry (default: arch/wasm/index.x07wasm.json).
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/index.x07wasm.json"
    )]
    pub index: PathBuf,

    /// Validate only this profile id (looked up in the registry).
    #[arg(long, value_name = "ID", conflicts_with = "profile_file")]
    pub profile: Option<String>,

    /// Validate a profile JSON file directly (bypass registry).
    #[arg(long, value_name = "PATH", conflicts_with = "profile")]
    pub profile_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct CliArgs {
    #[command(subcommand)]
    pub cmd: CliCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CliCommand {
    Specrows(CliSpecrowsArgs),
    /// Alias for `x07-wasm cli specrows check`.
    #[command(name = "validate-specrows")]
    ValidateSpecrows(CliSpecrowsCheckArgs),
}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct CliSpecrowsArgs {
    #[command(subcommand)]
    pub cmd: CliSpecrowsCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CliSpecrowsCommand {
    Check(CliSpecrowsCheckArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CliSpecrowsCheckArgs {
    /// Read specrows JSON from file (mutually exclusive with --stdin; default is self).
    #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
    pub r#in: Option<PathBuf>,

    /// Read specrows JSON from stdin (mutually exclusive with --in).
    #[arg(long)]
    pub stdin: bool,

    /// Expected app.name (default: x07-wasm).
    #[arg(long, value_name = "STR", default_value = "x07-wasm")]
    pub expect_app_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Build,
    Run,
    Doctor,
    ProfileValidate,
    CliSpecrowsCheck,
}

pub fn scope_for_command(cmd: Option<&Command>) -> Scope {
    match cmd {
        Some(Command::Build(_)) => Scope::Build,
        Some(Command::Run(_)) => Scope::Run,
        Some(Command::Doctor(_)) => Scope::Doctor,
        Some(Command::Profile(_)) => Scope::ProfileValidate,
        Some(Command::ProfileValidate(_)) => Scope::ProfileValidate,
        Some(Command::Cli(_)) => Scope::CliSpecrowsCheck,
        Some(Command::CliSpecrowsCheck(_)) => Scope::CliSpecrowsCheck,
        None => Scope::Doctor,
    }
}

pub mod specrows;

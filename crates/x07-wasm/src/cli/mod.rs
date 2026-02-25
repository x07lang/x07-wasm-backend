use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Run a wasi:http/proxy component as a local canary (Phase 1).
    Serve(ServeArgs),
    /// Check wasm toolchain prerequisites and emit a machine report.
    Doctor(DoctorArgs),

    /// WIT tooling (Phase 1).
    Wit(WitArgs),
    /// Alias for `x07-wasm wit validate`.
    #[command(name = "wit-validate")]
    WitValidate(WitValidateArgs),

    /// Component tooling (Phase 1).
    Component(ComponentArgs),
    /// Alias for `x07-wasm component profile validate`.
    #[command(name = "component-profile-validate")]
    ComponentProfileValidate(ComponentProfileValidateArgs),
    /// Alias for `x07-wasm component build`.
    #[command(name = "component-build")]
    ComponentBuild(ComponentBuildArgs),
    /// Alias for `x07-wasm component compose`.
    #[command(name = "component-compose")]
    ComponentCompose(ComponentComposeArgs),
    /// Alias for `x07-wasm component targets`.
    #[command(name = "component-targets")]
    ComponentTargets(ComponentTargetsArgs),

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
            Command::Serve(args) => crate::serve::cmd_serve(raw_argv, scope, machine, args),
            Command::Doctor(args) => crate::toolchain::cmd_doctor(raw_argv, scope, machine, args),
            Command::Wit(args) => match args.cmd {
                WitCommand::Validate(v) => {
                    crate::wit::validate::cmd_wit_validate(raw_argv, scope, machine, v)
                }
            },
            Command::WitValidate(v) => {
                crate::wit::validate::cmd_wit_validate(raw_argv, scope, machine, v)
            }
            Command::Component(args) => match args.cmd {
                ComponentCommand::Profile(p) => match p.cmd {
                    ComponentProfileCommand::Validate(v) => {
                        crate::component::profile_validate::cmd_component_profile_validate(
                            raw_argv, scope, machine, v,
                        )
                    }
                },
                ComponentCommand::Build(v) => {
                    crate::component::build::cmd_component_build(raw_argv, scope, machine, v)
                }
                ComponentCommand::Compose(v) => {
                    crate::component::compose::cmd_component_compose(raw_argv, scope, machine, v)
                }
                ComponentCommand::Targets(v) => {
                    crate::component::targets::cmd_component_targets(raw_argv, scope, machine, v)
                }
                ComponentCommand::Run(v) => {
                    crate::component::run::cmd_component_run(raw_argv, scope, machine, v)
                }
            },
            Command::ComponentProfileValidate(v) => {
                crate::component::profile_validate::cmd_component_profile_validate(
                    raw_argv, scope, machine, v,
                )
            }
            Command::ComponentBuild(v) => {
                crate::component::build::cmd_component_build(raw_argv, scope, machine, v)
            }
            Command::ComponentCompose(v) => {
                crate::component::compose::cmd_component_compose(raw_argv, scope, machine, v)
            }
            Command::ComponentTargets(v) => {
                crate::component::targets::cmd_component_targets(raw_argv, scope, machine, v)
            }
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ServeMode {
    Canary,
    Listen,
}

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    /// Path to HTTP component (.wasm).
    #[arg(long, value_name = "PATH")]
    pub component: PathBuf,

    /// Listen address for mode=listen (e.g., 127.0.0.1:8080).
    #[arg(long, value_name = "STR", default_value = "127.0.0.1:0")]
    pub addr: String,

    /// Mode: canary|listen.
    #[arg(long, value_name = "STR", default_value = "canary")]
    pub mode: ServeMode,

    /// Stop after N requests (canary mode; or listen mode if nonzero).
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub stop_after: u32,

    /// Request body bytes for canary mode (hex:, b64:, @path).
    #[arg(long, value_name = "BYTES", default_value = "")]
    pub request_body: String,

    /// Request method for canary mode.
    #[arg(long, value_name = "STR", default_value = "POST")]
    pub method: String,

    /// Request path for canary mode.
    #[arg(long, value_name = "STR", default_value = "/")]
    pub path: String,

    /// Hard cap on request bytes (body + headers).
    #[arg(long, value_name = "N", default_value_t = 1024 * 1024)]
    pub max_request_bytes: u64,

    /// Hard cap on response body bytes.
    #[arg(long, value_name = "N", default_value_t = 1024 * 1024)]
    pub max_response_bytes: u64,

    /// Hard cap on wall time spent per request (ms).
    #[arg(long, value_name = "N", default_value_t = 5_000)]
    pub max_wall_ms_per_request: u64,

    /// Hard cap on concurrent request handling.
    #[arg(long, value_name = "N", default_value_t = 16)]
    pub max_concurrent: u32,

    /// Root directory for incident bundles.
    #[arg(long, value_name = "DIR", default_value = ".x07-wasm/incidents")]
    pub incidents_dir: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct WitArgs {
    #[command(subcommand)]
    pub cmd: WitCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WitCommand {
    Validate(WitValidateArgs),
}

#[derive(Debug, Clone, Args)]
pub struct WitValidateArgs {
    /// Path to the WIT registry file.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wit/index.x07wit.json"
    )]
    pub index: PathBuf,

    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,

    /// Only validate specific package id(s), e.g. wasi:http@0.2.8.
    #[arg(long, value_name = "STR")]
    pub package: Vec<String>,

    /// List packages discovered in the registry and exit (still emits a report).
    #[arg(long)]
    pub list: bool,
}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct ComponentArgs {
    #[command(subcommand)]
    pub cmd: ComponentCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ComponentCommand {
    Profile(ComponentProfileArgs),
    Build(ComponentBuildArgs),
    Compose(ComponentComposeArgs),
    Targets(ComponentTargetsArgs),
    Run(ComponentRunArgs),
}

#[derive(Debug, Clone, Args)]
#[command(subcommand_required = true)]
pub struct ComponentProfileArgs {
    #[command(subcommand)]
    pub cmd: ComponentProfileCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ComponentProfileCommand {
    Validate(ComponentProfileValidateArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ComponentProfileValidateArgs {
    /// Path to the component profile registry.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/component/index.x07wasm.component.json"
    )]
    pub index: PathBuf,

    /// Only validate specific profile id(s).
    #[arg(long, value_name = "ID")]
    pub profile: Vec<String>,

    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ComponentBuildEmit {
    Solve,
    #[value(name = "http-adapter")]
    HttpAdapter,
    #[value(name = "cli-adapter")]
    CliAdapter,
    All,
}

#[derive(Debug, Clone, Args)]
pub struct ComponentBuildArgs {
    /// Path to x07 project manifest.
    #[arg(long, value_name = "PATH", default_value = "x07.json")]
    pub project: PathBuf,

    /// Component profile id (loaded from arch/wasm/component/index.x07wasm.component.json).
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,

    /// Validate and use this component profile JSON file directly (bypass registry).
    #[arg(long, value_name = "PATH", conflicts_with = "profile")]
    pub profile_file: Option<PathBuf>,

    /// Path to the component profile registry.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/component/index.x07wasm.component.json"
    )]
    pub index: PathBuf,

    /// WASM profile id (loaded from arch/wasm/index.x07wasm.json).
    #[arg(long, value_name = "ID")]
    pub wasm_profile: Option<String>,

    /// Validate and use this wasm profile JSON file directly (bypass registry).
    #[arg(long, value_name = "PATH", conflicts_with = "wasm_profile")]
    pub wasm_profile_file: Option<PathBuf>,

    /// Path to wasm profile registry.
    #[arg(
        long,
        value_name = "PATH",
        default_value = "arch/wasm/index.x07wasm.json"
    )]
    pub wasm_index: PathBuf,

    /// Output directory for component artifacts.
    #[arg(long, value_name = "DIR", default_value = "target/x07-wasm/component")]
    pub out_dir: PathBuf,

    /// Artifact set to emit.
    #[arg(long, value_name = "SET", default_value = "all")]
    pub emit: ComponentBuildEmit,

    /// Delete out-dir before building.
    #[arg(long)]
    pub clean: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ComponentComposeAdapterKind {
    Http,
    Cli,
}

#[derive(Debug, Clone, Args)]
pub struct ComponentComposeArgs {
    /// Adapter kind: http|cli.
    #[arg(long, value_name = "KIND", alias = "target")]
    pub adapter: ComponentComposeAdapterKind,

    /// Path to solve component (.wasm).
    #[arg(long, value_name = "PATH")]
    pub solve: PathBuf,

    /// Path to adapter component (.wasm).
    #[arg(long, value_name = "PATH")]
    pub adapter_component: Option<PathBuf>,

    /// Output path for composed component (.wasm).
    #[arg(long, value_name = "PATH")]
    pub out: PathBuf,

    /// Artifact manifest output path.
    #[arg(long, value_name = "PATH")]
    pub artifact_out: Option<PathBuf>,

    /// Also run a targets check on the output component.
    #[arg(long)]
    pub targets_check: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ComponentTargetsArgs {
    /// Path to component wasm to check.
    #[arg(long, value_name = "PATH")]
    pub component: PathBuf,

    /// Path to a .wit file containing the world to target.
    #[arg(long, value_name = "PATH")]
    pub wit: PathBuf,

    /// World name within the WIT file.
    #[arg(long, value_name = "STR")]
    pub world: String,

    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ComponentRunArgs {
    /// Path to component wasm to run.
    #[arg(long, value_name = "PATH")]
    pub component: PathBuf,

    /// Process args as JSON array of strings.
    #[arg(long, value_name = "JSON", default_value = "[]")]
    pub args_json: String,

    /// Stdin bytes file path.
    #[arg(long, value_name = "PATH", conflicts_with = "stdin_b64")]
    pub stdin: Option<PathBuf>,

    /// Stdin bytes as base64.
    #[arg(long, value_name = "B64", conflicts_with = "stdin")]
    pub stdin_b64: Option<String>,

    /// Write stdout bytes to a file.
    #[arg(long, value_name = "PATH")]
    pub stdout_out: Option<PathBuf>,

    /// Write stderr bytes to a file.
    #[arg(long, value_name = "PATH")]
    pub stderr_out: Option<PathBuf>,

    /// Hard cap on stdout/stderr bytes captured by the host.
    #[arg(long, value_name = "N", default_value_t = 16 * 1024 * 1024)]
    pub max_output_bytes: u64,

    /// Hard cap on wall time spent running the component (ms).
    #[arg(long, value_name = "N")]
    pub max_wall_ms: Option<u64>,

    /// Root directory for incident bundles.
    #[arg(long, value_name = "DIR", default_value = ".x07-wasm/incidents")]
    pub incidents_dir: PathBuf,
}

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
    Serve,
    Doctor,
    WitValidate,
    ComponentProfileValidate,
    ComponentBuild,
    ComponentCompose,
    ComponentTargets,
    ComponentRun,
    ProfileValidate,
    CliSpecrowsCheck,
}

pub fn scope_for_command(cmd: Option<&Command>) -> Scope {
    match cmd {
        Some(Command::Build(_)) => Scope::Build,
        Some(Command::Run(_)) => Scope::Run,
        Some(Command::Serve(_)) => Scope::Serve,
        Some(Command::Doctor(_)) => Scope::Doctor,
        Some(Command::Wit(_)) => Scope::WitValidate,
        Some(Command::WitValidate(_)) => Scope::WitValidate,
        Some(Command::Component(args)) => match args.cmd {
            ComponentCommand::Profile(_) => Scope::ComponentProfileValidate,
            ComponentCommand::Build(_) => Scope::ComponentBuild,
            ComponentCommand::Compose(_) => Scope::ComponentCompose,
            ComponentCommand::Targets(_) => Scope::ComponentTargets,
            ComponentCommand::Run(_) => Scope::ComponentRun,
        },
        Some(Command::ComponentProfileValidate(_)) => Scope::ComponentProfileValidate,
        Some(Command::ComponentBuild(_)) => Scope::ComponentBuild,
        Some(Command::ComponentCompose(_)) => Scope::ComponentCompose,
        Some(Command::ComponentTargets(_)) => Scope::ComponentTargets,
        Some(Command::Profile(_)) => Scope::ProfileValidate,
        Some(Command::ProfileValidate(_)) => Scope::ProfileValidate,
        Some(Command::Cli(_)) => Scope::CliSpecrowsCheck,
        Some(Command::CliSpecrowsCheck(_)) => Scope::CliSpecrowsCheck,
        None => Scope::Doctor,
    }
}

pub mod specrows;

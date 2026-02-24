mod arch;
mod cli;
mod diag;
mod report;
mod schema;
mod toolchain;
mod util;
mod wasm;

use std::io::Write as _;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser as _;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8> {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let root = cli::RootCli::parse();

    if root.cli_specrows {
        let doc = cli::specrows::build_specrows_doc();
        let bytes = report::canon::canonical_json_bytes(&doc)?;
        std::io::stdout().write_all(&bytes)?;
        return Ok(0);
    }

    let scope = cli::scope_for_command(root.cmd.as_ref());

    if root.machine.json_schema || root.machine.json_schema_id {
        let out = report::schema::emit_schema_or_id(&root.machine, scope)?;
        return Ok(out);
    }

    if let Some(msg) = report::machine::validate_machine_args(&root.machine) {
        eprintln!("{msg}");
        return Ok(3);
    }

    match root.cmd {
        Some(cmd) => cmd.run(&argv, scope, &root.machine),
        None => {
            eprintln!("missing command (try --help)");
            Ok(3)
        }
    }
}

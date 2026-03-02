#![recursion_limit = "1024"]

mod app;
mod arch;
mod blob;
mod caps;
mod cli;
mod cmdutil;
mod component;
mod deploy;
mod device;
mod diag;
mod guest_diag;
mod http_component_host;
mod http_reducer;
mod json_doc;
mod ops;
mod policy;
mod provenance;
mod report;
mod schema;
mod serve;
mod slo;
mod stream_payload;
mod toolchain;
mod util;
mod wasm;
mod wasmtime_limits;
mod web_ui;
mod wit;

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
    let started = std::time::Instant::now();
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let hint = report::cli_parse::MachineHint::from_argv(&argv);

    let root = match cli::RootCli::try_parse_from(&argv) {
        Ok(v) => v,
        Err(err) => {
            if err.exit_code() == 0 {
                let _ = err.print();
                return Ok(0);
            }
            let _ = err.print();
            return report::cli_parse::emit_cli_parse_report(
                &argv,
                &hint,
                started,
                "clap",
                err.to_string(),
                3,
            );
        }
    };

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
        return report::cli_parse::emit_cli_parse_report(
            &argv,
            &hint,
            started,
            "machine_args",
            msg,
            3,
        );
    }

    match root.cmd {
        Some(cmd) => cmd.run(&argv, scope, &root.machine),
        None => {
            let msg = "missing command (try --help)".to_string();
            eprintln!("{msg}");
            report::cli_parse::emit_cli_parse_report(
                &argv,
                &hint,
                started,
                "missing_command",
                msg,
                3,
            )
        }
    }
}

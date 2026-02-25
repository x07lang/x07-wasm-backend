wit_bindgen::generate!({
    world: "x07:cli-adapter/command-with-solve@0.1.0",
    path: [
        "../../wit/deps/wasi/io/0.2.8",
        "../../wit/deps/wasi/random/0.2.8",
        "../../wit/deps/wasi/clocks/0.2.8",
        "../../wit/deps/wasi/sockets/0.2.8",
        "../../wit/deps/wasi/filesystem/0.2.8",
        "../../wit/deps/wasi/cli/0.2.8",
        "../../wit/x07/solve/0.1.0",
        "../../wit/x07/cli_adapter/0.1.0",
    ],
    generate_all,
});

struct CliAdapter;

impl exports::wasi::cli::run::Guest for CliAdapter {
    fn run() -> Result<(), ()> {
        let input = read_all_stdin();
        let output = x07::solve::handler::solve(&input);
        write_all_stdout(&output);
        Ok(())
    }
}

export!(CliAdapter);

fn read_all_stdin() -> Vec<u8> {
    let stdin = wasi::cli::stdin::get_stdin();
    let mut out = Vec::new();
    loop {
        match stdin.blocking_read(4096) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    break;
                }
                out.extend_from_slice(&chunk);
            }
            Err(wasi::io::streams::StreamError::Closed) => break,
            Err(wasi::io::streams::StreamError::LastOperationFailed(e)) => {
                panic!("stdin read failed: {}", e.to_debug_string());
            }
        }
    }
    out
}

fn write_all_stdout(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let stdout = wasi::cli::stdout::get_stdout();
    for chunk in bytes.chunks(4096) {
        stdout
            .blocking_write_and_flush(chunk)
            .expect("stdout write failed");
    }
}


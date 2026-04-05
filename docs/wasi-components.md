# WASI 0.2 components

This guide covers a **component build + compose + run** pipeline on top of solve-pure WASM modules.

Targets:

- `wasi:http/proxy` (HTTP handler component)
- `wasi:cli/command` (CLI batch/worker component)

## WIT (offline, pinned)

This repo vendors and pins WIT packages under `wit/` and declares them in:

- `arch/wit/index.x07wit.json`

Validate everything offline:

```sh
x07-wasm wit validate --json
```

## Component profiles

Component profiles live under:

- `arch/wasm/component/index.x07wasm.component.json`
- `arch/wasm/component/profiles/*.json`

Validate:

```sh
x07-wasm component profile validate --json
```

## Component pipeline

Build solve and runnable targets:

```sh
x07-wasm component build --project examples/http_echo/x07.json --emit all --json
```

Build only the runnable native HTTP target:

```sh
x07-wasm component build --project examples/http_echo/x07.json --emit http --json
```

Build only the runnable native CLI target:

```sh
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit cli --json
```

Legacy compose path (adapters + `wac plug`):

```sh
x07-wasm component build --project examples/http_echo/x07.json --emit solve --json
x07-wasm component build --project examples/solve_pure_echo/x07.json --emit http-adapter --json
```

Compose a runnable target (via `wac plug`):

```sh
x07-wasm component compose --adapter http --solve target/x07-wasm/component/solve.component.wasm --out dist/app.http.component.wasm --json
```

Validate standard-world targeting:

```sh
x07-wasm component targets --component dist/app.http.component.wasm --wit wit/deps/wasi/http/0.2.8/proxy.wit --world proxy --json
```

## Run

Canary-run an HTTP component (no network; the host drives a synthetic request):

```sh
x07-wasm serve --mode canary --component dist/app.http.component.wasm --request-body @examples/http_echo/tests/fixtures/request_body.bin --json
```

Run a CLI component:

```sh
x07-wasm component run --component dist/app.cli.component.wasm --stdin @examples/solve_pure_echo/tests/fixtures/in_hello.bin --stdout-out dist/stdout.bin --json
```

## Native backend targets

Native backend targets, budgets, and diagnostic channels are documented in:

- `docs/native-component-targets.md`

## Working with x07 programs (x07AST)

The examples in this document are x07AST JSON (`*.x07.json`). For safe editing:

- `x07 fmt --check|--write <path>`
- `x07 lint --input <path>`
- `x07 fix --input <path> --write`

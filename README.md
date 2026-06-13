# x07-wasm-backend

WebAssembly build and packaging pipeline for X07.

This repo contains the `x07-wasm` toolchain, host runners, schemas, examples, and release tooling used to turn X07 projects into WASM modules, WASI components, and workload packs.

**Start here:** [`examples/`](examples/) · [Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart)

> The browser web-ui, device-host, and full-stack app-bundle surfaces were
> archived in the 2026-06 refocus on the core deterministic execution substrate
> and have been removed from this repo.

## What This Repo Builds

- solve-pure WASM modules
- WASI components
- workload packs and related deploy inputs

If `x07` is where you write the program, `x07-wasm-backend` is where that program becomes a runnable WASM artifact.

## Quick Start

Install the released component:

```sh
x07up component add wasm
x07 wasm doctor --json
```

Build and run a simple WASM artifact:

```sh
x07-wasm build \
  --project examples/solve_pure_echo/x07.json \
  --profile wasm_release \
  --out dist/echo.wasm \
  --artifact-out dist/echo.wasm.manifest.json \
  --json

x07-wasm run \
  --wasm dist/echo.wasm \
  --input examples/solve_pure_echo/tests/fixtures/in_hello.bin \
  --output-out dist/out.bin \
  --json
```

## Common Paths

### Workload and backend delivery

```sh
x07-wasm workload build
x07-wasm workload inspect
x07-wasm topology preview
x07-wasm deploy plan
```

## Where To Look Next

- `examples/` for end-to-end sample projects

## How It Fits The X07 Ecosystem

- [`x07`](https://github.com/x07lang/x07) provides the language, repair loop, and package workflows
- `x07-wasm-backend` builds the runnable WASM and packaging outputs

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE).

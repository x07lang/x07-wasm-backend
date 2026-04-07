# x07-wasm-backend

WebAssembly build and packaging pipeline for X07.

This repo contains the `x07-wasm` toolchain, host runners, schemas, examples, and release tooling used to turn X07 projects into WASM modules, WASI components, browser UI apps, device bundles, and workload packs.

**Start here:** [`examples/`](examples/) · [`x07lang/x07-web-ui`](https://github.com/x07lang/x07-web-ui) · [`x07lang/x07-device-host`](https://github.com/x07lang/x07-device-host) · [Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart)

## What This Repo Builds

- solve-pure WASM modules
- WASI components
- browser UI apps
- desktop and mobile device bundles
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

### Browser UI apps

```sh
x07-wasm web-ui build
x07-wasm web-ui serve
x07-wasm web-ui test
```

### Device apps

```sh
x07-wasm device build
x07-wasm device verify
x07-wasm device package --target ios
x07-wasm device package --target android
```

### Workload and backend delivery

```sh
x07-wasm workload build
x07-wasm workload inspect
x07-wasm topology preview
x07-wasm deploy plan
```

## Where To Look Next

- `examples/` for end-to-end sample projects
- [`x07lang/x07-web-ui`](https://github.com/x07lang/x07-web-ui) for reducer-side UI contracts
- [`x07lang/x07-device-host`](https://github.com/x07lang/x07-device-host) for native device shells
- [`x07lang/x07-platform`](https://github.com/x07lang/x07-platform) for deploy, release, incident, and regression workflows built around the artifacts produced here

## How It Fits The X07 Ecosystem

- [`x07`](https://github.com/x07lang/x07) provides the language, repair loop, and package workflows
- `x07-wasm-backend` builds the runnable WASM and packaging outputs
- `x07-web-ui` and `x07-device-host` provide the app-side browser and device surfaces
- `x07-platform` consumes workload, app, and device artifacts when they move into operational flows

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE).

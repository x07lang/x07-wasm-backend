# WASM Phase 2 (Web UI)

Phase 2 adds a Web UI loop for **solve-pure** X07 programs as deterministic reducers that emit `x07.web_ui.*` frames.

This repo implements the `x07-wasm web-ui ...` toolchain. The canonical `std.web_ui.*` X07 package, browser host assets, and WIT contracts live in the `x07-web-ui` repo and are vendored/synced here.

## Contracts

- Dispatch envelope (input): `x07.web_ui.dispatch@0.1.0` (UTF-8 JSON bytes)
- Frame (output): `x07.web_ui.frame@0.1.0` (UTF-8 JSON bytes)
- Trace artifact: `x07.web_ui.trace@0.1.0` (dispatch+frame steps)

## Registries (contracts-as-data)

Web UI profiles:

- Index: `arch/web_ui/index.x07webui.json`
- Profiles: `arch/web_ui/profiles/*.json`

WASM profiles (used by web-ui profiles via `wasm_profile_id`):

- Index: `arch/wasm/index.x07wasm.json`
- Profiles: `arch/wasm/profiles/*.json`

Validate (offline, tool self-validation):

```sh
x07-wasm web-ui contracts validate --json
x07-wasm web-ui profile validate --json
```

## Build

Core wasm bundle (browser runs `app.wasm` directly):

```sh
x07-wasm web-ui build --project ./x07.json --profile web_ui_debug --out-dir dist --json
```

Outputs (minimum):

- `dist/app.wasm`
- `dist/app.wasm.manifest.json`
- `dist/web-ui.profile.json`
- `dist/index.html`
- `dist/app-host.mjs`

Component bundle (browser runs transpiled ESM; `app.wasm` is still emitted as a core fallback):

```sh
x07-wasm web-ui build --project ./x07.json --profile web_ui_debug --out-dir dist --format component --json
```

Additional outputs:

- `dist/app.component.wasm`
- `dist/transpiled/app.mjs`
- `dist/transpiled/*.core.wasm`

## Serve

Static dev server with correct `.wasm` MIME:

```sh
x07-wasm web-ui serve --dir dist --mode listen --strict-mime --json
```

CI smoke (one request; verifies `application/wasm` for `/app.wasm`):

```sh
x07-wasm web-ui serve --dir dist --mode smoke --strict-mime --json
```

## Test (trace replay)

Replays traces under Wasmtime (core wasm). If `dist/transpiled/app.mjs` is present, it also replays the trace under Node against the transpiled component output.

```sh
x07-wasm web-ui test --dist-dir dist --case ./tests/cases/counter.trace.json --json
```

Snapshots are written to `dist/test_snapshots/*.ui.json`.

## Incident → regression

`x07-wasm web-ui regress-from-incident` converts a `x07.web_ui.incident` artifact into:

- a deterministic trace fixture (`*.trace.json`)
- a final UI snapshot (`*.final.ui.json`)

```sh
x07-wasm web-ui regress-from-incident --incident incident.json --out-dir tests/regress --name incident --json
```

## Vendoring

Vendored/synced inputs from `x07-web-ui`:

- host assets: `vendor/x07-web-ui/host/*`
- examples + traces: `vendor/x07-web-ui/examples/*`
- `std-web-ui` modules: `vendor/x07-web-ui/packages/std-web-ui/0.1.1/modules/*`
- WIT contract: `wit/x07/web_ui/0.2.0/*`

Update/check:

```sh
python3 scripts/vendor_x07_web_ui.py update --src ../x07-web-ui
python3 scripts/vendor_x07_web_ui.py check
python3 scripts/vendor_x07_web_ui.py check --src ../x07-web-ui
```

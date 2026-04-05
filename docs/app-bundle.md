# Full-stack app bundles

This guide ties web-ui reducers and `wasi:http/proxy` backend components into a single **full-stack app bundle** plus an end-to-end loop:

> **app profile → app build → app serve → app test → incident → regression**

## Contracts

- App profile: `x07.app.profile@0.2.0`
- App bundle manifest: `x07.app.bundle@0.1.0`
- App trace (E2E): `x07.app.trace@0.1.0`
- HTTP envelopes:
  - request: `x07.http.request.envelope@0.1.0`
  - response: `x07.http.response.envelope@0.1.0`

## Registries (contracts-as-data)

App profiles:

- Index: `arch/app/index.x07app.json`
- Profiles: `arch/app/profiles/*.json`

Cross-checked registries:

- Web UI profiles: `arch/web_ui/index.x07webui.json`
- Backend component profiles: `arch/wasm/component/index.x07wasm.component.json`

Validate (offline, tool self-validation):

```sh
x07-wasm app contracts validate --json
x07-wasm app profile validate --json
```

## Build

```sh
x07-wasm app build --profile app_dev --out-dir dist/app --clean --json
```

Outputs (minimum):

- `dist/app/app.bundle.json`
- `dist/app/frontend/*` (host assets + `app.wasm` + `app.manifest.json`)
- `dist/app/backend/*` (runnable `wasi:http/proxy` backend component)

## Serve

Integrated devserver serving:

- static frontend (`/`)
- API routing (`/api/*`) into the in-process backend component host

```sh
x07-wasm app serve --dir dist/app --mode listen --strict-mime --json
```

CI smoke (starts server on port `0`, checks `.wasm` MIME, and runs one `/api` canary request):

```sh
x07-wasm app serve --dir dist/app --mode smoke --strict-mime --json
```

## Test (E2E trace replay)

Replays an `x07.app.trace@...` without a browser:

- drives the web-ui reducer under Wasmtime
- executes any emitted HTTP request effects by calling the backend component host directly

```sh
x07-wasm app test --dir dist/app --trace examples/app_fullstack_hello/tests/trace_0001.json --json
```

On mismatch, an incident bundle is written under:

- `.x07-wasm/incidents/app/<YYYY-MM-DD>/<id>/`

## Incident → regression

Convert an app test incident bundle into a deterministic regression fixture set:

```sh
x07-wasm app regress from-incident .x07-wasm/incidents/app/<YYYY-MM-DD>/<id> --out-dir tests/regress --name incident --json
```

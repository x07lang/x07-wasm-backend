# x07-wasm-backend

`x07-wasm-backend` is the WebAssembly production line for [X07](https://github.com/x07lang/x07). It contains the `x07-wasm` build pipeline, host runners, packaging commands, schemas, examples, and release tooling for the full WASM story: small solve-pure modules, server-side components, browser UI apps, and packaged desktop/mobile apps.

The vision is that WASM should not be a side path in x07. It should be a first-class way to ship fast, portable, inspectable programs that coding agents can build and verify reliably.

x07-wasm-backend is designed for **100% agentic coding**. An AI coding agent can build, test, package, deploy, and regress-check WASM artifacts using machine-readable reports and stable contracts instead of custom per-project glue.

## How it fits into the x07 ecosystem

`x07-wasm-backend` connects the core language to the rest of the application stack:

- **`x07`** is the language, compiler front door, repair loop, and docs entrypoint.
- **`x07-wasm-backend`** turns x07 projects into WASM modules, components, browser apps, and device bundles.
- **`x07-web-ui`** provides the canonical reducer-side UI contracts used by browser and device apps.
- **`x07-device-host`** runs packaged device bundles on desktop and mobile.
- **`x07-platform`** consumes the resulting app packs, release metadata, and incident/regression artifacts.

If the core repo tells you how to write x07, this repo is what turns that code into runnable WASM products.

## Prerequisites

The [X07 toolchain](https://github.com/x07lang/x07) must be installed before using x07-wasm-backend. If you (or your agent) are new to X07, start with the **[Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart)** — it covers toolchain setup, project structure, and the workflow conventions an agent needs to be productive.

Rust is pinned via `rust-toolchain.toml` for deterministic outputs (including embedded WASM adapter snapshots). Run `cargo`/CI gate scripts from this repo root so `rustup` applies the pin.

## Practical usage

Use `x07-wasm-backend` when you want to:

- compile an x07 program to a portable WASM module
- run WASM code with deterministic budgets and machine-readable reports
- build a browser UI app from an x07 reducer
- package the same reducer for desktop, iOS, and Android
- inspect workload-oriented packaging and topology previews for service-style backends
- produce app packs, incident bundles, regression inputs, SLO checks, and provenance data for release automation

## Install

```sh
x07up component add wasm
x07 wasm doctor --json
```

Fallbacks:

```sh
cargo install --locked x07-wasm --version 0.2.10
```

Use `cargo install --locked --git https://github.com/x07lang/x07-wasm-backend.git x07-wasm` only when you need unreleased development state from this repo.

## Quickstart

```sh
x07-wasm doctor --json
x07-wasm profile validate --json

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

That is the simplest standalone flow: build one WASM artifact and run it locally.

As part of the wider x07 ecosystem, the usual progression is:

1. start in `x07` with a normal project
2. use `x07-wasm` to build the target you need
3. use `x07-web-ui` for browser/device UI reducers when applicable
4. use `x07-device-host` for native shells
5. use `x07-platform` when the artifact needs staged release, incident capture, and regression feedback

## Consumer web-ui/device apps

For a consumer repo that ships one reducer across browser and device targets, keep these surfaces together:

- `frontend/` — the X07 reducer project
- `arch/wasm/` — wasm build profiles
- `arch/web_ui/` — web-ui profiles
- `arch/device/` — device profiles and index

The expected validation loop is:

```sh
x07-wasm web-ui build --project frontend/x07.json --profile web_ui_debug --out-dir dist/web_ui_debug --clean --json
x07-wasm web-ui test --dist-dir dist/web_ui_debug --case tests/your_case.trace.json --json
x07-wasm device build --index arch/device/index.x07device.json --profile device_ios_dev --out-dir dist/device_ios_dev_bundle --clean --strict --json
x07-wasm device verify --dir dist/device_ios_dev_bundle --json
x07-wasm device package --bundle dist/device_ios_dev_bundle --target ios --out-dir dist/device_ios_dev_package --json
```

Start from [`examples/x07_builder_io_min`](examples/x07_builder_io_min/README.md) for the current import/edit/export/share reducer path, [`examples/x07_capture_min`](examples/x07_capture_min/README.md) for the camera/location/notification-native surface, and [`examples/x07_hex_min`](examples/x07_hex_min/README.md) for the Tactics M0 select/move/audio/haptics line. Pair those with the reducer patterns in [`x07-web-ui/examples/web_ui_form`](../x07-web-ui/examples/web_ui_form). Together they are the current reference path for a consumer-owned web-ui/device app.

Generated Android projects now include a pinned Gradle wrapper (`./gradlew`). Build them with a supported JDK (17 or 21); on a machine with Android Studio installed, the bundled JBR is a valid `JAVA_HOME`:

```sh
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./gradlew assembleDebug
```

Device profiles now keep runtime capabilities and telemetry transport settings in standalone sidecar contracts. A validated bundle embeds:

- `app.manifest.json`
- `profile/device.profile.json`
- `profile/device.capabilities.json`
- `profile/device.telemetry.profile.json`

The device-capability sidecar now carries the Forge/Tactics M0 fields consumed by `std-web-ui@0.2.6` and `x07-device-host@0.2.5`: `audio.playback`, `haptics.present`, `clipboard.read_text`, `clipboard.write_text`, `files.pick_multiple`, `files.save`, `files.drop`, and `share.present`.

`x07-wasm device build` writes `app.manifest.json` into the bundle root so device hosts can reuse the same `apiPrefix`, component entrypoint, and `webUi` runtime limits that the browser host reads from app builds. `x07-wasm app serve` also answers API `OPTIONS` preflight requests with the canonical CORS headers expected by packaged device hosts calling a local or remote HTTP backend.

The app/device replay surface now keeps storage-write acknowledgements payload-free: browser/device hosts and `x07-wasm app test` re-dispatch `state.__x07_storage.set.ok` without replaying the stored value blob into reducer state. `x07-wasm app test` also replays the builder-I/O files-save path with deterministic `files.save` acknowledgements for `x07.web_ui.effect.device.files.save_json` and `x07.web_ui.effect.device.files.save_text`, which keeps export flows traceable in CI.

The shared web-ui runner now resets Wasmtime fuel before each reducer dispatch when a profile declares `runtime.max_fuel`. Multi-step app traces therefore honor the packaged per-dispatch fuel ceiling instead of draining one cumulative store budget across an entire replay case.

The telemetry sidecar must now declare the standard device-observability event classes and OTLP transport profile used by the platform release loop:

- `app.lifecycle`
- `app.http`
- `runtime.error`
- `bridge.timing`
- `reducer.timing`
- `policy.violation`
- `host.webview_crash`

`x07-wasm device verify --json` and `x07-wasm device package --json` now emit the `@0.2.0` report line with a deterministic `result.native_summary` plus machine-readable `result.release_readiness` diagnostics.

`x07-wasm device regress from-incident` now prefers platform incident directories containing `incident.bundle.json`, optional incident meta files, and `regression.request.json`; legacy `incident.json` inputs are still accepted when that is the only available incident format.

## Official showcase apps

For richer end-to-end references, start with:

- [`examples/x07_builder_io_min`](examples/x07_builder_io_min): builder-I/O proving app with import/edit/export, clipboard, share, and cross-target device packaging
- [`examples/x07_hex_min`](examples/x07_hex_min): Tactics M0 reference with deterministic select/move/end-turn flow, cue audio, haptics, and cross-target device packaging
- [`examples/x07_atlas`](examples/x07_atlas): full-stack app bundle with offline-first UI, API traces, incident regression generation, pack verification, provenance, deploy planning, and SLO checks
- [`examples/x07_studio`](examples/x07_studio): desktop device bundle with persistent project notes, import/export flows, provenance, packaging, and desktop host smoke
- [`examples/x07_field_notes`](examples/x07_field_notes): one reducer packaged across desktop, iOS, and Android with replay traces and embedded-assets-only mobile outputs

The examples index lives in [`examples/README.md`](examples/README.md).

## Command surface

### Solve-Pure WASM Modules

- `x07-wasm build` — build solve-pure wasm modules (current releases default to native `x07 build --emit-wasm`; legacy `clang`/`wasm-ld` path is available via `--codegen-backend c_toolchain_v1`)
- `x07-wasm run` — deterministic runner for the solve-pure ABI (`x07_solve_v2` via WASM Basic C ABI sret)
- `x07-wasm doctor`, `x07-wasm profile validate`, `x07-wasm cli specrows check`

### WASI 0.2 Components

- `x07-wasm wit validate`
- `x07-wasm component profile validate` / `build` / `compose` / `targets`
- `x07-wasm serve` / `x07-wasm component run` (use `x07-wasm serve --hot-path` for low-overhead request serving)

### Web UI

- `x07-wasm web-ui contracts validate` / `profile validate`
- `x07-wasm web-ui build` / `serve` / `test` / `regress-from-incident`

### Full-Stack App Bundle

- `x07-wasm app contracts validate` / `profile validate`
- `x07-wasm app build` / `serve` / `test` / `regress from-incident`

### Service Workload Scaffolding

- `x07-wasm workload build` / `pack` / `inspect` / `contracts-validate`
- `x07-wasm topology preview`
- `x07-wasm binding resolve`

These commands are the service-oriented workload lane. They emit deterministic workload metadata, provider-neutral binding requirements, topology previews, and a deployable `x07.workload.pack@0.1.0` manifest with first-class API, event-consumer, and scheduled-job hints. The runtime pack now carries cell kind, scale class, binding probe hints, event metadata, schedule metadata, probe definitions, rollout hints, autoscaling hints, and OCI image executables for both `native-http` and `native-worker` cells.

For `embedded-kernel` cells, the runtime pack uses `execution_kind=embedded` and includes an `embedded.manifest` file digest that points at `embedded-kernel.<cell_key>.starter.json` in the pack output directory.

### Native Backend Targets

- `x07-wasm component build --emit http-native` / `--emit cli-native`

### Production Hardening

- `x07-wasm toolchain validate`
- `x07-wasm app pack` / `app verify`
- `x07-wasm http contracts validate` / `http serve` / `http test` / `http regress from-incident`

### Ops / Policy / SLO / Deploy / Provenance

- `x07-wasm ops validate` / `caps validate` / `policy validate`
- `x07-wasm slo validate` / `slo eval`
- `x07-wasm deploy plan`
- `x07-wasm provenance attest` / `provenance verify`

`x07-wasm deploy plan` can emit Kubernetes YAML with stable telemetry identity:

- Resource labels: `lp.environment_id`, `lp.deployment_id`, `lp.service_id`
- Container env: `LP_ENVIRONMENT_ID`, `LP_DEPLOYMENT_ID`, `LP_SERVICE_ID`, `OTEL_RESOURCE_ATTRIBUTES`

Use `--environment-id`, `--deployment-id`, and `--service-id` to set those values (defaults: `environment_id=default`, `service_id=profile_id`, `deployment_id=<service_id>.<k8s_name>`).

Supported D-OSS command surface:

- `x07-wasm app pack`
- `x07-wasm app verify`
- `x07-wasm deploy plan`
- `x07-wasm slo eval`
- `x07-wasm app regress from-incident`

### Native x07 -> Wasm Backend

- Wasm profiles add `codegen_backend` (default: `native_x07_wasm_v1`)
- `--codegen-backend native_x07_wasm_v1` (native) or `--codegen-backend c_toolchain_v1` (legacy)

### Device Apps

- `x07-wasm device index validate` / `device profile validate`
- `x07-wasm device build` / `device verify`
- `x07-wasm device run` / `device package`
- `x07-wasm device regress from-incident`
- `x07-wasm device package --target ios` / `--target android`

Native incident replay gate: `bash scripts/ci/check_phase10_native_regressions.sh`

Repo-local MCP inspect smoke: `bash scripts/ci/check_phase10_mcp_inspect.sh` (skips cleanly when `../x07-mcp` is not present and inspects repo-local `x07lang-mcp` through `x07-mcp inspect --command`)

The packaged mobile templates are vendored from `x07-device-host/mobile/*` and refreshed through `scripts/vendor_x07_device_host_abi.py`; this repo no longer maintains a second editable copy of those templates.

## Contracts-as-data

- WASM profile registry: `arch/wasm/index.x07wasm.json`
- Profiles: `arch/wasm/profiles/*.json`
- WIT registry: `arch/wit/index.x07wit.json`
- Component profile registry: `arch/wasm/component/index.x07wasm.component.json`
- Web UI profile registry: `arch/web_ui/index.x07webui.json`
- App profile registry: `arch/app/index.x07app.json`
- Device profile registry: `arch/device/index.x07device.json`
- Workload profile registry: `arch/workload/index.x07workload.json`
- Schemas (published to `https://x07.io/spec/`): `crates/x07-wasm/spec/schemas/*.schema.json`

## CI gates

| Area | Gate |
|------|------|
| Solve-pure wasm | `scripts/ci/check_phase0.sh` |
| Components | `scripts/ci/check_phase1.sh` |
| Web UI | `scripts/ci/check_phase2.sh` |
| Full-stack app | `scripts/ci/check_phase3.sh` |
| Native backend targets | `scripts/ci/check_phase4.sh` |
| Hardening and HTTP | `scripts/ci/check_phase5.sh` |
| Ops, policy, and provenance | `scripts/ci/check_phase6.sh` |
| Native x07 wasm codegen | `scripts/ci/check_phase7.sh` |
| Device build and verification | `scripts/ci/check_phase8.sh` |
| Desktop host integration | `scripts/ci/check_phase9.sh` |
| Mobile packaging, builder I/O, MCP inspect smoke, and native regressions | `scripts/ci/check_phase10.sh` |

Release-ready also enforces `scripts/ci/check_schema_index.sh` so newly added public schemas stay indexed.

Example freestanding smoke: `examples/solve_pure_echo/ci/freestanding_smoke.sh`

## Avoiding CI reruns (pre-push checklist)

The CI workflow runs `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` on every push. Run these locally before pushing (especially to `main`) to avoid “fix-and-push-again” loops:

```sh
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

If you touch `crates/x07-wasm/src/device/*`, run `cargo fmt --all` even when the change looks trivial. The device incident regression generator regularly trips the rustfmt gate because long schema-validation and file-write calls wrap differently than they read in review.

Then run the gate(s) that match what you changed:
- Solve-pure wasm and component toolchain changes: `check_phase0.sh`, `check_phase1.sh`
- Web UI and app pipeline changes: `check_phase2.sh`, `check_phase3.sh`
- Native backend, hardening, ops, and provenance changes: `check_phase4.sh` through `check_phase7.sh`
- Device pipeline, templates, and host ABI changes: `check_phase8.sh` through `check_phase10.sh`

If CI fails in one of those gates, run the corresponding `scripts/ci/check_phase*.sh` locally; they are the same entry points CI uses.

Some gates, notably the component and web-ui checks, also validate that embedded adapter snapshots under `crates/x07-wasm/src/support/adapters/` match what `guest/*` builds produce. If you change `guest/*` or bump `rust-toolchain.toml`, refresh the snapshots:

```sh
bash scripts/update_adapter_snapshots.sh
```

Notes:
- Adapter WASM bytes are not stable across build environments. To keep CI deterministic, the snapshot drift check builds adapters inside a pinned `rust:<channel>` container and runs only on Linux (Ubuntu CI, Docker required).
- `scripts/update_adapter_snapshots.sh` requires Docker; it builds the guest adapters in a linux/amd64 container using the pinned `rust-toolchain.toml` channel, then copies the outputs into `crates/x07-wasm/src/support/adapters/*.component.wasm`.

## Incidents

On `x07-wasm run` failures, a deterministic incident bundle is written under `.x07-wasm/incidents/<YYYY-MM-DD>/<run_id>/` containing `input.bin`, `run.report.json`, `wasm.manifest.json`, and `stderr.txt`.

## Links

- [X07 Agent Quickstart](https://x07lang.org/docs/getting-started/agent-quickstart) — start here
- [X07 toolchain](https://github.com/x07lang/x07)
- [X07 website](https://x07lang.org)

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE).

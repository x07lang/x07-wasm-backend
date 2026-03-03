# x07-wasm-backend

Phase 0–10 WASM tooling for X07:

- `x07-wasm build`: build solve-pure wasm modules (Phase 7 defaults to native `x07 build --emit-wasm`; legacy `clang`/`wasm-ld` path is still available)
- `x07-wasm run`: deterministic runner for the Phase 0 ABI (`x07_solve_v2` via WASM Basic C ABI sret)
- `x07-wasm doctor`, `x07-wasm profile validate`, `x07-wasm cli specrows check`
- Phase 1 (WASI 0.2 components):
  - `x07-wasm wit validate`
  - `x07-wasm component profile validate`
  - `x07-wasm component build`
  - `x07-wasm component compose`
  - `x07-wasm component targets`
  - `x07-wasm serve`
  - `x07-wasm component run`
- Phase 2 (web-ui):
  - `x07-wasm web-ui contracts validate`
  - `x07-wasm web-ui profile validate`
  - `x07-wasm web-ui build`
  - `x07-wasm web-ui serve`
  - `x07-wasm web-ui test`
  - `x07-wasm web-ui regress-from-incident`
- Phase 3 (app bundle):
  - `x07-wasm app contracts validate`
  - `x07-wasm app profile validate`
  - `x07-wasm app build`
  - `x07-wasm app serve`
  - `x07-wasm app test`
  - `x07-wasm app regress from-incident`
- Phase 4 (native backend targets):
  - `x07-wasm component build --emit http-native`
  - `x07-wasm component build --emit cli-native`
- Phase 5 (hardening):
  - `x07-wasm toolchain validate`
  - `x07-wasm app pack`
  - `x07-wasm app verify`
  - `x07-wasm http contracts validate`
  - `x07-wasm http serve`
  - `x07-wasm http test`
  - `x07-wasm http regress from-incident`
- Phase 6 (ops / policy / SLO / deploy / provenance):
  - `x07-wasm ops validate`
  - `x07-wasm caps validate`
  - `x07-wasm policy validate`
  - `x07-wasm slo validate`
  - `x07-wasm slo eval`
  - `x07-wasm deploy plan`
  - `x07-wasm provenance attest`
  - `x07-wasm provenance verify`
- Phase 7 (native x07 → wasm backend):
  - wasm profiles add `codegen_backend` (default: `native_x07_wasm_v1`)
  - `x07-wasm build --codegen-backend native_x07_wasm_v1` to force native backend
  - `x07-wasm build --codegen-backend c_toolchain_v1` to force legacy C toolchain backend
- Phase 8 (device bundles):
  - `x07-wasm device index validate`
  - `x07-wasm device profile validate`
  - `x07-wasm device build`
  - `x07-wasm device verify`
- Phase 9 (device run/package):
  - `x07-wasm device run`
  - `x07-wasm device package`
- Phase 10 (mobile project generation):
  - `x07-wasm device package --target ios`
  - `x07-wasm device package --target android`

## Install (local)

```sh
cargo install --locked --path crates/x07-wasm
```

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

## Phase 0 docs

- `docs/phase0.md`

## Phase 1 docs

- `docs/phase1.md`

## Phase 2 docs

- `docs/phase2.md`

## Phase 3 docs

- `docs/phase3.md`

## Phase 4 docs

- `docs/phase4.md`

## Phase 5 docs

- `docs/phase5.md`

## Phase 6 docs

- `docs/phase6.md`

## Phase 7 docs

- `docs/phase7.md`

## Phase 8 docs

- `docs/phase8.md`

## Phase 9 docs

- `docs/phase9.md`

## Phase 10 docs

- `docs/phase10.md`

## Contracts-as-data

- WASM profile registry: `arch/wasm/index.x07wasm.json`
- Profiles: `arch/wasm/profiles/*.json`
- WIT registry: `arch/wit/index.x07wit.json`
- Component profile registry: `arch/wasm/component/index.x07wasm.component.json`
- Web UI profile registry: `arch/web_ui/index.x07webui.json`
- App profile registry: `arch/app/index.x07app.json`
- Device profile registry: `arch/device/index.x07device.json`
- Schemas (published to `https://x07.io/spec/`): `spec/schemas/*.schema.json`

## CI / smoke

- Phase 0 gate: `scripts/ci/check_phase0.sh`
- Phase 1 gate: `scripts/ci/check_phase1.sh`
- Phase 2 gate: `scripts/ci/check_phase2.sh`
- Phase 3 gate: `scripts/ci/check_phase3.sh`
- Phase 4 gate: `scripts/ci/check_phase4.sh`
- Phase 5 gate: `scripts/ci/check_phase5.sh`
- Phase 6 gate: `scripts/ci/check_phase6.sh`
- Phase 7 gate: `scripts/ci/check_phase7.sh`
- Phase 8 gate: `scripts/ci/check_phase8.sh`
- Phase 9 gate: `scripts/ci/check_phase9.sh`
- Phase 10 gate: `scripts/ci/check_phase10.sh`
- Example freestanding smoke: `examples/solve_pure_echo/ci/freestanding_smoke.sh`

## Incidents

On `x07-wasm run` failures, a deterministic incident bundle is written under:

- `.x07-wasm/incidents/<YYYY-MM-DD>/<run_id>/`

It includes:

- `input.bin`
- `run.report.json`
- `wasm.manifest.json` (copied from `<wasm>.manifest.json` when present; otherwise a synthesized Phase 0 incident manifest)
- `stderr.txt` (trap + diagnostics, if any)

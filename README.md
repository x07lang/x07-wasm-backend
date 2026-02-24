# x07-wasm-backend

Phase 0 WASM tooling for X07:

- `x07-wasm build`: `x07 build --freestanding` → C → `clang` → `wasm-ld` → `.wasm` + manifest + build report
- `x07-wasm run`: deterministic runner for the Phase 0 ABI (`x07_solve_v2` via WASM Basic C ABI sret)
- `x07-wasm doctor`, `x07-wasm profile validate`, `x07-wasm cli specrows check`

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

## Phase 0 contracts-as-data

- WASM profile registry: `arch/wasm/index.x07wasm.json`
- Profiles: `arch/wasm/profiles/*.json`
- Schemas (published to `https://x07.io/spec/`): `spec/schemas/*.schema.json`

## CI / smoke

- Phase 0 gate: `scripts/ci/check_phase0.sh`
- Example freestanding smoke: `examples/solve_pure_echo/ci/freestanding_smoke.sh`

## Incidents

On `x07-wasm run` failures, a deterministic incident bundle is written under:

- `.x07-wasm/incidents/<YYYY-MM-DD>/<run_id>/`

It includes:

- `input.bin`
- `run.report.json`
- `wasm.manifest.json` (copied from `<wasm>.manifest.json` when present; otherwise a synthesized Phase 0 incident manifest)
- `stderr.txt` (trap + diagnostics, if any)

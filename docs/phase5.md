# WASM Phase 5 (Track-1 Hardening)

Phase 5 hardens the WASM execution loop after Phase 4:

- Toolchain pin validation (`x07-wasm toolchain validate`)
- Host-enforced runtime limits (fuel, memory, table, wasm stack) with stable `X07WASM_BUDGET_EXCEEDED_*` diagnostics
- Deployable app packs (`x07-wasm app pack` + `x07-wasm app verify`)
- Web UI dist completeness (`dist/wasm.profile.json`) so replay/test does not depend on external profile registries
- (Recommended) core-wasm HTTP reducer loop (`x07-wasm http ...`) with deterministic traces

## Toolchain pin validation

Phase 5 adds a data registry under:

- `arch/wasm/toolchain/index.x07wasm.toolchain.json`

and pinned profiles under:

- `arch/wasm/toolchain/profiles/toolchain_ci.json`
- `arch/wasm/toolchain/profiles/toolchain_local.json`

Validate the pinned CI toolchain locally:

```sh
x07-wasm toolchain validate --profile arch/wasm/toolchain/profiles/toolchain_ci.json --json
```

## Runtime limits

WASM profiles now include a required `runtime` section (`x07.wasm.runtime.limits@0.1.0`) and `x07.wasm.profile` is bumped to `@0.2.0`.

Most runners support explicit overrides:

- `x07-wasm run --max-fuel ... --max-memory-bytes ... --max-table-elements ... --max-wasm-stack-bytes ...`
- `x07-wasm serve --max-fuel ... --max-memory-bytes ... --max-table-elements ...`
- `x07-wasm component run --max-fuel ... --max-memory-bytes ... --max-table-elements ...`

Budget failures surface as pinned diagnostics:

- `X07WASM_BUDGET_EXCEEDED_CPU_FUEL`
- `X07WASM_BUDGET_EXCEEDED_WASM_STACK`
- `X07WASM_BUDGET_EXCEEDED_MEMORY`
- `X07WASM_BUDGET_EXCEEDED_TABLE`

## App pack / verify

Phase 5 adds deployable pack artifacts:

- `x07-wasm app pack` produces a content-addressed `x07.app.pack@0.1.0` manifest.
- `x07-wasm app verify` recomputes digests and enforces required headers (notably `.wasm` must be served as `application/wasm`).

## Core-wasm HTTP reducers

The `x07-wasm http` command group validates contracts and provides a deterministic reducer loop:

- `x07-wasm http contracts validate --strict --json`
- `x07-wasm http serve ...`
- `x07-wasm http test ...`
- `x07-wasm http regress from-incident ...`

## CI gate

Run the Phase 5 gate locally:

```sh
export PATH="${WASI_SDK_DIR}/bin:${PATH}"
bash scripts/ci/check_phase5.sh
```


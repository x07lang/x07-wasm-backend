# Execution hardening

This guide covers hardening the WASM execution loop:

- Toolchain pin validation (`x07-wasm toolchain validate`)
- Host-enforced runtime limits (fuel, memory, table, wasm stack) with stable `X07WASM_BUDGET_EXCEEDED_*` diagnostics
- (Recommended) core-wasm HTTP reducer loop (`x07-wasm http ...`) with deterministic traces

## Toolchain pin validation

This repo includes a toolchain validation registry under:

- `arch/wasm/toolchain/index.x07wasm.toolchain.json`

and pinned profiles under:

- `arch/wasm/toolchain/profiles/toolchain_ci.json`
- `arch/wasm/toolchain/profiles/toolchain_local.json`

Validate the pinned CI toolchain locally:

```sh
x07-wasm toolchain validate --profile arch/wasm/toolchain/profiles/toolchain_ci.json --json
```

## Runtime limits

WASM profiles include a required `runtime` section (`x07.wasm.runtime.limits@0.1.0`) and `x07.wasm.profile` is bumped to `@0.3.0`.

Most runners support explicit overrides:

- `x07-wasm run --max-fuel ... --max-memory-bytes ... --max-table-elements ... --max-wasm-stack-bytes ...`
- `x07-wasm serve --max-fuel ... --max-memory-bytes ... --max-table-elements ...`
- `x07-wasm component run --max-fuel ... --max-memory-bytes ... --max-table-elements ...`

Budget failures surface as pinned diagnostics:

- `X07WASM_BUDGET_EXCEEDED_CPU_FUEL`
- `X07WASM_BUDGET_EXCEEDED_WASM_STACK`
- `X07WASM_BUDGET_EXCEEDED_MEMORY`
- `X07WASM_BUDGET_EXCEEDED_TABLE`

Optional host runtime knobs (profile-level):

- `runtime.instance_allocator`: `on_demand` (default) or `pooling`
- `runtime.cache_config`: path to a Wasmtime cache config file (passed to `Config::cache_config_load`)

Shipped WASM profiles:

- `wasm_release` (default)
- `wasm_release_cached` (enables Wasmtime compilation cache via `arch/wasm/toolchain/wasmtime_cache.toml`)
- `wasm_release_pooling` (enables Wasmtime pooling allocator)

## Core-wasm HTTP reducers

The `x07-wasm http` command group validates contracts and provides a deterministic reducer loop:

- `x07-wasm http contracts validate --strict --json`
- `x07-wasm http serve ...`
- `x07-wasm http test ...`
- `x07-wasm http regress from-incident ...`

## CI coverage

CI exercises the hardening surfaces under both the native backend and (when configured) the legacy C toolchain path (`WASI_SDK_DIR`).

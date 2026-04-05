# Native x07 → wasm backend (no C toolchain)

This guide covers the toolchain-free `solve-pure` wasm build path by teaching `x07` to emit core wasm directly and teaching `x07-wasm build` to use it.

## Profiles: `codegen_backend`

WASM profiles (`arch/wasm/profiles/*.json`) add a required field:

- `codegen_backend: "native_x07_wasm_v1" | "c_toolchain_v1"`

Default profiles use `native_x07_wasm_v1`.

## CLI override

You can override the profile’s backend selection:

```sh
x07-wasm build --project examples/solve_pure_echo/x07.json --profile wasm_release --codegen-backend native_x07_wasm_v1 --json
x07-wasm build --project examples/solve_pure_echo/x07.json --profile wasm_release --codegen-backend c_toolchain_v1 --json
```

## Build behavior

When `codegen_backend=native_x07_wasm_v1`, `x07-wasm build`:

- calls `x07 build --emit-wasm ...` (core wasm module)
- skips `clang` and `wasm-ld`
- inspects/validates the resulting wasm and emits the usual artifact + report

New build-path diagnostics:

- `X07WASM_NATIVE_BACKEND_WASM_MISSING` (expected wasm output missing)
- `X07WASM_NATIVE_BACKEND_WASM_INVALID` (inspect/validation failed)

## Toolchain validation changes

`arch/wasm/toolchain/profiles/*` add per-tool `required: true|false` so `clang`/`wasm-ld` can be optional when using the native backend.

## Component/app builds

To keep this toolchain-free by default:

- `x07-wasm component build --emit http|cli` uses the composed path (adapters + `wac plug`)
- legacy adapterless targets remain available as `--emit http-native` / `--emit cli-native` (may require the legacy C toolchain backend)

## CI coverage

CI enforces that the default backend path stays toolchain-free by shadowing `clang`/`wasm-ld` with failing stubs during the dedicated gate.

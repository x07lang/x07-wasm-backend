# WASM Phase 0

Phase 0 builds and runs **solve-pure** X07 programs as WASM modules without adding a new compiler backend.

## Build pipeline

`x07-wasm build`:

1. `x07 build --freestanding --emit-c-header …` (exports `x07_solve_v2`)
2. Compile the generated C to `wasm32` with `clang`
3. Link with `wasm-ld` (reactor-style: `--no-entry`)
4. Inspect exports + memory plan and emit:
   - `dist/*.wasm`
   - `dist/*.wasm.manifest.json` (`x07.wasm.artifact@0.1.0`)
   - build report (`x07.wasm.build.report@0.1.0`)

## Run contract (ABI)

`x07_solve_v2` returns a non-singleton C struct (`bytes_t`), so Phase 0 runners must use the **sret** calling convention under the WASM Basic C ABI:

- wasm export signature: `(retptr, arena_ptr, arena_cap, input_ptr, input_len) -> ()`
- `bytes_t` is 8 bytes: `{ ptr:u32_le, len:u32_le }`

`x07-wasm run` enforces:

- fixed memory plan expectations from the selected profile
- hard cap on returned output length (`max_output_bytes`)

## Reports

All Phase 0 commands support JSON reports with:

- stable canonical JSON ordering
- `--json-schema` / `--json-schema-id` discovery
- `--report-out` + `--quiet-json` for file-only emission

Schemas live in `spec/schemas/` and are intended to be published under `https://x07.io/spec/`.

## Incident bundles

On `x07-wasm run` failures, an incident bundle is written under `.x07-wasm/incidents/<YYYY-MM-DD>/<run_id>/`.

It always includes:

- `input.bin`
- `run.report.json`
- `wasm.manifest.json` (copied from `<wasm>.manifest.json` when present; otherwise a synthesized manifest with `schema_version: x07.wasm.incident.manifest@0.1.0`)
- `stderr.txt` (when available)

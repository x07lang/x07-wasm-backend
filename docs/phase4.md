# WASM Phase 4 (Native Backend Targets)

Phase 4 adds **native** component emit targets to `x07-wasm component build`:

- `--emit http-native`: build a runnable `wasi:http/proxy` component (`http.component.wasm`)
- `--emit cli-native`: build a runnable `wasi:cli/command` component (`cli.component.wasm`)

“Native” means these components are produced **directly from the x07 project** (x07 -> freestanding C -> core wasm -> component) without building the Rust guest adapters and without composing via `wac plug`.

## Profiles and budgets

Phase 4 bumps the component profile schema to:

- `x07.wasm.component.profile@0.2.0`

Profiles must include `cfg.native_targets` with hard budgets for:

- HTTP: request body, response body, headers, header bytes, path/query bytes, envelope bytes
- CLI: stdin, stdout, stderr

See:

- `arch/wasm/component/profiles/component_release.json`
- `arch/wasm/component/profiles/component_debug.json`

## Emit semantics

`x07-wasm component build --emit` supports:

- `solve`: Phase 1 solve component (`solve.component.wasm`)
- `http`: composed HTTP component (Phase 1 adapters + `wac plug`; does not require the C toolchain)
- `cli`: composed CLI component (Phase 1 adapters + `wac plug`; does not require the C toolchain)
- `http-native`: Phase 4 native HTTP component (legacy C toolchain path)
- `cli-native`: Phase 4 native CLI component (legacy C toolchain path)
- `http-adapter` / `cli-adapter`: legacy adapter components
- `all` (default): build `solve + http + cli` (composed)

Adapters are no longer part of the default path; build them explicitly when needed.

## HTTP envelope contract

The native HTTP glue expects the same contract as the legacy `http-adapter`:

- Input: `x07.http.request.envelope@0.1.0` JSON bytes
- Output: `x07.http.response.envelope@0.1.0` JSON bytes

The body is represented as a stream payload (`{ bytes_len, base64 }`).

## Diagnostics and incidents

Phase 4 standardizes guest->host diagnostic channels:

- HTTP: response headers
  - `x-x07-diag-code: <CODE>`
  - `x-x07-diag-data-b64: <base64(json-object)>` (optional)
  `x07-wasm serve` extracts these into `diagnostics[]` and writes an incident bundle on errors.
- CLI: stderr sentinel lines
  - `x07-diag-code: <CODE>`
  - `x07-diag-data-b64: <base64(json-object)>` (optional)
  `x07-wasm component run` extracts these into `diagnostics[]` and writes an incident bundle on errors.

Host mapping failures (duplicates, invalid encodings, orphaned data) are reported as pinned
`X07WASM_HOST_DIAG_*` diagnostics.

## CI gate

Run the Phase 4 loop locally:

```sh
# Required for the legacy C toolchain backend (`http-native` / `cli-native`).
export PATH="${WASI_SDK_DIR}/bin:${PATH}"
bash scripts/ci/check_phase4.sh
```

# WASM Phase 9 (Device host runner + run/package)

Phase 9 introduces a single desktop host runner (`x07-device-host-desktop`) and wires it into `x07-wasm`:

- `x07-wasm device run`: delegates to the host to execute `ui/reducer.wasm` in a system WebView.
- `x07-wasm device package`: packages a device bundle into a desktop payload and emits `package.manifest.json`, carrying the sealed profile/capabilities/telemetry sidecar digests from the bundle manifest.

## Toolchain

The desktop host tool must be available on PATH (or specified via `X07_DEVICE_HOST_DESKTOP`):

- `arch/device/toolchain/device_toolchain_dev.json`

## Host ABI sync gate

Phase 9 adds a deterministic ABI sync gate so host/bundle drift is caught even when `x07-device-host-desktop` is not installed:

- `scripts/ci/check_phase9_host_abi_sync.sh` enforces that:
  - `vendor/x07-device-host/host_abi.snapshot.json` matches `vendor/x07-device-host/snapshot.json`, and
  - `crates/x07-wasm/src/device/host_abi.rs` matches the vendored `host_abi_hash`.
- `scripts/ci/check_phase9_host_abi_negative.sh` tampers a known-good bundle and asserts `x07-wasm device verify` fails with `X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH` (exit code 3).

## CLI

Run a device bundle:

```sh
x07-wasm device run --bundle dist/device --target desktop --json
```

Package a device bundle:

```sh
x07-wasm device package --bundle dist/device --target desktop --out-dir dist/device_package --json
```

## CI gate

```sh
bash scripts/ci/check_phase9.sh
```

The official desktop showcase for this surface is [`examples/x07_studio`](../examples/x07_studio/README.md).

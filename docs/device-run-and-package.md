# Device runner: run and package

This guide covers the desktop host runner (`x07-device-host-desktop`) and how it wires into `x07-wasm`:

- `x07-wasm device run`: delegates to the host to execute `ui/reducer.wasm` in a system WebView.
- `x07-wasm device package`: packages a device bundle into a desktop payload and emits `package.manifest.json`, carrying the sealed profile/capabilities/telemetry sidecar digests from the bundle manifest.

## Toolchain

The desktop host tool must be available on PATH (or specified via `X07_DEVICE_HOST_DESKTOP`):

- `arch/device/toolchain/device_toolchain_dev.json`

## Host ABI sync coverage

CI enforces that the vendored host ABI snapshot stays in sync and that `crates/x07-wasm/src/device/host_abi.rs` matches the pinned `host_abi_hash`. It also tampers a known-good bundle and asserts `x07-wasm device verify` fails with `X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH` (exit code 3).

## CLI

Run a device bundle:

```sh
x07-wasm device run --bundle dist/device --target desktop --json
```

Package a device bundle:

```sh
x07-wasm device package --bundle dist/device --target desktop --out-dir dist/device_package --json
```

The official desktop showcase for this surface is [`examples/x07_studio`](../examples/x07_studio/README.md).

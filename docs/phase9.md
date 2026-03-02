# WASM Phase 9 (Device host runner + run/package)

Phase 9 introduces a single desktop host runner (`x07-device-host-desktop`) and wires it into `x07-wasm`:

- `x07-wasm device run`: delegates to the host to execute `ui/reducer.wasm` in a system WebView.
- `x07-wasm device package`: packages a device bundle into a desktop payload and emits `package.manifest.json`.

## Toolchain

The desktop host tool must be available on PATH (or specified via `X07_DEVICE_HOST_DESKTOP`):

- `arch/device/toolchain/device_toolchain_dev.json`

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


# WASM Phase 8 (Device bundles: system WebView host + pinned host ABI)

Phase 8 introduces a device contract layer for running `std.web_ui` reducers in a system WebView host (desktop + mobile).

The device bundle format pins the host ABI hash from `x07-device-host` so that a device app can reject incompatible hosts deterministically.

## Contracts-as-data

- Device profile registry: `arch/device/index.x07device.json`
- Device profiles: `arch/device/profiles/*.json`
- Bundle manifest: `bundle.manifest.json` (`x07.device.bundle.manifest@0.1.0`)

## CLI

Validate contracts:

```sh
x07-wasm device index validate --json
x07-wasm device profile validate --json
```

Build a device bundle (web-ui reducer wasm + pinned host ABI):

```sh
x07-wasm device build --profile device_dev --out-dir dist/device --clean --json
```

Verify a device bundle:

```sh
x07-wasm device verify --dir dist/device --json
```

## CI gate

```sh
bash scripts/ci/check_phase8.sh
```


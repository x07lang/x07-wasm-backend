# WASM Phase 8 (Device bundles: system WebView host + pinned host ABI)

Phase 8 introduces a device contract layer for running `std.web_ui` reducers in a system WebView host (desktop + mobile).

The device bundle format pins the host ABI hash from `x07-device-host` so that a device app can reject incompatible hosts deterministically.

## Host ABI pin

The device host ABI is defined by the `x07-device-host` repo and includes the host bridge protocol version and the embedded host assets (including CSP).

This repo vendors the device host ABI snapshot:

- Source of truth: `../x07-device-host/arch/host_abi/host_abi.snapshot.json`
- Vendored copy: `vendor/x07-device-host/host_abi.snapshot.json`

The mobile project templates are vendored from `x07-device-host` together with the ABI snapshot:

- Source of truth: `../x07-device-host/mobile/{ios,android}/template`
- Vendored copy: `vendor/x07-device-host/mobile/{ios,android}/template`

Update/check vendoring:

```sh
python3 scripts/vendor_x07_device_host_abi.py update --src ../x07-device-host
python3 scripts/vendor_x07_device_host_abi.py check
```

Consumer repos do not need to vendor either the device host ABI snapshot or the mobile templates. `x07-wasm` embeds the pinned host ABI hash and packages the vendored `x07-device-host` templates into generated iOS/Android projects at build time.

`x07-wasm device verify` enforces that `bundle.manifest.json` `host.host_abi_hash` matches the embedded pinned host ABI constant in `crates/x07-wasm/src/device/host_abi.rs` and emits `X07WASM_DEVICE_BUNDLE_HOST_ABI_HASH_MISMATCH` (exit code 3) on mismatch.

## Contracts-as-data

- Device profile registry: `arch/device/index.x07device.json`
- Device profiles: `arch/device/profiles/*.json`
- Device capabilities: `arch/device/profiles/*.capabilities.json`
- Device telemetry profiles: `arch/device/profiles/*.telemetry.profile.json`
- Bundle manifest: `bundle.manifest.json` (`x07.device.bundle.manifest@0.1.0`)

Telemetry profiles must advertise the standard event-class set used by the platform device-release loop and may target either `http/json` or `http/protobuf` OTLP transport. M0 device bundles also carry `x07.device.capabilities@0.2.0`, which separates build-time capability allowlisting from runtime permission outcomes for camera, audio, haptics, files, clipboard, blob storage, foreground location, local notifications, and share support. The `files` capability line now also carries the Forge builder-I/O switches for `pick_multiple`, `save`, and `drop`, while `audio`, `haptics`, `clipboard`, and `share` live under their own device subobjects.

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

`x07-wasm web-ui build` and `x07-wasm device build` emit the canonical host assets from the embedded vendored snapshot; end-user projects should not copy `vendor/x07-web-ui/*` into their own repos.

Bundle layout notes:

- The resolved device profile is embedded into the bundle under `profile/device.profile.json`.
- The resolved capabilities and telemetry sidecars are embedded under `profile/device.capabilities.json` and `profile/device.telemetry.profile.json`.
- `x07-wasm device verify` digest-verifies all three profile-sidecar files from `bundle.manifest.json`.
- `x07-wasm device package` projects the enabled M0 capabilities into generated iOS and Android templates so packaged apps include the required usage-description and runtime-permission declarations.

Verify a device bundle:

```sh
x07-wasm device verify --dir dist/device --json
```

Notes:

- `x07-wasm device verify` streams digests and enforces hard size caps to avoid unbounded reads:
  - bundle manifest: 8 MiB (`X07WASM_DEVICE_BUNDLE_MANIFEST_TOO_LARGE`)
  - bundle files: 256 MiB (`X07WASM_DEVICE_BUNDLE_FILE_TOO_LARGE` with `role=ui_wasm|profile|capabilities|telemetry_profile`)

Create and verify a signed provenance attestation for a device bundle:

```sh
x07-wasm device provenance attest --dir dist/device --signing-key arch/provenance/dev.ed25519.signing_key.b64 --out dist/device.provenance.dsse.json --json
x07-wasm device provenance verify --attestation dist/device.provenance.dsse.json --bundle-dir dist/device --trusted-public-key arch/provenance/dev.ed25519.public_key.b64 --json
```

## CI gate

```sh
bash scripts/ci/check_phase8.sh
```
